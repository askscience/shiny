/**
 * overview.js — the window overview (three-finger swipe up; the phone's
 * window switcher).
 *
 * Every open window, from every workspace, as a grid of cards over a dimmed
 * backdrop. The cards are the real tiles — nothing is cloned and no iframe is
 * torn down, so a running video or a half-typed document survives the trip.
 * A tap on a card goes to that window (switching to its workspace); its ✕
 * closes it; a tap outside, Escape or the gesture back closes the overview.
 *
 * The overview is one progress value, 0 = the desktop and 1 = open. A swipe
 * drives it with the fingers (gestures.js → `overviewTransition`), a discrete
 * open/close animates the same path. At `beginOverview()` every tile is
 * measured once and pinned (absolutely positioned at its current rect, so any
 * layout mode works); each frame then interpolates a transform from that rect
 * to the tile's card (FLIP). How a card travels follows the user's motion
 * language (`<html data-motion>`, overviewGrid.js `MOTION`): kinetic arrives
 * fast in a cascade, fluid on a spring, depth out of a blur. Windows of other
 * workspaces come in from the side their workspace lives on. Closing animates
 * back and restores each tile's inline style exactly as it was.
 *
 * The grid scrolls (wheel, or a drag on a touch screen) when the cards do not
 * fit. Themes style the rest from `--overview-p` (0..1, every frame) and
 * `html[data-overview="opening|open|closing"]`.
 */
import { focusWindow, getWorkspacesList, activeWorkspaceIndex, togglePluginFocus, isPluginFocused } from './desktop.js';
import { refreshTiles, showPhoneWindow, closePluginWindow } from './tiles.js';
import { PHONE_QUERY } from './viewport.js';
import { fullscreenWindow, exitWindowFullscreen } from './fullscreen.js';
import { iconButton } from '../ui/components/button.js';
import { createTransition, clamp01 } from './gestureTransition.js';
import { gridLayout, motionFor, blurFor, LABEL_H, LABEL_H_PHONE } from './overviewGrid.js';
import { pluginIconEl } from './pluginIcon.js';
import { t } from './i18n.js';

/** Pointer travel (px) that turns a tap into a scroll. */
const DRAG_PX = 8;

let grid = null;
let backdrop = null;
let chrome = null;
let open = false;
/** The pinned tiles and their cards while the overview is on screen. */
let stage = null;
/** The current openness, 0..1. */
let progress = 0;
/** The discrete open/close in flight, if any. */
let running = null;
let swallowNextClick = false;
/** A pointer down on the grid: a tap picks, a drag scrolls. */
let press = null;

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

function motionName() {
  return document.documentElement.dataset.motion || 'kinetic';
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

/* ── Cards ────────────────────────────────────────────────────── */

/** Fit a `w`×`h` window into a box, keeping its aspect. */
const fit = (w, h, boxW, boxH) => Math.min(boxW / Math.max(1, w), boxH / Math.max(1, h));

function computeTargets() {
  if (!stage) return;
  const { W, H } = stage;
  const layout = gridLayout(stage.items.length, W, H, { phone: PHONE_QUERY.matches });
  stage.maxScroll = layout.maxScroll;
  stage.scroll = Math.min(stage.scroll, stage.maxScroll);
  stage.targets = stage.items.map((it, i) => {
    const card = layout.cards[i];
    const s = fit(it.src.w, it.src.h, card.w, card.h);
    const w = it.src.w * s;
    const h = it.src.h * s;
    // On the card's bottom edge, so every label sits right under its window.
    return {
      x: card.x + (card.w - w) / 2,
      y: card.y + card.h - h,
      s,
      label: { x: card.x + (card.w - w) / 2, y: card.y + card.h, w },
    };
  });
}

/* ── Chrome: a label (and the window's buttons) under each card ── */

/** The label line's height: taller on a phone, for finger-sized buttons. */
const labelHeight = () => (PHONE_QUERY.matches ? LABEL_H_PHONE : LABEL_H);

function buildChrome() {
  if (!chrome) {
    chrome = document.createElement('div');
    chrome.className = 'overview-chrome';
    grid.after(chrome);
  }
  chrome.textContent = '';
  const spaces = getWorkspacesList().length;
  stage.items.forEach((it) => {
    const label = document.createElement('div');
    label.className = 'overview-label';
    label.style.height = `${labelHeight()}px`;
    const title = windowTitle(it.el);
    const plugin = it.el.dataset.plugin;
    // The app's icon: a window without its title bar (a phone) is still
    // told apart at a glance.
    if (plugin) {
      try {
        const glyph = pluginIconEl(plugin, { size: 16 });
        glyph.classList.add('overview-label-icon');
        label.appendChild(glyph);
      } catch (_) { /* no icon for a core window */ }
    }
    const name = document.createElement('span');
    name.className = 'overview-label-name';
    name.textContent = title;
    label.appendChild(name);
    // Which workspace, only where there is more than one to tell apart.
    if (spaces > 1 && it.ws >= 0 && !PHONE_QUERY.matches) {
      const ws = document.createElement('span');
      ws.className = 'overview-label-ws';
      ws.textContent = String(it.ws + 1);
      label.appendChild(ws);
    }
    // The window's own buttons live here too — on a phone, where the title
    // bar is gone, this is where they live at all: round, on a plate, for a
    // finger. Focus (the eye) narrows the AI to this window (desktop.js).
    if (plugin) {
      const focus = iconButton({
        icon: 'ui/eye',
        size: 'sm',
        iconSize: 14,
        label: t('desktop.focusWindow', { name: title }, `Focus ${title}`),
        className: 'overview-label-btn overview-label-focus',
        onClick: (e) => {
          e.stopPropagation();
          togglePluginFocus(plugin);
          const on = isPluginFocused(plugin);
          focus.classList.toggle('is-active', on);
          focus.setAttribute('aria-pressed', String(on));
          const name = on ? t('desktop.unfocusWindow', { name: title }, `Unfocus ${title}`)
            : t('desktop.focusWindow', { name: title }, `Focus ${title}`);
          focus.title = name;
          focus.setAttribute('aria-label', name);
        },
      });
      const on = isPluginFocused(plugin);
      focus.classList.toggle('is-active', on);
      focus.setAttribute('aria-pressed', String(on));
      if (on) {
        const name = t('desktop.unfocusWindow', { name: title }, `Unfocus ${title}`);
        focus.title = name;
        focus.setAttribute('aria-label', name);
      }
      label.appendChild(focus);
    }
    const close = iconButton({
      icon: 'ui/close',
      size: 'sm',
      iconSize: 14,
      label: t('desktop.closeWindow', { name: title }, `Close ${title}`),
      className: 'overview-label-btn overview-label-close',
      onClick: (e) => { e.stopPropagation(); closeCard(it); },
    });
    label.appendChild(close);
    chrome.appendChild(label);
    it.label = label;
  });
  chrome.classList.remove('hidden');
}

/** Close one window from the overview; the others close ranks. */
function closeCard(it) {
  if (!stage || !open) return;
  const name = it.el.dataset.plugin;
  stage.items = stage.items.filter((x) => x !== it);
  it.label?.remove();
  it.el.classList.remove('tile--overview');
  it.el.style.cssText = it.saved;
  it.el.classList.add('hidden');
  if (name) closePluginWindow(name);
  if (stage.items.length === 0) {
    closeOverview();
    return;
  }
  computeTargets();
  render();
}

/** Scroll the grid by `dy` px (clamped). */
function scrollBy(dy) {
  if (!stage || !stage.maxScroll) return;
  const next = Math.min(stage.maxScroll, Math.max(0, stage.scroll + dy));
  if (next === stage.scroll) return;
  stage.scroll = next;
  render();
}

/* ── Rendering ────────────────────────────────────────────────── */

/** Paint the overview at the current progress. */
function render() {
  if (!stage) return;
  const n = stage.items.length;
  const motion = motionFor(stage.motion);
  // The cards move in a short cascade, the first one first.
  const stagger = n > 1 ? Math.min(motion.stagger, 0.36 / (n - 1)) : 0;
  const span = 1 - (n - 1) * stagger;
  const p = progress;
  const ox = stage.box.left;
  const oy = stage.box.top - stage.scroll;
  stage.items.forEach((it, i) => {
    const target = stage.targets[i];
    const q = clamp01((p - i * stagger) / span);
    // Outside 0..1 (a rubber band) the whole stage stretches a little.
    const e = p > 1 ? 1 + (p - 1) : p < 0 ? p : motion.ease(q);
    const away = it.side * stage.W * (1 - clamp01(e));
    const tx = (target.x - it.src.x) * e + away;
    const ty = (target.y - stage.scroll - it.src.y) * e;
    // A window that was not on screen grows in from a little smaller.
    const from = it.fade ? target.s * 0.85 : 1;
    const scale = from + (target.s - from) * e;
    it.el.style.transform = `translate3d(${tx.toFixed(2)}px, ${ty.toFixed(2)}px, 0) scale(${scale.toFixed(4)})`;
    if (it.fade) it.el.style.opacity = clamp01(e).toFixed(3);
    const blur = blurFor(stage.motion, e);
    it.el.style.filter = blur ? `blur(${blur}px)` : '';
    if (it.label) {
      const lt = target.label;
      const l = clamp01((q - 0.6) / 0.4);
      it.label.style.left = `${Math.round(ox + lt.x)}px`;
      it.label.style.top = `${Math.round(oy + lt.y)}px`;
      it.label.style.width = `${Math.round(lt.w)}px`;
      it.label.style.opacity = l.toFixed(3);
      it.label.style.transform = l < 1 ? `translateY(${((1 - l) * 6).toFixed(1)}px)` : '';
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
    motion: motionName(),
    scroll: 0,
    maxScroll: 0,
    targets: [],
  };
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
    el.style.zIndex = String(10 + i);
    el.style.setProperty('--overview-i', String(i));
  });

  setPhase('opening');
  document.body.classList.add('overview-active');
  backdrop?.classList.remove('hidden');
  buildChrome();
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
  stage = null;
  press = null;
  progress = 0;
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
  root.style.removeProperty('--overview-p');
  // Re-tile: re-applies the layout and hides the other workspaces' windows.
  refreshTiles();
  window.dispatchEvent(new Event('map:resize'));
  window.dispatchEvent(new Event('overview:closed'));
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
    if (!name) return;
    // A phone shows one window at a time: picking one is switching to it.
    if (PHONE_QUERY.matches) showPhoneWindow(name);
    focusWindow(name);
  });
  return true;
}

export function initOverview() {
  if (grid) return;
  grid = document.getElementById('tile-grid');
  backdrop = document.getElementById('overview-backdrop');
  if (!grid) return;

  // Capture-phase so a tap selects the window instead of reaching a button
  // inside it. A tap picks on release; a drag scrolls a grid that overflows.
  grid.addEventListener('pointerdown', (e) => {
    if (!stage) {
      // Clear any stale swallow from a previous pointerdown that never
      // produced a click (a drag, say).
      swallowNextClick = false;
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (!open || e.button > 0) return;
    press = { id: e.pointerId, y: e.clientY, lastY: e.clientY, target: e.target, dragging: false };
    try { grid.setPointerCapture(e.pointerId); } catch (_) { /* synthetic events */ }
  }, true);
  grid.addEventListener('pointermove', (e) => {
    if (!stage) return;
    e.stopPropagation();
    if (!press || e.pointerId !== press.id) return;
    if (!press.dragging && Math.abs(e.clientY - press.y) > DRAG_PX) press.dragging = true;
    if (press.dragging) {
      scrollBy(press.lastY - e.clientY);
      press.lastY = e.clientY;
    }
  }, true);
  grid.addEventListener('pointerup', (e) => {
    if (!stage) return;
    e.preventDefault();
    e.stopPropagation();
    if (!press || e.pointerId !== press.id) return;
    const { dragging, target } = press;
    press = null;
    if (dragging || !open) return;
    // Outside every window is the backdrop: leave.
    if (!pick(target)) closeOverview();
  }, true);
  grid.addEventListener('pointercancel', () => { press = null; }, true);
  grid.addEventListener('click', (e) => {
    if (!stage && !swallowNextClick) return;
    swallowNextClick = false;
    e.preventDefault();
    e.stopPropagation();
  }, true);

  // No scrolling a window while picking one: the wheel scrolls the grid.
  const onWheel = (e) => {
    if (!stage) return;
    e.preventDefault();
    e.stopPropagation();
    if (open) scrollBy(e.deltaY);
  };
  grid.addEventListener('wheel', onWheel, { capture: true, passive: false });
  backdrop?.addEventListener('wheel', onWheel, { passive: false });

  backdrop?.addEventListener('click', () => closeOverview());

  window.addEventListener('keydown', (e) => {
    if (!open) return;
    if (e.key === 'Escape') {
      // The overview is the topmost layer: Escape closes it, not the
      // fullscreen window under it.
      e.preventDefault();
      e.stopImmediatePropagation();
      closeOverview();
    }
  }, true);

  // Another overlay opening (the launcher) closes this one.
  window.addEventListener('overlay:open', (e) => {
    if (stage && e.detail?.which !== 'overview') closeOverview();
  });

  // A resize while open re-flows the cards; a motion change applies to the
  // next trip.
  const reflow = () => {
    if (!stage) return;
    stage.W = grid.clientWidth || window.innerWidth;
    stage.H = grid.clientHeight || window.innerHeight;
    stage.box = grid.getBoundingClientRect();
    computeTargets();
    render();
  };
  window.addEventListener('resize', reflow);
  window.addEventListener('theme:change', reflow);
}
