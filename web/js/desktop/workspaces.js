import { controlButton } from '../../ui/components/button.js';
import { createLayoutController } from './layout.js';
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
  getPluginFocus as readPluginFocus, setPluginFocus as savePluginFocus,
} from '../preferences.js';
import { toast, icon } from '../../ui/index.js';
import { pluginIconEl } from '../pluginIcon.js';
import { isMobileScreen, MOBILE_QUERY } from '../viewport.js';
import { t } from '../i18n.js';

let workspaces = [];      // [{ id, windows: [pluginName], focus, fullscreen }]
let activeWs = null;      // active workspace id
let focus = null;         // focused plugin name (cache of active workspace's)
let fullscreen = null;    // fullscreen plugin name (cache of active workspace's)
/** Plugins in "focus mode" — a deliberate choice, by the user or the AI, never
 *  by an open, and several plugins can be focused at once. It paints the
 *  palette frame, lights the tray underline and scopes the AI's plugin skills
 *  to these plugins for the turn. Unlike `focus`/`fullscreen` it is NOT part of
 *  a workspace: it is the working set, not a place on screen, so the AI knows
 *  about it whichever workspace is up. */
let focusedPlugins = new Set();

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
  return !isMobileScreen();
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
  wasMobileScreen = isMobileScreen();
  workspaces = getWorkspaces() || [];
  activeWs = getActiveWorkspaceId();
  if (!workspaces.length || !workspaces.some((w) => w.id === activeWs)) {
    activeWs = workspaces.length ? workspaces[0].id : null;
  }
  windowGeom = getWindowsGeom();
  zSeq = Object.values(windowGeom).reduce((m, g) => Math.max(m, Number(g?.z) || 0), 0);
  loadActiveFocus();
  // Focus mode is global, so it is restored from its own key rather than from a
  // workspace. Pruned on the first render against whatever windows exist.
  focusedPlugins = new Set(readPluginFocus());
  wireShortcuts();
  wireAgentActions();
  // Turning the screen switches the workspace system off or on. The media
  // query is the precise signal; `resize` is the fallback, because a few
  // browsers (and embedded webviews) only deliver that one. Both funnel into
  // the same transition check so a resize that changes nothing is free.
  MOBILE_QUERY.addEventListener('change', onScreenTurn);
  window.addEventListener('resize', onScreenTurn);
  // The handles are positioned in pixels, so keep them on the gaps when the
  // viewport changes without a full re-layout.
  window.addEventListener('resize', () => {
    const grid = document.getElementById('tile-grid');
    if (grid) repositionSplitters(grid);
  });
}

let wasMobileScreen = null;

function onScreenTurn() {
  const now = isMobileScreen();
  if (now === wasMobileScreen) return;
  const first = wasMobileScreen === null;
  wasMobileScreen = now;
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
          // Ask fullscreen.js for what the window button does — its own
          // workspace, the chrome takeover, real fullscreen when the browser
          // allows it. The desktop half lives here; the *effect* must not depend
          // on the browser's answer, since an AI action carries no user gesture.
          if (d.plugin) {
            window.dispatchEvent(new CustomEvent('app:fullscreen-window', {
              detail: { name: d.plugin, on: d.fullscreen !== false },
            }));
          }
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
 *     to any ordinary workspace) and stays non-fullscreen. Several workspaces
 *     may be sealed this way at once — one fullscreen window each;
 *   • a dedicated fullscreen workspace (one with a `home`) left with no windows
 *     at all is dropped.
 *
 * Idempotent, so it is safe to call from every mutation and every render.
 */
function reconcile() {
  if (!workspacesEnabled() || !workspaces.length) return false;
  let changed = false;

  // A dedicated fullscreen workspace whose window is gone (a deactivation, a
  // prune during a render, or a leftover from an older build) is dropped rather
  // than kept as an empty space named after an app that is no longer there.
  if (workspaces.length > 1) {
    const kept = workspaces.filter((w) => !(w.home && !w.windows.length));
    if (kept.length !== workspaces.length) {
      workspaces = kept;
      changed = true;
      if (!workspaces.some((w) => w.id === activeWs)) activeWs = workspaces[0].id;
    }
  }

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

/** True while any workspace still holds a fullscreen window, whichever one is on
 *  screen. fullscreen.js keeps real browser fullscreen until this goes false. */
export function hasFullscreenWindow() {
  return workspaces.some((w) => w.fullscreen && w.windows.includes(w.fullscreen));
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
  return ws.name
    ? t('desktop.workspaceNamed', { n: i + 1, name: ws.name }, `Workspace ${i + 1} — ${ws.name}`)
    : t('ctx.workspaceNumber', { n: i + 1 });
}

/** Snapshot of the desktop sent to the AI on every request (1-based indices),
 *  so it knows which windows live in which workspace before reorganizing — and
 *  which plugins the user has focused (`focused`), which decides whether the
 *  turn runs in focus mode. */
export function getDesktopSnapshot() {
  if (!workspacesEnabled()) {
    return {
      active: 1,
      workspaces: [{ index: 1, windows: everyWindow() }],
      workspaces_enabled: false,
      focused: [...focusedPlugins],
    };
  }
  return {
    active: workspaces.length ? activeWorkspaceIndex() + 1 : 1,
    workspaces: workspaces.map((ws, i) => ({
      index: i + 1,
      windows: [...ws.windows],
    })),
    workspaces_enabled: true,
    focused: [...focusedPlugins],
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

/** Bring a window on screen *without* making it the active (focused) one: switch
 *  to the workspace that holds it. Used when a window opens — a new window must
 *  show up, but the active-window accent belongs to a click and focus mode
 *  belongs to the user. The stacking order of a window being opened is already
 *  handled by `renderTiles`, which bumps it before the layout applies. */
export function revealWindow(name) {
  if (!name) return;
  const ws = workspaces.find((w) => w.windows.includes(name));
  if (!ws || ws.id === activeWs) return;
  syncActiveFocus();
  activeWs = ws.id;
  loadActiveFocus();
  persist();
  notify();
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

/* ── Plugin focus mode ──────────────────────────────────────── */

export function isPluginFocused(name) { return !!name && focusedPlugins.has(name); }

/**
 * Focus (or unfocus) a plugin's window: "focus mode", where the system limits the
 * AI's tools to the focused plugins and the window/HUD wear the accent palette.
 * Only the user sets it — the AI can just suggest it — and it is always a
 * deliberate act: opening, clicking or fullscreening a window does not focus it.
 * Several plugins can be focused at once.
 */
export function setPluginFocus(name, on = true) {
  if (!name) return false;
  if (focusedPlugins.has(name) === !!on) return false;
  if (on) focusedPlugins.add(name); else focusedPlugins.delete(name);
  savePluginFocus([...focusedPlugins]);
  renderWorkspaceBar();
  notify('focus');
  return true;
}

export function togglePluginFocus(name) {
  return setPluginFocus(name, !isPluginFocused(name));
}

/** A focus entry only lives while its window does: drop the ones whose plugin is
 *  not in `openNames` (the plugins that still have a window — the caller knows,
 *  and must not pass an empty list before the plugin set has loaded). Silent, so
 *  a render pass can call it without causing one. */
function prunePluginFocus(openNames) {
  if (!focusedPlugins.size) return false;
  const open = new Set(openNames || []);
  let changed = false;
  for (const name of [...focusedPlugins]) {
    if (open.has(name)) continue;
    focusedPlugins.delete(name);
    changed = true;
  }
  return changed;
}

/** Prune focus entries whose window is gone (a plugin was deactivated) and
 *  repaint the HUD/chrome if anything moved. */
export function reconcilePluginFocus(openNames) {
  if (!prunePluginFocus(openNames)) return false;
  savePluginFocus([...focusedPlugins]);
  renderWorkspaceBar();
  notify('focus');
  return true;
}

/** Toggle (or force) the in-app fullscreen pointer for a window — the low-level
 *  state. The user-facing entry is `enterWindowFullscreen` (its own workspace,
 *  the chrome takeover, the browser request) and the way out is
 *  `exitWindowFullscreen` (hands the workspace back), both in fullscreen.js;
 *  this setter alone would leave a workspace sealed around nothing. */
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

/** Forget the in-app fullscreen pointer without handing the window anywhere. A
 *  repair, not a way out of fullscreen: only for screens with no workspace
 *  system (fullscreen.js use); everywhere else the workspace must be unsealed
 *  and the window sent home through `restoreFromNewWorkspace`. */
export function clearFullscreen() {
  fullscreen = null;
  syncActiveFocus();
  persist();
  notify();
}

/**
 * Move a window into a brand-new dedicated workspace and make it that
 * workspace's fullscreen window — the desktop half of a fullscreen window
 * (fullscreen.js does the browser half, and the chrome).
 *
 * The new workspace remembers where the window came from (`ws.home`), and that
 * record is persisted with the list, so leaving fullscreen hands the window back
 * even after a reload. Several windows can own a workspace like this at once,
 * one each. Returns the new workspace id, or null when the window has no
 * workspace system to be isolated in (vertical phone screens).
 */
export function isolateInNewWorkspace(name) {
  if (!workspacesEnabled() || !name) return null;
  // Where the window lives is where it is handed back to: the workspace holding
  // it, which is not necessarily the one on screen (the AI can fullscreen
  // either, and the user may have moved on since).
  const home = workspaces.find((w) => w.windows.includes(name))?.id
    || activeWsObj()?.id
    || null;
  syncActiveFocus();
  const ws = pushWorkspace();
  // The dedicated space is named after the app it holds, so the workspace
  // switcher says "Youtube" instead of a bare number.
  ws.name = label(name);
  ws.home = home;
  for (const w of workspaces) {
    if (w.id === ws.id) continue;
    w.windows = w.windows.filter((x) => x !== name);
    if (w.focus === name) w.focus = null;
    if (w.fullscreen === name) w.fullscreen = null;
  }
  ws.windows = [name];
  activeWs = ws.id;
  focus = name;
  fullscreen = name;
  syncActiveFocus();
  persist();
  notify();
  return ws.id;
}

/**
 * Undo `isolateInNewWorkspace`: take the window out of whichever workspace it is
 * in now (its dedicated one, or one it was moved to while fullscreen), hand it
 * back to the workspace it came from, and drop the dedicated workspace when it
 * is left empty. The home is read from the workspace itself, so this works for
 * every fullscreen window and survives a reload.
 */
export function restoreFromNewWorkspace(name) {
  if (!workspacesEnabled() || !name) return false;
  const dedicated = workspaces.find((w) => w.windows.includes(name)) || null;
  const from = workspaces.find((w) => w.id === dedicated?.home) || null;
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
    toast(t('desktop.lastWorkspace', null, 'Can\u2019t remove the last workspace'), { type: 'error' });
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
  toast(t('desktop.movedToWorkspace', { name: label(name), n: activeWorkspaceIndex() + 1 },
    `Moved ${label(name)} to workspace ${activeWorkspaceIndex() + 1}`), { type: 'info' });
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

const layoutController = createLayoutController({
  get focus() { return focus; }, set focus(value) { focus = value; },
  get fullscreen() { return fullscreen; },
  get windowGeom() { return windowGeom; },
  get zSeq() { return zSeq; }, set zSeq(value) { zSeq = value; },
  get geomTimer() { return geomTimer; }, set geomTimer(value) { geomTimer = value; },
}, { getDesktopLayout, setWindowsGeom, syncActiveFocus, persist, setLayout });

export function bumpZ(name) { return layoutController.bumpZ(name); }
export function applyLayout(grid, items) { return layoutController.applyLayout(grid, items); }
export function repaintFocus() { return layoutController.repaintFocus(); }
function flushGeom() { return layoutController.flushGeom(); }
function repositionSplitters(grid) { return layoutController.repositionSplitters(grid); }

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
    const dot = controlButton();
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
    // Focus mode is marked on the bar under the plugin tray: the dot of a
    // workspace holding focused windows wears the accent.
    dot.classList.toggle('is-focused', ws.windows.some((n) => focusedPlugins.has(n)));
    dot.addEventListener('click', () => {
      if (i !== activeWorkspaceIndex()) switchWorkspace(i);
    });
    dots.appendChild(dot);
  });

  const add = controlButton();
  add.type = 'button';
  add.className = 'workspace-bar-btn';
  add.title = t('ctx.newWorkspace');
  add.setAttribute('aria-label', t('ctx.newWorkspace'));
  add.appendChild(icon('ui/plus', { size: 14 }));
  add.addEventListener('click', () => createWorkspace());

  const del = controlButton();
  del.type = 'button';
  del.className = 'workspace-bar-btn';
  del.title = t('ctx.removeWorkspace');
  del.setAttribute('aria-label', t('ctx.removeWorkspace'));
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
