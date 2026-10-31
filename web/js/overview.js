/**
 * overview.js — the window overview (three-finger swipe up).
 *
 * Mission-Control for the desktop: every open plugin window, from every
 * workspace, over a dimmed backdrop. The windows are the real tiles — nothing
 * is cloned and no iframe is torn down, so a running video or a half-typed
 * document survives the trip. Clicking a window focuses it (switching to its
 * workspace) and closes the overview.
 *
 * The overview is one progress value, 0 = the desktop and 1 = open. A swipe
 * drives it with the fingers (gestures.js → `overviewTransition`), a discrete
 * open/close animates the same path. At `beginOverview()` every tile is
 * measured once and pinned (absolutely positioned at its current rect, so any
 * layout mode works); each frame then interpolates a transform from that rect
 * to the tile's overview target (FLIP). Windows of other workspaces fly in
 * from the side their workspace lives on. Closing animates back and restores
 * each tile's inline style exactly as it was.
 *
 * The active theme picks the arrangement (`overview` in theme.json):
 *
 *   strip — a horizontal row of equal boxes, windows on a common baseline,
 *           a label under each and Prev / Next (or ← / →) to scroll it;
 *   cloud — windows scattered at different depths around a centred
 *           headline, drifting gently while open.
 *
 * Themes style the rest from `--overview-p` (0..1, every frame) and
 * `html[data-overview="opening|open|closing"]` / `[data-overview-style]` on
 * the document element. Escape or a click outside the windows closes.
 */
import { focusWindow, getWorkspacesList, activeWorkspaceIndex } from './desktop.js';
import { refreshTiles } from './tiles.js';
import { fullscreenWindow, exitWindowFullscreen } from './fullscreen.js';
import { getThemeManifest } from '../ui/theme-loader.js';
import { controlButton } from '../ui/components/button.js';
import { createTransition, animateValue, clamp01 } from './gestureTransition.js';
import { t, tp } from './i18n.js';

const STYLES = new Set(['strip', 'cloud']);
const STRIP_GAP = 26;
const LABEL_GAP = 14;

let grid = null;
let backdrop = null;
let chrome = null;
let open = false;
/** The pinned tiles and their targets while the overview is on screen. */
let stage = null;
/** The current openness, 0..1. */
let progress = 0;
/** The discrete open/close in flight, if any. */
let running = null;
let swallowNextClick = false;

export function isOverviewOpen() {
  return open;
}

/** True from `beginOverview()` until the overview is fully closed again. */
export function isOverviewActive() {
  return !!stage;
}

/** Every mounted plugin window, active workspace or not. */
function tiles() {
  return grid ? [...grid.querySelectorAll(':scope > .tile[data-plugin]')] : [];
}

function overviewStyle() {
  const style = getThemeManifest()?.overview;
  return STYLES.has(style) ? style : 'strip';
}

function setPhase(phase) {
  const root = document.documentElement;
  if (phase) root.dataset.overview = phase;
  else delete root.dataset.overview;
}

function windowTitle(el) {
  const text = el.querySelector(':scope > .tile-header .tile-header-title')?.textContent?.trim();
  if (text) return text;
  const name = el.dataset.plugin || '';
  return name.charAt(0).toUpperCase() + name.slice(1);
}

function wsLabel(index) {
  return index >= 0
    ? t('desktop.overviewWs', { n: index + 1 }, `WS ${index + 1}`)
    : t('desktop.overviewWsNone', null, 'WS');
}

/** A "WS n" chip in each window's own header (the cloud has no labels). */
function addBadge(el, index) {
  const header = el.querySelector(':scope > .tile-header');
  if (!header) return;
  const badge = document.createElement('span');
  badge.className = 'overview-badge';
  badge.textContent = wsLabel(index);
  badge.dataset.overview = 'badge';
  header.appendChild(badge);
}

function clearBadges() {
  grid?.querySelectorAll('.overview-badge').forEach((b) => b.remove());
}

/* ── Targets ──────────────────────────────────────────────────── */

/** Fit a `w`×`h` window into a box, keeping its aspect. */
const fit = (w, h, boxW, boxH) => Math.min(boxW / Math.max(1, w), boxH / Math.max(1, h));

function stripGeometry(W, H, n) {
  const pad = Math.max(24, Math.round(W * 0.053));
  let boxH = Math.round(H * 0.36);
  let boxW = Math.round(boxH * 1.3);
  if (boxW > W - pad * 2) {
    boxW = W - pad * 2;
    boxH = Math.round(boxW / 1.3);
  }
  const top = Math.round(H * 0.12);
  const visible = Math.max(1, Math.floor((W - pad * 2 + STRIP_GAP) / (boxW + STRIP_GAP)));
  return { pad, boxW, boxH, top, visible, max: Math.max(0, n - visible) };
}

function stripTargets(items, W, H, strip) {
  const g = stripGeometry(W, H, items.length);
  return items.map((it, i) => {
    const s = fit(it.src.w, it.src.h, g.boxW, g.boxH);
    const boxX = g.pad + (i - strip) * (g.boxW + STRIP_GAP);
    return {
      x: boxX + (g.boxW - it.src.w * s) / 2,
      y: g.top + g.boxH - it.src.h * s,
      s, z: 0, ry: 0,
      label: { x: boxX, y: g.top + g.boxH + LABEL_GAP, w: g.boxW },
    };
  });
}

/** Depths (px towards the viewer) and a little turn, cycled over the windows. */
const CLOUD_DEPTH = [-60, 90, -160, 40, 120, -240, 10, -110];

function cloudTargets(items, W, H) {
  const n = items.length;
  const boxW = Math.min(W, H * 1.6) * (n <= 3 ? 0.34 : n <= 6 ? 0.28 : 0.22);
  const boxH = boxW * 0.7;
  const rx = W * 0.36;
  const ry = H * 0.34;
  return items.map((it, i) => {
    // Around the centred headline, starting top-left; a little irregular so
    // it reads as scattered rather than as a clock face.
    const a = -Math.PI * 0.75 + (i / n) * Math.PI * 2 + Math.sin(i * 2.4) * 0.18;
    const s = fit(it.src.w, it.src.h, boxW, boxH);
    const w = it.src.w * s;
    const h = it.src.h * s;
    const cx = W / 2 + Math.cos(a) * rx;
    const cy = H / 2 + Math.sin(a) * ry;
    const x = Math.min(Math.max(0, cx - w / 2), Math.max(0, W - w));
    const y = Math.min(Math.max(0, cy - h / 2), Math.max(0, H - h));
    return { x, y, s, z: CLOUD_DEPTH[i % CLOUD_DEPTH.length], ry: -Math.cos(a) * 8, label: null };
  });
}

function computeTargets() {
  if (!stage) return;
  const { W, H } = stage;
  stage.targets = stage.style === 'cloud'
    ? cloudTargets(stage.items, W, H)
    : stripTargets(stage.items, W, H, stage.strip);
}

/* ── Chrome: labels, headline, strip navigation ───────────────── */

function buildChrome() {
  if (!chrome) {
    chrome = document.createElement('div');
    chrome.className = 'overview-chrome';
    chrome.setAttribute('aria-hidden', 'true');
    grid.after(chrome);
  }
  chrome.textContent = '';
  chrome.dataset.style = stage.style;

  const labels = document.createElement('div');
  labels.className = 'overview-labels';
  stage.items.forEach((it, i) => {
    const label = document.createElement('div');
    label.className = 'overview-label';
    const index = document.createElement('span');
    index.className = 'overview-label-index';
    index.textContent = String(i + 1).padStart(2, '0');
    const name = document.createElement('span');
    name.className = 'overview-label-name';
    name.textContent = windowTitle(it.el);
    const ws = document.createElement('span');
    ws.className = 'overview-label-ws';
    ws.textContent = wsLabel(it.ws);
    label.append(index, name, ws);
    labels.appendChild(label);
    it.label = label;
  });

  const title = document.createElement('div');
  title.className = 'overview-title';
  const spaces = Math.max(1, getWorkspacesList().length);
  [
    tp('desktop.overviewWindows', stage.items.length, null, `${stage.items.length} windows,`),
    tp('desktop.overviewSpaces', spaces, null, `${spaces} spaces.`),
  ].forEach((text, i) => {
    const line = document.createElement('span');
    line.className = 'overview-title-line';
    line.style.setProperty('--i', String(i));
    const inner = document.createElement('span');
    inner.textContent = text;
    line.appendChild(inner);
    title.appendChild(line);
  });

  chrome.append(labels, title);

  if (stage.style === 'strip') {
    const nav = document.createElement('div');
    nav.className = 'overview-nav';
    const count = document.createElement('span');
    count.className = 'overview-nav-count';
    const prev = controlButton();
    prev.className = 'overview-nav-btn';
    prev.dataset.dir = '-1';
    prev.textContent = `← ${t('desktop.overviewPrev', null, 'Prev')}`;
    const next = controlButton();
    next.className = 'overview-nav-btn';
    next.dataset.dir = '1';
    next.textContent = `${t('desktop.overviewNext', null, 'Next')} →`;
    nav.append(count, prev, next);
    nav.addEventListener('click', (e) => {
      const btn = e.target.closest?.('.overview-nav-btn');
      if (btn) scrollStrip(Number(btn.dataset.dir));
    });
    chrome.appendChild(nav);
    stage.nav = { el: nav, count, prev, next };
  }
  chrome.classList.remove('hidden');
}

function syncNav() {
  const nav = stage?.nav;
  if (!nav) return;
  const { max } = stripGeometry(stage.W, stage.H, stage.items.length);
  const at = Math.round(stage.stripTo);
  nav.el.classList.toggle('hidden', max === 0);
  nav.count.textContent = `${String(at + 1).padStart(2, '0')} — ${String(stage.items.length).padStart(2, '0')}`;
  nav.prev.disabled = at <= 0;
  nav.next.disabled = at >= max;
}

/** Scroll the strip by whole windows (Prev / Next, ← / →). */
function scrollStrip(dir) {
  if (!stage || stage.style !== 'strip' || !dir) return;
  const { max } = stripGeometry(stage.W, stage.H, stage.items.length);
  const to = Math.min(max, Math.max(0, Math.round(stage.stripTo) + Math.sign(dir)));
  if (to === stage.stripTo) return;
  stage.stripTo = to;
  stage.stopStrip?.();
  stage.stopStrip = animateValue({
    from: stage.strip,
    to,
    duration: 420,
    onFrame: (v) => {
      if (!stage) return;
      stage.strip = v;
      computeTargets();
      render();
    },
  });
  syncNav();
}

/* ── Rendering ────────────────────────────────────────────────── */

const easeOut = (k) => 1 - (1 - k) ** 3;

/** Paint the overview at the current progress. */
function render() {
  if (!stage) return;
  const n = stage.items.length;
  // The windows leave in a short cascade, the first one first.
  const stagger = n > 1 ? Math.min(0.06, 0.36 / (n - 1)) : 0;
  const span = 1 - (n - 1) * stagger;
  const p = progress;
  const ox = stage.box.left;
  const oy = stage.box.top;
  stage.items.forEach((it, i) => {
    const target = stage.targets[i];
    const q = clamp01((p - i * stagger) / span);
    // Outside 0..1 (a rubber band) the whole stage stretches a little.
    const e = p > 1 ? 1 + (p - 1) : p < 0 ? p : easeOut(q);
    const away = it.side * stage.W * (1 - clamp01(e));
    const tx = (target.x - it.src.x) * e + away;
    const ty = (target.y - it.src.y) * e;
    const scale = 1 + (target.s - 1) * e;
    it.el.style.transform = `translate3d(${tx.toFixed(2)}px, ${ty.toFixed(2)}px, ${(target.z * e).toFixed(1)}px)`
      + ` rotateY(${(target.ry * e).toFixed(2)}deg) scale(${scale.toFixed(4)})`;
    if (it.fade) it.el.style.opacity = clamp01(e).toFixed(3);
    if (it.label) {
      const lt = target.label;
      if (lt) {
        it.label.style.left = `${Math.round(ox + lt.x)}px`;
        it.label.style.top = `${Math.round(oy + lt.y)}px`;
        it.label.style.width = `${Math.round(lt.w)}px`;
      }
      it.label.style.opacity = lt ? clamp01((q - 0.55) / 0.45).toFixed(3) : '0';
    }
  });
  document.documentElement.style.setProperty('--overview-p', clamp01(p).toFixed(4));
}

/* ── Lifecycle ────────────────────────────────────────────────── */

/**
 * Put the overview on stage. From closed: measure every tile once, pin it at
 * its rect and compute the targets, at progress 0. From open: mark it as
 * closing. Returns false when there is nothing to show.
 */
export function beginOverview() {
  if (!grid) return false;
  if (stage) {
    setPhase(open ? 'closing' : 'opening');
    return true;
  }
  if (tiles().length === 0) return false;
  // A real-fullscreen window owns its workspace; the overview cannot represent
  // that state honestly, so leave fullscreen first and show the plain desktop.
  if (fullscreenWindow()) exitWindowFullscreen();
  const list = tiles();
  if (list.length === 0) return false;

  // The grid is for picking a window, not using it: drop any focus a window
  // held so typing cannot reach it behind the overview.
  document.activeElement?.blur?.();
  // Any other overlay stands down (the launcher listens for this).
  window.dispatchEvent(new CustomEvent('overlay:open', { detail: { which: 'overview' } }));

  const workspaces = getWorkspacesList();
  const active = activeWorkspaceIndex();
  const wsOf = (name) => workspaces.findIndex((ws) => (ws.windows || []).includes(name));
  const W = grid.clientWidth || window.innerWidth;
  const H = grid.clientHeight || window.innerHeight;

  // Measure everything before pinning anything: the first absolutely
  // positioned tile would otherwise re-flow the rest of a tiled layout.
  const items = list.map((el, order) => {
    const ws = wsOf(el.dataset.plugin);
    const shown = !el.classList.contains('hidden');
    const src = shown
      ? { x: el.offsetLeft, y: el.offsetTop, w: el.offsetWidth, h: el.offsetHeight }
      : { x: W * 0.2, y: H * 0.2, w: W * 0.6, h: H * 0.6 };
    return {
      el, ws, order, src,
      // Windows of another workspace come in from the side it lives on; a
      // hidden window of this one (the phone's other windows) fades in.
      side: shown || ws < 0 || ws === active ? 0 : Math.sign(ws - active),
      fade: !shown && (ws < 0 || ws === active),
      saved: el.style.cssText,
      label: null,
    };
  });
  // Numbered by workspace, then by order within it.
  items.sort((a, b) => ((a.ws < 0 ? 1e3 : a.ws) - (b.ws < 0 ? 1e3 : b.ws))
    || (workspaces[a.ws]?.windows || []).indexOf(a.el.dataset.plugin)
      - (workspaces[b.ws]?.windows || []).indexOf(b.el.dataset.plugin)
    || a.order - b.order);

  stage = {
    items, W, H,
    box: grid.getBoundingClientRect(),
    style: overviewStyle(),
    strip: 0,
    stripTo: 0,
    targets: [],
    nav: null,
    stopStrip: null,
  };
  // Start the strip on the first window of the active workspace.
  if (stage.style === 'strip') {
    const first = items.findIndex((it) => it.ws === active);
    const { max } = stripGeometry(W, H, items.length);
    stage.strip = stage.stripTo = Math.min(max, Math.max(0, first));
  }
  computeTargets();

  items.forEach((it, i) => {
    const { el, src } = it;
    el.classList.remove('hidden');
    el.classList.add('tile--overview');
    // The overview owns position: drop the tiling/fullscreen placement.
    el.style.gridColumn = '';
    el.style.gridRow = '';
    el.style.left = `${src.x}px`;
    el.style.top = `${src.y}px`;
    el.style.width = `${src.w}px`;
    el.style.height = `${src.h}px`;
    el.style.zIndex = String(10 + (stage.style === 'cloud' ? stage.targets[i].z + 300 : i));
    el.style.setProperty('--overview-i', String(i));
    if (stage.style === 'cloud') addBadge(el, it.ws);
  });

  const root = document.documentElement;
  root.dataset.overviewStyle = stage.style;
  setPhase('opening');
  document.body.classList.add('overview-active');
  backdrop?.classList.remove('hidden');
  buildChrome();
  syncNav();
  progress = 0;
  render();
  return true;
}

/** Render the overview at `p` (0 = desktop, 1 = open). */
export function setOverviewProgress(p) {
  if (!stage) return;
  progress = Number(p) || 0;
  render();
}

/**
 * Settle the overview: `true` leaves it open, `false` restores the desktop
 * (every tile's inline style exactly as it was) and re-tiles.
 */
export function endOverview(openAtEnd) {
  if (!stage) return;
  if (openAtEnd) {
    progress = 1;
    render();
    open = true;
    setPhase('open');
    // The map was just resized; let Leaflet notice.
    window.dispatchEvent(new Event('map:resize'));
    return;
  }
  open = false;
  const items = stage.items;
  stage.stopStrip?.();
  stage = null;
  progress = 0;
  clearBadges();
  chrome?.classList.add('hidden');
  if (chrome) chrome.textContent = '';
  for (const it of items) {
    it.el.classList.remove('tile--overview');
    it.el.style.cssText = it.saved;
  }
  document.body.classList.remove('overview-active');
  backdrop?.classList.add('hidden');
  const root = document.documentElement;
  setPhase(null);
  delete root.dataset.overviewStyle;
  root.style.removeProperty('--overview-p');
  // Re-tile: re-applies the layout and hides the other workspaces' windows.
  refreshTiles();
  window.dispatchEvent(new Event('map:resize'));
}

/**
 * An interactive open (`wantOpen`) or close of the overview, for a gesture to
 * drive or a discrete caller to `play()`. Its progress is the completion of
 * that change (so 0 is "as it was"). `onSettled(isOpen)` runs once it has
 * landed. Null when there is nothing to do.
 */
export function overviewTransition(wantOpen, onSettled = null) {
  running?.finishNow(true);
  if (wantOpen ? open : !stage) return null;
  if (!beginOverview()) return null;
  setPhase(wantOpen ? 'opening' : 'closing');
  const transition = createTransition({
    apply: (k) => setOverviewProgress(wantOpen ? k : 1 - k),
    finish: (committed) => {
      if (running === transition) running = null;
      const isOpen = committed ? wantOpen : !wantOpen;
      endOverview(isOpen);
      onSettled?.(isOpen);
    },
  });
  running = transition;
  return transition;
}

export function openOverview() {
  overviewTransition(true)?.play();
}

/** Animate the overview closed; `then` runs once the desktop is back. */
export function closeOverview(then = null) {
  const transition = overviewTransition(false, then ? () => then() : null);
  if (transition) transition.play();
  else then?.();
}

function pick(target) {
  const el = target?.closest?.('#tile-grid > .tile[data-plugin]');
  if (!el) return false;
  const name = el.dataset.plugin;
  swallowNextClick = true;
  // Back to the desktop first, then to the window — on another workspace that
  // is a workspace slide, so the window is seen arriving where it lives.
  closeOverview(() => {
    if (name) focusWindow(name);
  });
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
    if (!stage) {
      // Clear any stale swallow from a previous pointerdown that never
      // produced a click (a drag, say).
      swallowNextClick = false;
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (!open) return;
    // Outside every window is the backdrop: leave.
    if (!pick(e.target)) closeOverview();
  }, true);
  grid.addEventListener('click', (e) => {
    if (!stage && !swallowNextClick) return;
    swallowNextClick = false;
    e.preventDefault();
    e.stopPropagation();
  }, true);

  // No scrolling a window while picking one.
  grid.addEventListener('wheel', (e) => {
    if (!stage) return;
    e.preventDefault();
    e.stopPropagation();
  }, { capture: true, passive: false });

  backdrop?.addEventListener('click', () => closeOverview());

  window.addEventListener('keydown', (e) => {
    if (!open) return;
    if (e.key === 'Escape') {
      // The overview is the topmost layer: Escape closes it, not the
      // fullscreen window under it.
      e.preventDefault();
      e.stopImmediatePropagation();
      closeOverview();
      return;
    }
    if ((e.key === 'ArrowLeft' || e.key === 'ArrowRight') && stage?.style === 'strip'
        && !e.altKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      e.stopImmediatePropagation();
      scrollStrip(e.key === 'ArrowLeft' ? -1 : 1);
    }
  }, true);

  // Another overlay opening (the launcher) closes this one.
  window.addEventListener('overlay:open', (e) => {
    if (stage && e.detail?.which !== 'overview') closeOverview();
  });

  // A resize or theme change while open re-flows the targets.
  const reflow = () => {
    if (!stage) return;
    stage.W = grid.clientWidth || window.innerWidth;
    stage.H = grid.clientHeight || window.innerHeight;
    stage.box = grid.getBoundingClientRect();
    const style = overviewStyle();
    if (style !== stage.style && open) {
      // A different arrangement needs its own chrome: rebuild from scratch.
      endOverview(false);
      if (beginOverview()) endOverview(true);
      return;
    }
    computeTargets();
    syncNav();
    render();
  };
  window.addEventListener('resize', reflow);
  window.addEventListener('theme:change', reflow);
}
