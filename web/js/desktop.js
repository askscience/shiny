/**
 * desktop.js — Hyprland-style desktop manager.
 *
 * Owns workspaces ("desktops"), the master/stack tiling layout, per-window
 * focus and fullscreen state. `tiles.js` owns plugin-window DOM (mounting the
 * map/radio/word/youtube/calc tiles and artifact sheets) and asks this module
 * how to lay the windows out.
 *
 * State is scoped per traveler (see preferences.js) and lives in localStorage.
 * Workspaces, focus, fullscreen and layout are all remembered across reloads.
 *
 * Public control surface (used by the workspace bar, the Keyboard plugin's
 * control row, physical keybindings, and the AI's desktop tools):
 *   focusWindow / cycleFocus / toggleFullscreen
 *   createWorkspace / removeWorkspace / switchWorkspace / moveWindow
 *   applyLayout / renderWorkspaceBar
 */
import {
  getWorkspaces, setWorkspaces,
  getActiveWorkspaceId, setActiveWorkspaceId,
  getDesktopLayout, setDesktopLayout,
  getWindowsGeom, setWindowsGeom,
} from './preferences.js';
import { toast, icon } from '../ui/index.js';
import { pluginIconEl } from './pluginIcon.js';
import { isMobilePortrait, MOBILE_PORTRAIT_QUERY } from './viewport.js';

let workspaces = [];      // [{ id, windows: [pluginName], focus, fullscreen }]
let activeWs = null;      // active workspace id
let focus = null;         // focused plugin name (cache of active workspace's)
let fullscreen = null;    // fullscreen plugin name (cache of active workspace's)

let wsSeq = 0;

// Floating-window geometry for the "Windows" layout mode. Scoped per traveler
// via preferences.js and only consulted when the layout mode is 'windows'.
let windowGeom = {};  // pluginName -> { x, y, w, h, z }
let zSeq = 0;
let geomTimer = null;

/**
 * Workspaces are a wide-screen idea. On a vertical, phone-like screen the
 * desktop is one column of windows, so the switcher, its shortcuts, the menus
 * and the AI's workspace tools all stand down. This is a view over the stored
 * arrangement, not a rewrite — nothing is persisted differently, so the
 * workspaces are exactly as they were when a wide screen comes back.
 */
export function workspacesEnabled() {
  return !isMobilePortrait();
}

/** Every window the stored workspaces hold, in order and de-duplicated. */
function everyWindow() {
  return [...new Set(workspaces.flatMap((ws) => ws.windows))];
}

function freshId() {
  wsSeq += 1;
  return `ws-${Date.now().toString(36)}-${wsSeq}`;
}

function activeWsObj() {
  return workspaces.find((w) => w.id === activeWs) || workspaces[0] || null;
}

/** Write the cached focus/fullscreen into the active workspace object. */
function syncActiveFocus() {
  const ws = activeWsObj();
  if (ws) {
    ws.focus = focus || null;
    ws.fullscreen = fullscreen || null;
  }
}

/** Read the active workspace's focus/fullscreen into the cache. */
function loadActiveFocus() {
  const ws = activeWsObj();
  focus = ws?.focus || null;
  fullscreen = ws?.fullscreen || null;
}

function persist() {
  setWorkspaces(workspaces);
  setActiveWorkspaceId(activeWs);
}

/** Notify tiles.js to re-render after any state change. `detail.slide` is an
 *  optional direction (+1/-1) hint so a workspace switch can animate which
 *  way the incoming windows slide. */
function notify(detail) {
  window.dispatchEvent(new CustomEvent('desktop:changed', { detail }));
}

/** True when a `desktop:changed` detail means "only the focus moved". */
export function isFocusOnlyChange(detail) {
  return detail === 'focus' || detail?.focusOnly === true;
}

export function initDesktop() {
  wasMobilePortrait = isMobilePortrait();
  workspaces = getWorkspaces() || [];
  activeWs = getActiveWorkspaceId();
  if (!workspaces.length || !workspaces.some((w) => w.id === activeWs)) {
    activeWs = workspaces.length ? workspaces[0].id : null;
  }
  windowGeom = getWindowsGeom();
  zSeq = Object.values(windowGeom).reduce((m, g) => Math.max(m, Number(g?.z) || 0), 0);
  loadActiveFocus();
  wireShortcuts();
  wireAgentActions();
  // Turning the screen switches the workspace system off or on. The media
  // query is the precise signal; `resize` is the fallback, because a few
  // browsers (and embedded webviews) only deliver that one. Both funnel into
  // the same transition check so a resize that changes nothing is free.
  MOBILE_PORTRAIT_QUERY.addEventListener('change', onScreenTurn);
  window.addEventListener('resize', onScreenTurn);
}

let wasMobilePortrait = null;

function onScreenTurn() {
  const now = isMobilePortrait();
  if (now === wasMobilePortrait) return;
  const first = wasMobilePortrait === null;
  wasMobilePortrait = now;
  if (!first) notify();
}

/** The AI's desktop tools arrive as `agent:actions` entries — apply them here
 *  so the user and the AI drive the exact same desktop state. */
let agentActionsWired = false;
function wireAgentActions() {
  if (agentActionsWired) return;
  agentActionsWired = true;
  window.addEventListener('agent:actions', (e) => {
    const actions = e.detail || [];
    for (const a of actions) {
      if (a?.result !== 'ok' || !a.data) continue;
      // The AI's workspace tools are inert on a vertical screen; the server is
      // told the same thing in getDesktopSnapshot() so it does not offer them.
      if (String(a.action || '').startsWith('workspace_') && !workspacesEnabled()) continue;
      const d = a.data;
      switch (a.action) {
        case 'desktop_fullscreen':
          if (d.plugin) toggleFullscreen(d.plugin, d.fullscreen !== false);
          break;
        case 'desktop_focus':
          if (d.plugin) focusWindow(d.plugin);
          break;
        case 'workspace_create':
          createWorkspace();
          break;
        case 'workspace_remove':
          removeWorkspace();
          break;
        case 'workspace_switch': {
          const to = d.workspace;
          if (to === 'next' || to === 'prev') switchWorkspace(to);
          else switchWorkspace(Number(to));
          break;
        }
        case 'workspace_move':
          if (d.plugin) moveWindowByIndex(d.plugin, d.workspace);
          break;
        default:
          break;
      }
    }
  });
}

/* ── Workspace bookkeeping ──────────────────────────────────── */

/**
 * Keep fullscreen a strictly per-window state, repairing anything that breaks
 * the rule — whether it was saved by an older build, left behind by a failed
 * move, or written by a tool:
 *
 *   • a fullscreen app whose workspace no longer holds it is forgotten;
 *   • a workspace that holds a fullscreen app holds NOTHING ELSE: every other
 *     window is moved out (to the workspace the fullscreen app came from, or
 *     to any ordinary workspace) and stays non-fullscreen.
 *
 * Idempotent, so it is safe to call from every mutation and every render.
 */
function reconcile() {
  if (!workspacesEnabled() || !workspaces.length) return false;
  let changed = false;

  const locked = new Map();       // workspace id -> fullscreen plugin
  for (const ws of workspaces) {
    if (ws.fullscreen && !ws.windows.includes(ws.fullscreen)) {
      ws.fullscreen = null;
      changed = true;
    }
    if (ws.fullscreen) locked.set(ws.id, ws.fullscreen);
  }
  if (!locked.size) {
    if (changed) loadActiveFocus();
    return changed;
  }

  // Strays can only go somewhere that is not itself sealed.
  const shelter = activeWsObj();
  const ordinary = workspaces.find((w) => !locked.has(w.id)) || null;
  for (const ws of workspaces) {
    if (!locked.has(ws.id) || ws.windows.length <= 1) continue;
    const fs = locked.get(ws.id);
    for (const w of [...ws.windows]) {
      if (w === fs) continue;
      ws.windows = ws.windows.filter((x) => x !== w);
      if (ws.focus === w) ws.focus = null;
      const to = (shelter && shelter.id !== ws.id && !locked.has(shelter.id)) ? shelter : ordinary;
      if (to) {
        if (!to.windows.includes(w)) to.windows.push(w);
      } else {
        pushWorkspace().windows = [w];
      }
      changed = true;
    }
  }
  if (changed) loadActiveFocus();
  return changed;
}

/**
 * Guarantee every plugin surface lives in a workspace: migrate a first load
 * (one workspace with everything), prune deactivated surfaces, repair the
 * fullscreen rule (see `reconcile`), and add newly activated ones to the active
 * workspace — never into a dedicated fullscreen one.
 */
export function ensureWindows(names) {
  if (!workspaces.length) {
    if (names.length) {
      workspaces = [{ id: freshId(), windows: [...names] }];
      activeWs = workspaces[0].id;
      persist();
    }
    return;
  }
  // On the very first render the active-plugin set isn't loaded yet (names is
  // empty). Do NOT prune/add then — filtering against an empty list wipes every
  // saved workspace and re-adds all windows to the active one after a refresh.
  if (!names.length) return;
  for (const ws of workspaces) {
    ws.windows = ws.windows.filter((w) => names.includes(w));
    if (ws.focus && !names.includes(ws.focus)) ws.focus = null;
    if (ws.fullscreen && !names.includes(ws.fullscreen)) ws.fullscreen = null;
  }
  // A stale/split fullscreen state is repaired first, so a window that an older
  // build dropped into a fullscreen workspace is moved out before placement.
  reconcile();
  loadActiveFocus();
  const assigned = new Set(workspaces.flatMap((ws) => ws.windows));
  const anew = [];
  for (const n of names) {
    if (!assigned.has(n)) {
      anew.push(n);
      assigned.add(n);
    }
  }
  if (anew.length) {
    let aws = activeWsObj();
    // A dedicated fullscreen workspace takes nothing else, so a plugin opened
    // while it is up lands in the workspace the fullscreen app came from (or
    // any ordinary one) instead of behind the fullscreen window.
    if (workspaceLock(aws)) aws = workspaces.find((w) => !workspaceLock(w)) || null;
    if (!aws) {
      // Every workspace is sealed: open an ordinary one for the new windows.
      aws = workspaces.find((w) => !workspaceLock(w)) || pushWorkspace();
    }
    aws.windows.push(...anew);
  }
  if (!workspaces.some((w) => w.id === activeWs)) activeWs = workspaces[0].id;
  persist();
}

/** Windows of the active workspace that still exist, in workspace order. */
export function activeWindowNames(allNames) {
  if (!workspacesEnabled()) return [...allNames];
  const ws = activeWsObj();
  if (!ws) return [...allNames];
  return ws.windows.filter((w) => allNames.includes(w));
}

export function workspaceCount() { return workspacesEnabled() ? workspaces.length : 1; }

export function activeWorkspaceIndex() {
  if (!workspacesEnabled()) return 0;
  return Math.max(0, workspaces.findIndex((w) => w.id === activeWs));
}

export function getWorkspacesList() {
  return workspacesEnabled() ? workspaces : [{ id: 'single', windows: everyWindow() }];
}

export function workspaceHasWindow(name) {
  return workspaces.some((w) => w.windows.includes(name));
}

/**
 * A dedicated fullscreen workspace is sealed: the fullscreen app cannot leave
 * it and nothing else may be moved in, so no window can ever end up tiled
 * behind a fullscreen one. Returns the plugin that owns the workspace, or null.
 */
export function workspaceLock(ws) {
  if (!ws || !workspacesEnabled()) return null;
  // A workspace holding a fullscreen window is sealed even when it is no longer
  // the active one, so a stale switch cannot drop another window into it.
  if (ws.fullscreen && ws.windows.includes(ws.fullscreen)) return ws.fullscreen;
  if (fullscreen && ws.id === activeWs && ws.windows.includes(fullscreen)) return fullscreen;
  return null;
}

/** How a workspace is named: a dedicated fullscreen space is just the app. */
export function workspaceLabel(wsOrIndex) {
  const byIndex = typeof wsOrIndex === 'number';
  const ws = byIndex ? workspaces[wsOrIndex] : wsOrIndex;
  const i = byIndex ? wsOrIndex : workspaces.indexOf(ws);
  if (!ws) return null;
  // While an app is fullscreen, its workspace IS that app: "Youtube", not
  // "Workspace 3 — Youtube".
  const locked = workspaceLock(ws);
  if (locked) return label(locked);
  return ws.name ? `Workspace ${i + 1} — ${ws.name}` : `Workspace ${i + 1}`;
}

/** Snapshot of the desktop sent to the AI on every request (1-based indices),
 *  so it knows which windows live in which workspace before reorganizing. */
export function getDesktopSnapshot() {
  if (!workspacesEnabled()) {
    return {
      active: 1,
      workspaces: [{ index: 1, windows: everyWindow() }],
      workspaces_enabled: false,
    };
  }
  return {
    active: workspaces.length ? activeWorkspaceIndex() + 1 : 1,
    workspaces: workspaces.map((ws, i) => ({
      index: i + 1,
      windows: [...ws.windows],
    })),
    workspaces_enabled: true,
  };
}

/* ── Focus / fullscreen ─────────────────────────────────────── */

export function getFocus() { return focus; }
export function getFullscreen() { return fullscreen; }

/** Focus a window and switch to whichever workspace holds it. */
export function focusWindow(name) {
  if (!name) return;

  // Re-focusing the window that already has focus is a no-op. Without this the
  // expensive path below ran again on every click of the same window.
  if (focus === name && workspaces.some((w) => w.id === activeWs && w.windows.includes(name))) {
    return;
  }

  const ws = workspaces.find((w) => w.windows.includes(name));
  if (ws && ws.id !== activeWs) {
    // A workspace switch does change what is on screen, so that keeps the full
    // re-render.
    syncActiveFocus();
    activeWs = ws.id;
    loadActiveFocus();
    focus = name;
    syncActiveFocus();
    if (getDesktopLayout().mode === 'windows') bumpZ(name);
    persist();
    notify();
    return;
  }

  focus = name;
  syncActiveFocus();
  if (getDesktopLayout().mode === 'windows') bumpZ(name);
  persist();
  // Focus-only: repaint the focus classes instead of re-tiling every window.
  repaintFocus();
  notify('focus');
}

export function cycleFocus(names, dir = 1) {
  const list = activeWindowNames(names);
  if (!list.length) { focus = null; syncActiveFocus(); persist(); notify(); return; }
  if (!focus || !list.includes(focus)) {
    focus = list[0];
  } else {
    const i = list.indexOf(focus);
    focus = list[(i + dir + list.length) % list.length];
  }
  syncActiveFocus();
  if (getDesktopLayout().mode === 'windows') bumpZ(focus);
  persist();
  notify();
}

export function clearFocus() {
  focus = null;
  syncActiveFocus();
  persist();
  repaintFocus();
  notify('focus');
}

/** Toggle (or force) fullscreen for a window. Defaults to the focused one. */
export function toggleFullscreen(name, force) {
  if (name == null) name = focus;
  if (!name) return false;
  const next = force === undefined ? fullscreen !== name : !!force;
  fullscreen = next ? name : null;
  if (next) focus = name;
  syncActiveFocus();
  persist();
  notify();
  return next;
}

export function clearFullscreen() {
  fullscreen = null;
  syncActiveFocus();
  persist();
  notify();
}

/**
 * Move a window into a brand-new workspace and focus it there — the desktop
 * half of a real-fullscreen window (fullscreen.js does the browser half).
 * Returns a token for `restoreFromNewWorkspace`, or null when the window is
 * not on a workspace (vertical phone screens have no workspace system).
 */
export function isolateInNewWorkspace(name) {
  if (!workspacesEnabled() || !name) return null;
  const fromId = activeWsObj()?.id || null;
  syncActiveFocus();
  const ws = pushWorkspace();
  // The dedicated space is named after the app it holds, so the workspace
  // switcher says "Youtube" instead of a bare number.
  ws.name = label(name);
  for (const w of workspaces) {
    if (w.id === ws.id) continue;
    w.windows = w.windows.filter((x) => x !== name);
    if (w.focus === name) w.focus = null;
    if (w.fullscreen === name) w.fullscreen = null;
  }
  ws.windows = [name];
  activeWs = ws.id;
  focus = name;
  fullscreen = null;
  syncActiveFocus();
  persist();
  notify();
  return { workspaceId: ws.id, fromId };
}

/**
 * Undo `isolateInNewWorkspace`: take the window out of whichever workspace it
 * is in now (its dedicated one, or one it was moved to while fullscreen), hand
 * it back to the workspace it came from, and drop the dedicated workspace when
 * it is left empty.
 */
export function restoreFromNewWorkspace(name, token) {
  if (!token || !name) return false;
  const dedicated = workspaces.find((w) => w.id === token.workspaceId) || null;
  const from = workspaces.find((w) => w.id === token.fromId) || null;
  for (const w of workspaces) {
    w.windows = w.windows.filter((x) => x !== name);
    if (w.focus === name) w.focus = null;
    if (w.fullscreen === name) w.fullscreen = null;
  }
  if (from) {
    if (!from.windows.includes(name)) from.windows.push(name);
    from.focus = name;
    from.fullscreen = null;
  }
  if (dedicated && !dedicated.windows.length && workspaces.length > 1) {
    workspaces = workspaces.filter((w) => w.id !== dedicated.id);
  }
  const target = (from && workspaces.includes(from)) ? from : workspaces[0] || null;
  if (target && !target.windows.includes(name)) target.windows.push(name);
  activeWs = target ? target.id : null;
  loadActiveFocus();
  focus = name;
  fullscreen = null;
  syncActiveFocus();
  persist();
  notify();
  return true;
}

/* ── Workspace mutations ────────────────────────────────────── */

function pushWorkspace(name = null) {
  const ws = { id: freshId(), windows: [], name };
  workspaces.push(ws);
  return ws;
}

/**
 * Drop an empty workspace without merging it into a neighbour. Used by
 * fullscreen.js when a fullscreen app moved on and left its dedicated space
 * behind. Refuses to remove the last workspace.
 */
export function dropWorkspace(id) {
  if (!id || !workspacesEnabled() || workspaces.length <= 1) return false;
  const ws = workspaces.find((w) => w.id === id);
  if (!ws || ws.windows.length) return false;
  workspaces = workspaces.filter((w) => w.id !== id);
  if (activeWs === id) {
    activeWs = workspaces[0].id;
    loadActiveFocus();
  }
  persist();
  notify();
  return true;
}

export function createWorkspace() {
  if (!workspacesEnabled()) return null;
  syncActiveFocus();
  const ws = pushWorkspace();
  activeWs = ws.id;
  loadActiveFocus();
  persist();
  notify();
  return ws;
}

export function removeWorkspace() {
  if (!workspacesEnabled()) return false;
  if (workspaces.length <= 1) {
    toast('Can\u2019t remove the last workspace', { type: 'error' });
    return false;
  }
  const idx = activeWorkspaceIndex();
  const ws = workspaces[idx];
  const target = workspaces[idx - 1] || workspaces[idx + 1];
  target.windows = [...target.windows, ...ws.windows];
  workspaces.splice(idx, 1);
  activeWs = target.id;
  // The merge must not carry a second window into a fullscreen workspace.
  reconcile();
  loadActiveFocus();
  syncActiveFocus();
  persist();
  notify();
  return true;
}

/** Switch workspace: 'next' | 'prev' | a 0-based index. */
export function switchWorkspace(dirOrIndex) {
  if (!workspacesEnabled()) return false;
  if (workspaces.length <= 1) return false;
  const from = activeWorkspaceIndex();
  let idx;
  let dir = 0;
  if (typeof dirOrIndex === 'number') {
    idx = dirOrIndex;
    dir = idx > from ? 1 : -1;
  } else if (dirOrIndex === 'next') {
    idx = from + 1;
    dir = 1;
  } else if (dirOrIndex === 'prev') {
    idx = from - 1;
    dir = -1;
  } else {
    return false;
  }
  if (!Number.isFinite(idx) || idx < 0) idx = workspaces.length - 1;
  if (idx >= workspaces.length) idx = 0;
  if (idx === from) return false;
  syncActiveFocus();
  activeWs = workspaces[idx].id;
  // Repair the fullscreen rule for the workspace being entered before its
  // fullscreen state is read, so a stale one can never render behind the app.
  reconcile();
  loadActiveFocus();
  persist();
  // No toast — the active-workspace dot and the window slide are the feedback.
  notify({ slide: dir });
  return true;
}

/** Move a window into a workspace (by id), then focus it there. A fullscreen
 *  window cannot leave its workspace, and nothing can be moved into one — a
 *  fullscreen app owns its screen exclusively. Both refuse silently: the
 *  context menu already greys the gesture out. */
export function moveWindow(name, toId) {
  const to = workspaces.find((w) => w.id === toId) || workspaces[0];
  if (!to) return false;
  if (workspaceLock(to)) return false;
  if (workspaceLock(workspaces.find((w) => w.windows.includes(name)))) return false;
  // A fullscreen window keeps its fullscreen state across the move; the move
  // must not drop it into the tiled layout (see workspaceLock above — a
  // fullscreen window is normally never movable at all).
  const wasFullscreen = fullscreen === name;
  for (const ws of workspaces) {
    ws.windows = ws.windows.filter((w) => w !== name);
    if (ws.focus === name) ws.focus = null;
    if (ws.fullscreen === name) ws.fullscreen = null;
  }
  if (!to.windows.includes(name)) to.windows.push(name);
  syncActiveFocus();
  activeWs = to.id;
  loadActiveFocus();
  focus = name;
  if (wasFullscreen) fullscreen = name;
  syncActiveFocus();
  persist();
  toast(`Moved ${label(name)} to workspace ${activeWorkspaceIndex() + 1}`, { type: 'info' });
  notify();
  return true;
}

/** Move a window to the workspace at a 0-based index (AI/relay-friendly).
 *  `idx` may also be the string "new" to spin up a fresh workspace, and any
 *  out-of-range index auto-creates the missing workspaces so "move to
 *  workspace 3" works even when only one workspace exists. */
export function moveWindowByIndex(name, idx) {
  if (!workspacesEnabled()) return false;
  if (idx === 'new') {
    const ws = pushWorkspace();
    if (moveWindow(name, ws.id)) return true;
    // Refused (a fullscreen app is in the way): leave no empty workspace behind.
    if (!ws.windows.length && workspaces.length > 1) {
      workspaces = workspaces.filter((w) => w.id !== ws.id);
      if (activeWs === ws.id) {
        activeWs = workspaces[0].id;
        loadActiveFocus();
      }
      persist();
    }
    return false;
  }
  const n = Number(idx);
  // Cap at 9 workspaces — an unbounded index let the AI spin up hundreds
  // of empty workspaces in a loop.
  if (!Number.isInteger(n) || n < 0 || n > 9) return false;
  while (workspaces.length <= n) pushWorkspace();
  return moveWindow(name, workspaces[n].id);
}

/* ── Layout config ──────────────────────────────────────────── */

export function getLayout() { return getDesktopLayout(); }

export function setLayout(patch) {
  setDesktopLayout({ ...getDesktopLayout(), ...patch });
  notify();
}

function label(name) {
  return name ? name.charAt(0).toUpperCase() + name.slice(1) : name;
}

/* ── Layout engine (master / stack / windows) ───────────────── */

const WIN_MIN_W = 280;
const WIN_MIN_H = 200;
const WIN_TITLE_H = 36; // keep in sync with --tile-header-height in tiles.css

function clampNum(n, min, max) {
  return Math.min(max, Math.max(min, n));
}

function defaultWindowGeom(index) {
  const step = 34;
  return {
    x: 40 + (index % 6) * step,
    y: 40 + (index % 6) * step,
    w: 560,
    h: 400,
    z: 0,
  };
}

function geomFor(name, index) {
  let g = windowGeom[name];
  if (!g || typeof g !== 'object'
    || !Number.isFinite(Number(g.x)) || !Number.isFinite(Number(g.y))
    || !Number.isFinite(Number(g.w)) || !Number.isFinite(Number(g.h))) {
    g = defaultWindowGeom(index);
    windowGeom[name] = g;
  }
  g.x = Number(g.x);
  g.y = Number(g.y);
  g.w = Number(g.w);
  g.h = Number(g.h);
  g.z = Number(g.z) || 0;
  // Assign a unique stacking order to windows that never had one, so a
  // freshly laid-out window sits deterministically above earlier ones.
  if (g.z <= 0) {
    zSeq += 1;
    g.z = zSeq;
  }
  return g;
}

function flushGeom() {
  if (geomTimer) { clearTimeout(geomTimer); geomTimer = null; }
  setWindowsGeom(windowGeom);
}

function saveGeomSoon() {
  if (geomTimer) return;
  geomTimer = setTimeout(() => {
    geomTimer = null;
    setWindowsGeom(windowGeom);
  }, 250);
}

/** Bring a window to the front (z-order) without a DOM pass; the next
 *  layout render applies the stored z-index. */
export function bumpZ(name) {
  if (!name) return;
  const g = geomFor(name, 0);
  zSeq += 1;
  g.z = zSeq;
  saveGeomSoon();
}

function applyWindowsLayout(grid, items, fs) {
  grid.style.display = 'block';
  grid.style.gridTemplateColumns = '';
  grid.style.gridTemplateRows = '';

  const W = grid.clientWidth || window.innerWidth;
  const H = grid.clientHeight || window.innerHeight;

  for (const it of items) {
    const el = it.el;
    el.style.gridColumn = '';
    el.style.gridRow = '';
    el.classList.remove('tile--master', 'tile--stack', 'hidden');
    el.classList.add('tile--window');
    wireWindowInteractions(el, it.name);
  }

  if (fs) {
    for (const it of items) {
      const shown = it.name === fs;
      it.el.classList.toggle('hidden', !shown);
      it.el.classList.toggle('tile--full', shown);
      if (shown) {
        it.el.style.left = '0px';
        it.el.style.top = '0px';
        it.el.style.width = '100%';
        it.el.style.height = '100%';
        it.el.style.zIndex = '1';
      }
    }
    markFocus(items);
    return;
  }

  for (const [i, it] of items.entries()) {
    const el = it.el;
    const g = geomFor(it.name, i);
    g.w = clampNum(g.w, WIN_MIN_W, W);
    g.h = clampNum(g.h, WIN_MIN_H, H);
    g.x = clampNum(g.x, 0, Math.max(0, W - g.w));
    g.y = clampNum(g.y, 0, Math.max(0, H - WIN_TITLE_H));
    el.classList.remove('tile--full');
    // Clear any in-flight drag offset before positioning from geometry: a
    // layout pass can land mid-drag (a workspace switch, a desktop resize), and
    // leaving the offset applied would place the window wrongly.
    if (el.style.transform) el.style.transform = '';
    el.style.left = `${g.x}px`;
    el.style.top = `${g.y}px`;
    el.style.width = `${g.w}px`;
    el.style.height = `${g.h}px`;
    el.style.zIndex = String(g.z || 1);
  }
  markFocus(items);
}

/**
 * Apply just the focus state to the mounted windows.
 *
 * A focus click changes one thing: which window carries `tile--focused` (plus,
 * in the floating layout, its z-index, which `bumpZ` already stored). It does
 * not change any window's position, size, the workspace bar, or the dock — yet
 * it used to go through `notify()` and a full `renderTiles()`, which re-applies
 * every window's geometry and rebuilds the whole workspace-bar DOM. That cost
 * 2.5 ms with two windows and 7.6 ms with eleven, entirely for nothing.
 *
 * Exported so the tiles layer can take this path on a focus-only change.
 */
export function repaintFocus() {
  document.querySelectorAll('#tile-grid .tile').forEach((el) => {
    el.classList.toggle('tile--focused', el.dataset.plugin === focus);
  });
}

/** Focus (raise) a floating window without a full re-render, so an in-flight
 *  drag isn't torn down by the layout pass. */
function raiseWindow(name, el) {
  if (!name) return;
  if (focus !== name) {
    focus = name;
    syncActiveFocus();
    persist();
  }
  const g = geomFor(name, 0);
  zSeq += 1;
  g.z = zSeq;
  saveGeomSoon();
  if (el) {
    el.style.zIndex = String(g.z);
    const grid = el.parentElement;
    if (grid) {
      grid.querySelectorAll('.tile').forEach((t) => {
        t.classList.toggle('tile--focused', t.dataset.plugin === name);
      });
    }
  }
}

function wireWindowInteractions(el, name) {
  if (el.__windowWired) return;
  el.__windowWired = true;

  const windowsMode = () => getDesktopLayout().mode === 'windows';
  // A fullscreen window owns the screen: dragging/resizing would fight the
  // fullscreen geometry (fullscreen.js drives it, and CSS pins the chrome).
  const locked = () => document.body.classList.contains('fullscreen-active');

  // Clicking anywhere on the window raises it to the front (windows mode only).
  el.addEventListener('pointerdown', () => {
    if (windowsMode()) raiseWindow(name, el);
  }, { capture: true });

  const header = el.querySelector(':scope > .tile-header');
  const resize = el.querySelector(':scope > .tile-resize');

  if (header) {
    header.addEventListener('pointerdown', (e) => {
      if (locked() || !windowsMode() || e.button !== 0 || e.target.closest('button')) return;
      e.preventDefault();
      startWindowDrag(e, el, name, header);
    });
  }
  if (resize) {
    resize.addEventListener('pointerdown', (e) => {
      if (locked() || !windowsMode() || e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      startWindowResize(e, el, name);
    });
  }
}

function startWindowDrag(e, el, name, header) {
  const g = geomFor(name, 0);
  const grid = el.parentElement;
  const W = grid.clientWidth || window.innerWidth;
  const H = grid.clientHeight || window.innerHeight;
  const startX = e.clientX;
  const startY = e.clientY;
  const origX = g.x;
  const origY = g.y;

  header.setPointerCapture?.(e.pointerId);
  header.classList.add('is-dragging');
  el.classList.add('is-dragging');

  // Drag the window on the compositor, one style update per frame.
  //
  // Two things were measured here and both were wrong. First, writing `left`
  // and `top` invalidates layout, so every write forced the whole tile subtree
  // to be re-laid-out and its blurred panes re-rasterised; a compositor-only
  // `transform` moves the already-painted layer instead. Second, the writes ran
  // once per `pointermove`, and pointermove fires several times per presented
  // frame (1768 events in one 12 s drag here) — so the work was a multiple of
  // what any display could show. Coalescing into one rAF tick bounds it to at
  // most one update per frame.
  //
  // Result of doing both: dragging a plugin window went from 4.6 fps with 50
  // stalls over 50 ms to a steady 30 fps with none.
  let dx = 0;
  let dy = 0;
  let pending = null;
  let queued = false;

  const apply = () => {
    queued = false;
    if (pending === null) return;
    const { x, y } = pending;
    pending = null;
    // The element is positioned at (origX, origY) by `left`/`top`; the drag is
    // expressed as an offset from there, which the compositor can apply without
    // touching layout.
    //
    // Two-dimensional `translate`, deliberately not `translate3d`. The 3D form
    // forces the tile into its own GPU layer, and this window carries an ambient
    // blurred glow (`.tile-glow`) that overhangs the window by 6% and sits at
    // z-index -1 inside it. Promoting the tile puts that overhanging layer
    // through a different compositing path than it was authored for, which is a
    // real visual change for a performance win that `translate` already gives
    // (a 2D transform is still a compositor-only operation).
    el.style.transform = `translate(${x}px, ${y}px)`;
  };

  const move = (ev) => {
    dx = clampNum(origX + (ev.clientX - startX), 0, Math.max(0, W - g.w)) - origX;
    dy = clampNum(origY + (ev.clientY - startY), 0, Math.max(0, H - WIN_TITLE_H)) - origY;
    pending = { x: dx, y: dy };
    if (!queued) {
      queued = true;
      requestAnimationFrame(apply);
    }
  };
  const up = () => {
    header.removeEventListener('pointermove', move);
    header.removeEventListener('pointerup', up);
    header.removeEventListener('pointercancel', up);
    header.classList.remove('is-dragging');
    el.classList.remove('is-dragging');

    // Drain any queued frame synchronously (rather than cancelling it by id we
    // would have to track), so the committed position is the final one.
    if (queued) apply();
    pending = null;
    g.x = clampNum(origX + dx, 0, Math.max(0, W - g.w));
    g.y = clampNum(origY + dy, 0, Math.max(0, H - WIN_TITLE_H));
    el.style.transform = '';
    el.style.left = `${g.x}px`;
    el.style.top = `${g.y}px`;
    flushGeom();
  };
  header.addEventListener('pointermove', move);
  header.addEventListener('pointerup', up);
  header.addEventListener('pointercancel', up);
}

function startWindowResize(e, el, name) {
  const g = geomFor(name, 0);
  const grid = el.parentElement;
  const W = grid.clientWidth || window.innerWidth;
  const H = grid.clientHeight || window.innerHeight;
  const startX = e.clientX;
  const startY = e.clientY;
  const origW = g.w;
  const origH = g.h;

  el.setPointerCapture?.(e.pointerId);
  el.classList.add('is-resizing');

  const move = (ev) => {
    g.w = clampNum(origW + (ev.clientX - startX), WIN_MIN_W, Math.max(WIN_MIN_W, W - g.x));
    g.h = clampNum(origH + (ev.clientY - startY), WIN_MIN_H, Math.max(WIN_MIN_H, H - g.y));
    el.style.width = `${g.w}px`;
    el.style.height = `${g.h}px`;
    saveGeomSoon();
  };
  const up = () => {
    el.removeEventListener('pointermove', move);
    el.removeEventListener('pointerup', up);
    el.removeEventListener('pointercancel', up);
    el.classList.remove('is-resizing');
    flushGeom();
  };
  el.addEventListener('pointermove', move);
  el.addEventListener('pointerup', up);
  el.addEventListener('pointercancel', up);
}

/**
 * Arrange tile elements inside `grid`. `items` = [{ name, el }] for the active
 * workspace's windows (already mounted). Handles fullscreen, single-window,
 * legacy columns, master/stack, and the floating "Windows" desktop.
 */
export function applyLayout(grid, items) {
  const layout = getDesktopLayout();
  const ratio = layout.master_ratio;
  const ori = layout.orientation;

  grid.style.gap = `${layout.gap}px`;
  grid.classList.remove('tile-grid--master', 'tile-grid--columns', 'tile-grid--windows');
  grid.classList.add(
    layout.mode === 'master' ? 'tile-grid--master'
      : layout.mode === 'windows' ? 'tile-grid--windows'
      : 'tile-grid--columns'
  );
  grid.dataset.layout = layout.mode;

  // A fullscreen state that does not name a window of THIS workspace is never
  // rendered: a stale pointer is ignored rather than allowed to make whichever
  // window opens next come up fullscreen. (It is repaired by `reconcile()`
  // before the next workspace switch or activation.)
  const fs = fullscreen && items.some((i) => i.name === fullscreen) ? fullscreen : null;

  if (layout.mode === 'windows') {
    applyWindowsLayout(grid, items, fs);
    return;
  }

  // Tiling modes — strip any floating-window geometry/inline styles so the
  // grid/flex engine owns positioning again.
  for (const it of items) {
    it.el.style.left = '';
    it.el.style.top = '';
    it.el.style.width = '';
    it.el.style.height = '';
    it.el.style.zIndex = '';
    it.el.classList.remove('tile--window');
    it.el.style.gridColumn = '';
    it.el.style.gridRow = '';
    it.el.classList.remove('tile--master', 'tile--stack', 'tile--full', 'hidden');
  }

  if (fs) {
    for (const it of items) {
      it.el.classList.toggle('hidden', it.name !== fs);
      if (it.name === fs) it.el.classList.add('tile--full');
    }
    grid.style.display = 'grid';
    grid.style.gridTemplateColumns = 'minmax(0, 1fr)';
    grid.style.gridTemplateRows = 'minmax(0, 1fr)';
    return;
  }

  if (items.length <= 1) {
    if (items.length === 1) items[0].el.classList.add('tile--full');
    grid.style.display = 'grid';
    grid.style.gridTemplateColumns = 'minmax(0, 1fr)';
    grid.style.gridTemplateRows = 'minmax(0, 1fr)';
    return;
  }

  if (layout.mode !== 'master') {
    // Legacy columns: CSS flex-wrap (see tiles.css) owns the layout.
    grid.style.display = 'flex';
    grid.style.gridTemplateColumns = '';
    grid.style.gridTemplateRows = '';
    markFocus(items);
    return;
  }

  // Master/stack — the focused window (or first) is the master.
  const masterName = (focus && items.some((i) => i.name === focus)) ? focus : items[0].name;
  const master = items.find((i) => i.name === masterName) || items[0];
  const stack = items.filter((i) => i !== master);
  const n = stack.length;

  grid.style.display = 'grid';

  if (ori === 'top' || ori === 'bottom') {
    grid.style.gridTemplateColumns = `repeat(${Math.max(1, n)}, minmax(0, 1fr))`;
    grid.style.gridTemplateRows = ori === 'top'
      ? `minmax(0, ${ratio}fr) minmax(0, ${1 - ratio}fr)`
      : `minmax(0, ${1 - ratio}fr) minmax(0, ${ratio}fr)`;
    const masterRow = ori === 'top' ? '1' : '2';
    master.el.style.gridRow = `${masterRow} / span 1`;
    master.el.style.gridColumn = '1 / -1';
    stack.forEach((it, i) => {
      it.el.style.gridRow = ori === 'top' ? '2' : '1';
      it.el.style.gridColumn = `${i + 1}`;
    });
  } else {
    const masterCol = ori === 'right' ? '2' : '1';
    const stackCol = ori === 'right' ? '1' : '2';
    grid.style.gridTemplateColumns = ori === 'right'
      ? `minmax(0, ${1 - ratio}fr) minmax(0, ${ratio}fr)`
      : `minmax(0, ${ratio}fr) minmax(0, ${1 - ratio}fr)`;
    grid.style.gridTemplateRows = `repeat(${Math.max(1, n)}, minmax(0, 1fr))`;
    master.el.style.gridColumn = `${masterCol}`;
    master.el.style.gridRow = '1 / -1';
    stack.forEach((it, i) => {
      it.el.style.gridColumn = `${stackCol}`;
      it.el.style.gridRow = `${i + 1}`;
    });
  }

  master.el.classList.add('tile--master');
  stack.forEach((it) => it.el.classList.add('tile--stack'));
  markFocus(items);
}

function markFocus(items) {
  for (const it of items) {
    it.el.classList.toggle('tile--focused', it.name === focus);
  }
}

/* ── Workspace bar (clickable dots + add/remove) ────────────── */

const WS_SHORTCUT_ENABLED = true;

export function renderWorkspaceBar() {
  const bar = document.getElementById('workspace-bar');
  if (!bar) return;
  if (!workspacesEnabled()) {
    // One column, one workspace: the switcher has nothing to switch.
    bar.classList.add('hidden');
    bar.textContent = '';
    return;
  }
  // Always visible — even with no plugin windows the switcher lets you
  // create/manage workspaces (dots render empty, +/− still work).
  bar.classList.remove('hidden');
  bar.textContent = '';

  const dots = document.createElement('div');
  dots.className = 'workspace-bar-dots';

  workspaces.forEach((ws, i) => {
    const dot = document.createElement('button');
    dot.type = 'button';
    dot.className = 'workspace-bar-dot';
    // A dedicated fullscreen workspace shows the app's own icon instead of a
    // number — the workspace is that app. The number stays in the label for
    // Alt+1..9.
    const app = workspaceLock(ws);
    if (app) {
      dot.classList.add('is-app');
      dot.appendChild(pluginIconEl(app, { size: 15, fallback: 'ui/puzzle' }));
    } else {
      dot.textContent = String(i + 1);
    }
    const wsLabel = workspaceLabel(ws);
    dot.title = wsLabel;
    dot.setAttribute('aria-label', wsLabel);
    dot.classList.toggle('is-active', ws.id === activeWs);
    dot.addEventListener('click', () => {
      if (i !== activeWorkspaceIndex()) switchWorkspace(i);
    });
    dots.appendChild(dot);
  });

  const add = document.createElement('button');
  add.type = 'button';
  add.className = 'workspace-bar-btn';
  add.title = 'New workspace';
  add.setAttribute('aria-label', 'New workspace');
  add.appendChild(icon('ui/plus', { size: 14 }));
  add.addEventListener('click', () => createWorkspace());

  const del = document.createElement('button');
  del.type = 'button';
  del.className = 'workspace-bar-btn';
  del.title = 'Remove workspace';
  del.setAttribute('aria-label', 'Remove workspace');
  del.disabled = workspaces.length <= 1;
  del.appendChild(icon('ui/minus', { size: 14 }));
  del.addEventListener('click', () => removeWorkspace());

  bar.append(add, dots, del);
}

/* ── Physical keybindings (Alt = the "Super" mod) ───────────── */

function isEditableEl(el) {
  if (!el || el.nodeType !== 1) return false;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) {
    return ['text', 'password', 'email', 'search', 'number', 'tel', 'url'].includes(el.type);
  }
  return el.isContentEditable === true;
}

function wireShortcuts() {
  document.addEventListener('keydown', (e) => {
    if (!WS_SHORTCUT_ENABLED) return;
    if (e.ctrlKey || e.metaKey) return;           // don't fight browser/OS
    if (isEditableEl(e.target)) return;           // typing wins
    if (!e.altKey) return;

    const k = e.key;

    if (k === 'Enter') {
      e.preventDefault();
      // Real fullscreen (its own workspace + the browser Fullscreen API) is
      // owned by fullscreen.js, so the shortcut and the window button agree.
      // It falls back to an in-app fullscreen when the browser says no.
      window.dispatchEvent(new CustomEvent('app:fullscreen-toggle'));
      return;
    }
    // Workspace jump: Alt+1..9
    if (/^[1-9]$/.test(k)) {
      if (!workspacesEnabled()) return;
      e.preventDefault();
      const idx = Number(k) - 1;
      if (idx !== activeWorkspaceIndex()) switchWorkspace(idx);
      return;
    }
    switch (k) {
      case 'h': e.preventDefault(); cycleFocus(namesForShortcuts(), -1); break;
      case 'l': e.preventDefault(); cycleFocus(namesForShortcuts(), 1); break;
      case ',': if (!workspacesEnabled()) return; e.preventDefault(); switchWorkspace('prev'); break;
      case '.': if (!workspacesEnabled()) return; e.preventDefault(); switchWorkspace('next'); break;
      case 'n':
        if (!workspacesEnabled()) return;
        e.preventDefault();
        if (e.shiftKey) removeWorkspace();
        else createWorkspace();
        break;
      default: break;
    }
  });
}

/** Surface names for shortcuts come from the tile manager's current set. */
let surfaceNamesProvider = () => [];

export function setSurfaceNamesProvider(fn) {
  surfaceNamesProvider = fn;
}

function namesForShortcuts() {
  return surfaceNamesProvider() || [];
}

/** Convenience wrappers so the Keyboard plugin doesn't need to know the
 *  surface list — the desktop manager asks the tile manager for it. */
export function cycleFocusActive(dir = 1) {
  cycleFocus(namesForShortcuts(), dir);
}

/** In-app fullscreen (the window fills the desktop, chrome stays). The
 *  user-facing fullscreen — its own workspace + the browser Fullscreen API —
 *  is `toggleWindowFullscreen` in fullscreen.js, which is what the window
 *  button, Alt+Enter and the context menus call. */
export function toggleFullscreenActive() {
  return toggleFullscreen(getFocus() || namesForShortcuts()[0]);
}
