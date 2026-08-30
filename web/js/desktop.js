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
} from './preferences.js';
import { toast } from '../ui/index.js';

let workspaces = [];      // [{ id, windows: [pluginName], focus, fullscreen }]
let activeWs = null;      // active workspace id
let focus = null;         // focused plugin name (cache of active workspace's)
let fullscreen = null;    // fullscreen plugin name (cache of active workspace's)

let wsSeq = 0;

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

/** Notify tiles.js to re-render after any state change. */
function notify() {
  window.dispatchEvent(new Event('desktop:changed'));
}

export function initDesktop() {
  workspaces = getWorkspaces() || [];
  activeWs = getActiveWorkspaceId();
  if (!workspaces.length || !workspaces.some((w) => w.id === activeWs)) {
    activeWs = workspaces.length ? workspaces[0].id : null;
  }
  loadActiveFocus();
  wireShortcuts();
  wireAgentActions();
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
 * Guarantee every plugin surface lives in a workspace: migrate a first load
 * (one workspace with everything), prune deactivated surfaces, and add newly
 * activated ones to the active workspace.
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
  loadActiveFocus();
  const assigned = new Set(workspaces.flatMap((ws) => ws.windows));
  const aws = activeWsObj();
  for (const n of names) {
    if (!assigned.has(n)) {
      aws.windows.push(n);
      assigned.add(n);
    }
  }
  if (!workspaces.some((w) => w.id === activeWs)) activeWs = workspaces[0].id;
  persist();
}

/** Windows of the active workspace that still exist, in workspace order. */
export function activeWindowNames(allNames) {
  const ws = activeWsObj();
  if (!ws) return [...allNames];
  return ws.windows.filter((w) => allNames.includes(w));
}

export function workspaceCount() { return workspaces.length; }

export function activeWorkspaceIndex() {
  return Math.max(0, workspaces.findIndex((w) => w.id === activeWs));
}

export function getWorkspacesList() { return workspaces; }

export function workspaceHasWindow(name) {
  return workspaces.some((w) => w.windows.includes(name));
}

/** Snapshot of the desktop sent to the AI on every request (1-based indices),
 *  so it knows which windows live in which workspace before reorganizing. */
export function getDesktopSnapshot() {
  return {
    active: workspaces.length ? activeWorkspaceIndex() + 1 : 1,
    workspaces: workspaces.map((ws, i) => ({
      index: i + 1,
      windows: [...ws.windows],
    })),
  };
}

/* ── Focus / fullscreen ─────────────────────────────────────── */

export function getFocus() { return focus; }
export function getFullscreen() { return fullscreen; }

/** Focus a window and switch to whichever workspace holds it. */
export function focusWindow(name) {
  if (!name) return;
  const ws = workspaces.find((w) => w.windows.includes(name));
  if (ws && ws.id !== activeWs) {
    syncActiveFocus();
    activeWs = ws.id;
    loadActiveFocus();
  }
  focus = name;
  syncActiveFocus();
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
  persist();
  notify();
}

export function clearFocus() {
  focus = null;
  syncActiveFocus();
  persist();
  notify();
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

/* ── Workspace mutations ────────────────────────────────────── */

function pushWorkspace() {
  const ws = { id: freshId(), windows: [] };
  workspaces.push(ws);
  return ws;
}

export function createWorkspace() {
  syncActiveFocus();
  const ws = pushWorkspace();
  activeWs = ws.id;
  loadActiveFocus();
  persist();
  toast(`Workspace ${activeWorkspaceIndex() + 1}`, { type: 'info' });
  notify();
  return ws;
}

export function removeWorkspace() {
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
  loadActiveFocus();
  syncActiveFocus();
  persist();
  toast('Workspace removed', { type: 'info' });
  notify();
  return true;
}

/** Switch workspace: 'next' | 'prev' | a 0-based index. */
export function switchWorkspace(dirOrIndex) {
  if (workspaces.length <= 1) return false;
  let idx;
  if (typeof dirOrIndex === 'number') {
    idx = dirOrIndex;
  } else if (dirOrIndex === 'next') {
    idx = activeWorkspaceIndex() + 1;
  } else if (dirOrIndex === 'prev') {
    idx = activeWorkspaceIndex() - 1;
  } else {
    return false;
  }
  if (!Number.isFinite(idx) || idx < 0) idx = workspaces.length - 1;
  if (idx >= workspaces.length) idx = 0;
  if (idx === activeWorkspaceIndex()) return false;
  syncActiveFocus();
  activeWs = workspaces[idx].id;
  loadActiveFocus();
  persist();
  toast(`Workspace ${idx + 1}`, { type: 'info' });
  notify();
  return true;
}

/** Move a window into a workspace (by id), then focus it there. */
export function moveWindow(name, toId) {
  const to = workspaces.find((w) => w.id === toId) || workspaces[0];
  if (!to) return false;
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
  if (idx === 'new') {
    const ws = pushWorkspace();
    return moveWindow(name, ws.id);
  }
  const n = Number(idx);
  if (!Number.isInteger(n) || n < 0) return false;
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

/* ── Layout engine (master / stack) ─────────────────────────── */

/**
 * Arrange tile elements inside `grid`. `items` = [{ name, el }] for the active
 * workspace's windows (already mounted). Handles fullscreen, single-window,
 * legacy columns, and master/stack.
 */
export function applyLayout(grid, items) {
  const layout = getDesktopLayout();
  const ratio = layout.master_ratio;
  const ori = layout.orientation;

  grid.style.gap = `${layout.gap}px`;
  grid.classList.remove('tile-grid--master', 'tile-grid--columns');
  grid.classList.add(layout.mode === 'master' ? 'tile-grid--master' : 'tile-grid--columns');
  grid.dataset.layout = layout.mode;

  for (const it of items) {
    it.el.style.gridColumn = '';
    it.el.style.gridRow = '';
    it.el.classList.remove('tile--master', 'tile--stack', 'tile--full', 'hidden');
  }

  const fs = fullscreen && items.some((i) => i.name === fullscreen) ? fullscreen : null;

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

export function renderWorkspaceBar(hasWindows = true) {
  const bar = document.getElementById('workspace-bar');
  if (!bar) return;
  bar.classList.toggle('hidden', !hasWindows);
  if (!hasWindows) return;
  bar.textContent = '';

  const dots = document.createElement('div');
  dots.className = 'workspace-bar-dots';

  workspaces.forEach((ws, i) => {
    const dot = document.createElement('button');
    dot.type = 'button';
    dot.className = 'workspace-bar-dot';
    dot.textContent = String(i + 1);
    dot.title = `Workspace ${i + 1}`;
    dot.setAttribute('aria-label', `Workspace ${i + 1}`);
    dot.classList.toggle('is-active', ws.id === activeWs);
    dot.addEventListener('click', () => {
      if (i !== activeWorkspaceIndex()) switchWorkspace(i);
    });
    dots.appendChild(dot);
  });

  const add = document.createElement('button');
  add.type = 'button';
  add.className = 'workspace-bar-btn';
  add.textContent = '+';
  add.title = 'New workspace';
  add.setAttribute('aria-label', 'New workspace');
  add.addEventListener('click', () => createWorkspace());

  const del = document.createElement('button');
  del.type = 'button';
  del.className = 'workspace-bar-btn';
  del.textContent = '\u2212';
  del.title = 'Remove workspace';
  del.setAttribute('aria-label', 'Remove workspace');
  del.disabled = workspaces.length <= 1;
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
      toggleFullscreen();
      return;
    }
    // Workspace jump: Alt+1..9
    if (/^[1-9]$/.test(k)) {
      e.preventDefault();
      const idx = Number(k) - 1;
      if (idx !== activeWorkspaceIndex()) switchWorkspace(idx);
      return;
    }
    switch (k) {
      case 'h': e.preventDefault(); cycleFocus(namesForShortcuts(), -1); break;
      case 'l': e.preventDefault(); cycleFocus(namesForShortcuts(), 1); break;
      case ',': e.preventDefault(); switchWorkspace('prev'); break;
      case '.': e.preventDefault(); switchWorkspace('next'); break;
      case 'n':
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

export function toggleFullscreenActive() {
  return toggleFullscreen(getFocus() || namesForShortcuts()[0]);
}
