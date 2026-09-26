/**
 * overview.js — the window overview (three-finger swipe up).
 *
 * Mission-Control for the desktop: every open plugin window, from every
 * workspace, laid out as a live grid over a dimmed backdrop. The windows are
 * the real tiles, resized and repositioned in place — nothing is cloned and no
 * iframe is torn down, so a running video or a half-typed document survives the
 * trip. Clicking a window focuses it (switching to its workspace) and closes
 * the overview.
 *
 * The gesture is recognised natively (crates/peakd/src/gestures.rs) and mapped
 * here by gestures.js; Escape or a click on the backdrop also closes.
 */
import { focusWindow, getWorkspacesList } from './desktop.js';
import { refreshTiles } from './tiles.js';
import { fullscreenWindow, exitWindowFullscreen } from './fullscreen.js';

const PAD = 40;   // margin of the grid region inside #tile-grid
const GAP = 16;   // between cells

let grid = null;
let backdrop = null;
let open = false;
let swallowNextClick = false;

export function isOverviewOpen() {
  return open;
}

/** Every mounted plugin window, active workspace or not. */
function tiles() {
  return [...document.querySelectorAll('#tile-grid > .tile[data-plugin]')];
}

function shape(count, regionW, regionH) {
  let bestCols = 1;
  let bestScore = -Infinity;
  for (let cols = 1; cols <= count; cols += 1) {
    const rows = Math.ceil(count / cols);
    const cellW = (regionW - (cols - 1) * GAP) / cols;
    const cellH = (regionH - (rows - 1) * GAP) / rows;
    if (cellW <= 0 || cellH <= 0) continue;
    // Prefer cells near a 16:10 window and a full last row.
    const aspect = Math.log((cellW / cellH) / 1.6);
    const empty = cols * rows - count;
    const score = -Math.abs(aspect) - empty * 0.05;
    if (score > bestScore) {
      bestScore = score;
      bestCols = cols;
    }
  }
  return { cols: bestCols, rows: Math.ceil(count / bestCols) };
}

/** A "Workspace n" chip in each window's own header. */
function addBadge(el, name) {
  const header = el.querySelector(':scope > .tile-header');
  if (!header) return;
  const list = getWorkspacesList();
  const index = list.findIndex((ws) => (ws.windows || []).includes(name));
  const badge = document.createElement('span');
  badge.className = 'overview-badge';
  badge.textContent = index >= 0 ? `WS ${index + 1}` : 'WS';
  badge.dataset.overview = 'badge';
  header.appendChild(badge);
}

function clearBadges() {
  document.querySelectorAll('#tile-grid .overview-badge').forEach((b) => b.remove());
}

function layout() {
  clearBadges();
  const list = tiles();
  const n = list.length;
  if (n === 0) return;

  // Positions are relative to the grid's own box (it is `position: fixed` and
  // offset under the top bar), so measure it rather than the viewport.
  const box = grid.getBoundingClientRect();
  const regionW = box.width - PAD * 2;
  const regionH = box.height - PAD * 2;
  if (regionW <= 0 || regionH <= 0) return;

  const { cols, rows } = shape(n, regionW, regionH);
  const cellW = (regionW - (cols - 1) * GAP) / cols;
  const cellH = (regionH - (rows - 1) * GAP) / rows;

  list.forEach((el, i) => {
    const col = i % cols;
    const row = Math.floor(i / cols);
    el.classList.remove('hidden');
    el.classList.add('tile--overview');
    // The overview owns position/size: drop the tiling/fullscreen placement.
    el.style.gridColumn = '';
    el.style.gridRow = '';
    el.style.left = `${Math.round(PAD + col * (cellW + GAP))}px`;
    el.style.top = `${Math.round(PAD + row * (cellH + GAP))}px`;
    el.style.width = `${Math.round(cellW)}px`;
    el.style.height = `${Math.round(cellH)}px`;
    el.style.zIndex = String(10 + i);
    addBadge(el, el.dataset.plugin);
  });
}

/** Restore the desktop layout after the overview. */
function restore() {
  clearBadges();
  document.body.classList.remove('overview-active');
  backdrop?.classList.add('hidden');
  for (const el of tiles()) {
    el.classList.remove('tile--overview');
    el.style.left = '';
    el.style.top = '';
    el.style.width = '';
    el.style.height = '';
    el.style.zIndex = '';
    el.style.gridColumn = '';
    el.style.gridRow = '';
  }
  // Re-tile: re-applies the layout and hides the other workspaces' windows.
  refreshTiles();
}

export function openOverview() {
  if (open) return;
  // Nothing to lay out: an empty dimmed desktop is not a useful overview.
  if (tiles().length === 0) return;
  // A real-fullscreen window owns its workspace; the overview cannot represent
  // that state honestly, so leave fullscreen first and show the plain desktop.
  if (fullscreenWindow()) exitWindowFullscreen();
  open = true;
  // The grid is for picking a window, not using it: drop any focus a window
  // held so typing cannot reach it behind the overview.
  document.activeElement?.blur?.();
  // Any other overlay stands down (the launcher listens for this).
  window.dispatchEvent(new CustomEvent('overlay:open', { detail: { which: 'overview' } }));
  document.body.classList.add('overview-active');
  backdrop?.classList.remove('hidden');
  layout();
  // The map was just resized; let Leaflet notice.
  window.dispatchEvent(new Event('map:resize'));
}

export function closeOverview() {
  if (!open) return;
  open = false;
  restore();
}

export function toggleOverview() {
  if (open) closeOverview();
  else openOverview();
}

function pick(target) {
  const el = target?.closest?.('#tile-grid > .tile[data-plugin]');
  if (!el) return false;
  const name = el.dataset.plugin;
  swallowNextClick = true;
  closeOverview();
  if (name) focusWindow(name);
  return true;
}

export function initOverview() {
  if (grid) return;
  grid = document.getElementById('tile-grid');
  backdrop = document.getElementById('overview-backdrop');
  if (!grid) return;

  // Capture-phase so a click selects the window instead of reaching a button
  // inside it. Pointer, not click, so a drag inside a window cannot start.
  grid.addEventListener('pointerdown', (e) => {
    if (!open) {
      // Clear any stale swallow from a previous pointerdown that never
      // produced a click (a drag, say).
      swallowNextClick = false;
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    pick(e.target);
  }, true);
  grid.addEventListener('click', (e) => {
    if (!open && !swallowNextClick) return;
    swallowNextClick = false;
    e.preventDefault();
    e.stopPropagation();
  }, true);

  // No scrolling a window while picking one.
  grid.addEventListener('wheel', (e) => {
    if (!open) return;
    e.preventDefault();
    e.stopPropagation();
  }, { capture: true, passive: false });

  backdrop?.addEventListener('click', () => closeOverview());

  window.addEventListener('keydown', (e) => {
    if (!open || e.key !== 'Escape') return;
    // The overview is the topmost layer: Escape closes it, not the fullscreen
    // window under it.
    e.preventDefault();
    e.stopImmediatePropagation();
    closeOverview();
  }, true);

  // Another overlay opening (the launcher) closes this one.
  window.addEventListener('overlay:open', (e) => {
    if (open && e.detail?.which !== 'overview') closeOverview();
  });

  // A resize while open re-flows the grid.
  window.addEventListener('resize', () => {
    if (open) layout();
  });
}
