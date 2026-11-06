import {
  confirmDialog, dial, toolbar, element as el, toolButton, button, toast,
  emptyState, eyebrow, metaLine, listReveal, cardEnter, sheet, segmented,
} from '../../../ui/index.js';

/**
 * editor.js — the Image window: a minimal photo editor.
 *
 * Layout: one window bar (menu, a display-type title over a mono meta line —
 * "4032×3024 · 63% · LOOK: NOIR" — and a fit / 1:1 toggle), a borderless
 * canvas that fills the window, and a flat bottom dock on a hairline with the
 * looks (grey square previews of every effect applied to the open image, the
 * name in mono below) and the adjustments, each under a mono eyebrow.
 * Every pixel change goes through the plugin's Rust engine; this module only
 * builds DOM, tracks state and paints returned RGBA.
 *
 * On a phone (`html[data-phone]`) the canvas keeps the whole screen: the dock
 * body moves into a bottom sheet opened from two icons in the window bar
 * (Looks, Adjust — one sheet, a segmented switch between the two sections)
 * and comes back to the inline dock when the screen is a desktop again.
 *
 * All window styling lives in the core stylesheet (`web/css/tiles.css`, the
 * `image-*` block) — this file ships no CSS, per the plugin UI contract.
 *
 * NOTE: `../` paths point at the app root (this module lives three levels deep,
 * `plugins/image/web/`).
 */

import { openContextMenu } from '../../../js/contextMenu.js';
import { saveOrDownload, pickFiles, onOpenFromFiles } from '../../../js/files.js';
import { t } from '../../../js/i18n.js';
import { PHONE_QUERY } from '../../../js/viewport.js';
import * as api from './api.js';

export const PLUGIN_NAME = 'image';

/* ── Static tables ──────────────────────────────────────────────── */

/**
 * The dock dials. Values are stored per document (`images.adjust`) and the
 * picture renders as look(settings(bytes)) — dials stay where you put them.
 * Units mirror `DIAL_ORDER`/`dial_op` in `src/doc.rs`.
 */
const ADJUST_DIALS = [
  { key: 'brightness', label: 'Brightness', labelKey: 'plugin.image.dial.brightness', min: -100, max: 100, step: 1 },
  { key: 'contrast', label: 'Contrast', labelKey: 'plugin.image.dial.contrast', min: -100, max: 100, step: 1 },
  { key: 'exposure', label: 'Exposure', labelKey: 'plugin.image.dial.exposure', min: -2, max: 2, step: 0.05 },
  { key: 'vibrance', label: 'Vibrance', labelKey: 'plugin.image.dial.vibrance', min: -100, max: 100, step: 1 },
  { key: 'saturation', label: 'Saturation', labelKey: 'plugin.image.dial.saturation', min: -100, max: 100, step: 1 },
  { key: 'blur', label: 'Blur', labelKey: 'plugin.image.dial.blur', min: 0, max: 20, step: 1 },
  { key: 'sharpen', label: 'Sharpen', labelKey: 'plugin.image.dial.sharpen', min: 0, max: 200, step: 5 },
  { key: 'vignette', label: 'Vignette', labelKey: 'plugin.image.dial.vignette', min: 0, max: 100, step: 1 },
];

const ONE_CLICKS = [
  ['auto_tone', 'plugin.image.oneClick.autoTone', 'Auto tone', 'ui/magic-wand'],
  ['auto_contrast', 'plugin.image.oneClick.autoContrast', 'Auto contrast', 'ui/contrast'],
  ['invert', 'plugin.image.oneClick.invert', 'Invert', 'ui/invert'],
  ['grayscale', 'plugin.image.oneClick.grayscale', 'B&W', 'ui/grayscale'],
  ['sepia', 'plugin.image.oneClick.sepia', 'Sepia', 'ui/sepia'],
];

/** The zoom range of the chip, the wheel, the keys and a pinch. */
const ZOOM_MIN = 0.05;
const ZOOM_MAX = 8;

/* ── State ──────────────────────────────────────────────────────── */

export const S = {
  tile: null,
  els: {},
  docs: [],
  doc: null,
  zoom: 1,
  fit: true,
  render: null, // { w, h, data: Uint8ClampedArray }
  pending: null,
  applying: false,
  drag: null,
  pointers: null, // active pointers on the canvas (a pinch needs two)
  pinch: null,    // { dist, zoom } at the start of a two-finger gesture
  lastTap: null,  // { at, x, y } of the last touch tap (double-tap = fit ↔ 1:1)
  touchToggledAt: 0,
  phone: false,   // the dock body lives in the sheet
  sheet: null,
  looksUrl: '',
  looksRevealed: false, // the filmstrip cascades in once per document
  enterPending: false,  // the next paint is a new image's first
  wired: false,
  resizeObs: null,
};

/* ── Small helpers ──────────────────────────────────────────────── */

export const iconButton = toolButton;

/* ── Workspace skeleton ─────────────────────────────────────────── */

export function buildWorkspace() {
  const tile = el('section', 'tile image-tile');
  tile.dataset.plugin = PLUGIN_NAME;
  S.tile = tile;

  /* Application bar: menu + document title + zoom. */
  const appbar = toolbar([], { className: 'image-appbar' });
  appbar.dataset.windowBar = '';
  const menuBtn = toolButton('ui/list', t('plugin.image.toolbar.menu', null, 'Menu'), (e) => openMainMenu(e.currentTarget));
  const appHead = el('div', 'image-appbar-head');
  const appTitle = el('span', 'image-appbar-title');
  S.els.appTitle = appTitle;
  const appMeta = metaLine([], 'image-appbar-meta');
  S.els.appMeta = appMeta;
  appHead.append(appTitle, appMeta);
  const appRight = el('div', 'image-appbar-right');
  // Fit ↔ actual size; the zoom itself reads in the meta line.
  const zoomChip = el('button', 'image-zoom-chip');
  zoomChip.type = 'button';
  zoomChip.title = t('plugin.image.toolbar.fitToggle', null, 'Toggle fit / actual size');
  zoomChip.addEventListener('click', () => (S.fit ? setZoom(1) : zoomToFit()));
  S.els.zoomChip = zoomChip;
  // Phone only: the dock is a sheet, opened on its Looks or Adjust section.
  const looksBtn = toolButton('ui/palette', t('plugin.image.dock.looks', null, 'Looks'), () => openToolSheet('looks'), { className: 'image-tools-btn image-tools-looks' });
  const adjustBtn = toolButton('ui/brightness', t('plugin.image.dock.adjust', null, 'Adjust'), () => openToolSheet('adjust'), { className: 'image-tools-btn image-tools-adjust' });
  looksBtn.hidden = true;
  adjustBtn.hidden = true;
  S.els.toolButtons = [looksBtn, adjustBtn];
  appRight.append(zoomChip, looksBtn, adjustBtn);
  appbar.append(menuBtn, appHead, appRight);
  tile.appendChild(appbar);

  /* Stage: the borderless canvas fills the window. */
  const area = el('div', 'image-canvas-area');
  const view = el('canvas', 'image-view');
  area.appendChild(view);
  S.els.area = area;
  S.els.view = view;
  wireCanvas(view, area);
  tile.appendChild(area);

  /* Bottom dock: looks + adjustments, each under a mono eyebrow. */
  const dock = el('div', 'image-dock');
  const body = el('div', 'image-dock-body');

  const looksSection = el('div', 'image-dock-section');
  looksSection.dataset.section = 'looks';
  looksSection.appendChild(eyebrow(t('plugin.image.dock.looks', null, 'Looks'), 'image-dock-eyebrow'));
  const lookRow = el('div', 'image-look-row');
  S.els.lookRow = lookRow;
  looksSection.appendChild(lookRow);
  body.appendChild(looksSection);

  body.appendChild(el('div', 'image-dock-divider'));
  const adjustSection = el('div', 'image-dock-section');
  adjustSection.dataset.section = 'adjust';
  adjustSection.appendChild(eyebrow(t('plugin.image.dock.adjust', null, 'Adjust'), 'image-dock-eyebrow'));
  adjustSection.appendChild(buildAdjustBody());
  body.appendChild(adjustSection);

  dock.appendChild(body);
  S.els.dock = dock;
  S.els.dockBody = body;
  tile.appendChild(dock);

  buildToolSheet();
  syncPhoneLayout();

  return tile;
}

/* ── Phone: the dock as a bottom sheet ──────────────────────────── */

function isPhone() {
  return document.documentElement?.dataset?.phone === '1';
}

/**
 * The sheet that holds the dock body on a phone: a segmented switch (Looks /
 * Adjust) over the same DOM the inline dock shows on a desktop — the dials
 * keep their listeners, so they work wherever the body sits.
 */
function buildToolSheet() {
  const wrap = el('div', 'image-sheet');
  wrap.dataset.tab = 'adjust';
  const tabs = segmented({
    label: t('plugin.image.sheet.label', null, 'Looks and adjustments'),
    options: [
      { value: 'looks', label: t('plugin.image.dock.looks', null, 'Looks'), icon: 'ui/palette' },
      { value: 'adjust', label: t('plugin.image.dock.adjust', null, 'Adjust'), icon: 'ui/brightness' },
    ],
    value: 'adjust',
    onChange: (value) => { wrap.dataset.tab = value; },
  });
  tabs.classList.add('image-sheet-tabs');
  wrap.appendChild(tabs);
  S.els.sheetWrap = wrap;
  S.els.sheetTabs = tabs;
  S.sheet = sheet({ label: t('plugin.image.sheet.label', null, 'Looks and adjustments'), body: [wrap] });
}

/** Open the tools sheet on one of its sections (phone only). */
export function openToolSheet(tab = 'adjust') {
  if (!S.sheet || !S.phone) return;
  S.els.sheetTabs?.setValue?.(tab, { silent: true });
  if (S.els.sheetWrap) S.els.sheetWrap.dataset.tab = tab;
  S.sheet.open();
}

/** Move the dock body between the inline dock and the sheet. */
export function syncPhoneLayout() {
  const phone = isPhone();
  const { dock, dockBody, sheetWrap, toolButtons } = S.els;
  if (!dock || !dockBody) return;
  if (phone === S.phone && dockBody.parentElement === (phone ? sheetWrap : dock)) return;
  S.phone = phone;
  if (phone) {
    sheetWrap?.appendChild(dockBody);
    // The sheet joins the document now (closed), so the looks and dials stay
    // in the page — and reachable — while it is not open.
    if (S.sheet?.el && !S.sheet.el.parentElement) document.body.appendChild(S.sheet.el);
    dock.hidden = true;
  } else {
    S.sheet?.close();
    dock.appendChild(dockBody);
    dock.hidden = false;
  }
  for (const b of toolButtons || []) b.hidden = !phone;
  if (S.fit) paintView();
}

let phoneObs = null;
let phoneListening = false;

function watchPhone() {
  if (phoneListening) return;
  phoneListening = true;
  PHONE_QUERY?.addEventListener?.('change', syncPhoneLayout);
  // The shell marks the document; follow the mark itself too.
  if (typeof MutationObserver !== 'undefined' && document.documentElement) {
    phoneObs = new MutationObserver(syncPhoneLayout);
    phoneObs.observe(document.documentElement, { attributes: true, attributeFilter: ['data-phone'] });
  }
}

function unwatchPhone() {
  if (!phoneListening) return;
  phoneListening = false;
  PHONE_QUERY?.removeEventListener?.('change', syncPhoneLayout);
  phoneObs?.disconnect();
  phoneObs = null;
}

/* ── Looks (preview thumbnails of every effect) ─────────────────── */

/** Build one thumbnail button per look; the strip image paints in later. */
export function renderLooks() {
  const row = S.els.lookRow;
  if (!row) return;
  if (S.looksUrl) {
    URL.revokeObjectURL(S.looksUrl);
    S.looksUrl = '';
  }
  row.textContent = '';
  row.style.removeProperty('--look-url');
  S.looksRevealed = false;
  const looks = S.doc?.looks || [];
  if (!S.doc || !looks.length) return;
  looks.forEach((look, i) => {
    // A grey square preview tile (a slice of the strip) with the name below.
    const chip = el('button', 'image-look-chip');
    chip.type = 'button';
    chip.title = look.name;
    chip.setAttribute('aria-label', look.name);
    chip.style.setProperty('--i', String(i));
    chip.append(el('span', 'image-look-thumb'), el('span', 'image-look-name', look.name));
    if (look.key === S.doc.look) chip.classList.add('is-active');
    chip.addEventListener('click', () => {
      // Clicking the selected look clears it; otherwise swap the selection.
      if (S.doc.look === look.key) void applyOp({ op: 'look' });
      else void applyOp({ op: 'look', name: look.key });
    });
    row.appendChild(chip);
  });
  void loadLooksStrip();
}

/** Fetch the strip PNG (every look applied to the open image) and show it. */
async function loadLooksStrip() {
  const row = S.els.lookRow;
  if (!row || !S.doc) return;
  const count = (S.doc.looks || []).length;
  if (!count) return;
  try {
    const blob = await api.fetchLooks(S.doc.image_id);
    if (!blob || !blob.size) return;
    const url = URL.createObjectURL(blob);
    if (S.looksUrl) URL.revokeObjectURL(S.looksUrl);
    S.looksUrl = url;
    row.style.setProperty('--look-n', String(count));
    row.style.setProperty('--look-url', `url("${url}")`);
    // First strip for this document: the previews cascade in (later
    // refreshes after edits just repaint).
    if (!S.looksRevealed) {
      S.looksRevealed = true;
      void listReveal(row.children || []);
    }
  } catch (_) {
    /* previews are best-effort */
  }
}

let looksTimer = null;

/** Refresh the previews after edits (trailing debounce — one request per burst). */
function scheduleLooksRefresh() {
  window.clearTimeout(looksTimer);
  looksTimer = window.setTimeout(() => { void loadLooksStrip(); }, 450);
}

/** Reflect the selected look on the filmstrip chips. */
function setActiveLook(look) {
  if (S.doc) S.doc.look = look || null;
  const row = S.els.lookRow;
  if (!row) return;
  const keys = (S.doc?.looks || []).map((l) => l.key);
  const active = S.doc?.look || null;
  [...(row.children || [])].forEach((chip, i) => {
    chip.classList.toggle('is-active', keys[i] === active);
  });
  updateChrome();
}

/** Apply a server-echoed dial state to the document + dials (no request). */
function setSettings(settings) {
  if (!S.doc) return;
  S.doc.settings = settings || {};
  for (const dial of S.els.dials || []) dial.set(S.doc.settings[dial.key] || 0);
}

/** Re-read the document's live state (an AI edit may have baked it away). */
async function syncDocState() {
  if (!S.doc) return;
  try {
    const meta = await api.fetchImageMeta(S.doc.image_id);
    if (!meta) return;
    S.doc.looks = meta.looks || S.doc.looks;
    setSettings(meta.adjust || {});
    setActiveLook(meta.look || null);
    if ((meta.looks || []).length !== (S.els.lookRow?.children || []).length) renderLooks();
  } catch (_) { /* best effort */ }
}

/** A lone look op only swaps the selection — the strip tiles stay valid. */
const isPureLook = (ops) => ops.length === 1 && ops[0]?.op === 'look';

/* ── Adjustments ────────────────────────────────────────────────── */

function fmtValue(spec, v) {
  const n = spec.step < 1 ? String(Math.round(v * 100) / 100) : String(Math.round(v));
  return v > 0 ? `+${n}` : n;
}

/* Dials — persistent, non-destructive settings. Each value lives on the
   document (`images.adjust`) and the picture renders as look(settings(bytes)),
   so dials stay where you put them and fine-tuning a knob REPLACES its value
   instead of stacking deltas. Drag vertical, scroll, or use the arrow keys;
   double-click resets one dial. */
function buildDial(spec) {
  const control = dial({
    label: t(spec.labelKey, null, spec.label), min: spec.min, max: spec.max, step: spec.step,
    format: value => fmtValue(spec, value),
    onInput: value => queueSettings({ [spec.key]: value }, { commit: false }),
    onChange: value => queueSettings({ [spec.key]: value }, { commit: true }),
    onCancel: () => { control.setValue(S.doc?.settings?.[spec.key] || 0); void reloadComposite(); },
  });
  return { key: spec.key, el: control, set: control.setValue, destroy: control.destroy };
}

function buildAdjustBody() {
  const wrap = el('div', 'image-adjust');

  const grid = el('div', 'image-dial-row');
  S.els.dials = ADJUST_DIALS.map((spec) => {
    const dial = buildDial(spec);
    grid.appendChild(dial.el);
    return dial;
  });
  wrap.appendChild(grid);

  const actions = el('div', 'image-oneclicks');
  for (const [op, key, label, iconName] of ONE_CLICKS) {
    const b = button({ label: t(key, null, label), icon: iconName, variant: 'ghost', size: 'sm', onClick: () => void applyOp({ op }) });
    b.classList.add('image-oneclick');
    actions.appendChild(b);
  }
  const revert = button({ label: t('plugin.image.oneClick.revert', null, 'Revert'), icon: 'ui/undo', variant: 'quiet', size: 'sm', onClick: () => void resetCurrent() });
  revert.classList.add('image-oneclick', 'image-oneclick--revert');
  revert.title = t('plugin.image.oneClick.revertTitle', null, 'Restore the original image');
  actions.appendChild(revert);
  wrap.appendChild(actions);

  return wrap;
}

/* ── Rendering pipeline ─────────────────────────────────────────── */

let offscreen = null;

export function plotRaw(w, h, buf) {
  S.render = { w, h, data: new Uint8ClampedArray(buf) };
  if (!offscreen) offscreen = document.createElement('canvas');
  offscreen.width = w;
  offscreen.height = h;
  offscreen.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(buf), w, h), 0, 0);
  if (S.doc) {
    S.doc.width = w;
    S.doc.height = h;
  }
  paintView();
}

export function paintView() {
  const { view, area } = S.els;
  if (!view || !area) return;
  if (!S.doc || !S.render) {
    view.width = 10;
    view.height = 10;
    view.style.display = 'none';
    renderEmptyState();
    updateChrome();
    return;
  }
  view.style.display = '';
  area.querySelector('.image-empty')?.remove();
  const { w, h } = S.render;
  // First paint after the window is laid out: fit the document to the view.
  if (S.fit && area.clientWidth > 60 && area.clientHeight > 60) {
    const z = Math.min(
      (area.clientWidth - 40) / Math.max(1, w),
      (area.clientHeight - 40) / Math.max(1, h),
      4,
    );
    S.zoom = Math.max(0.05, z);
  }
  const zw = Math.max(1, Math.round(w * S.zoom));
  const zh = Math.max(1, Math.round(h * S.zoom));
  if (view.width !== zw || view.height !== zh) {
    view.width = zw;
    view.height = zh;
  }
  const ctx = view.getContext('2d');
  ctx.imageSmoothingEnabled = S.zoom < 1;
  ctx.clearRect(0, 0, zw, zh);
  if (offscreen && offscreen.width) ctx.drawImage(offscreen, 0, 0, zw, zh);
  if (S.enterPending) {
    S.enterPending = false;
    void cardEnter(view);
  }
  updateChrome();
}

function renderEmptyState() {
  const { area } = S.els;
  if (!area) return;
  let empty = area.querySelector('.image-empty');
  if (!empty) {
    empty = el('div', 'image-empty');
    area.appendChild(empty);
  }
  empty.textContent = '';
  empty.appendChild(emptyState({
    icon: 'ui/image',
    title: t('plugin.image.empty.title', null, 'No image open'),
    body: t('plugin.image.empty.body', null, 'Open or drop a picture to start.'),
  }));
  const open = button({ label: t('plugin.image.empty.open', null, 'Open image'), icon: 'ui/folder-open', arrow: true, onClick: () => void pickFile() });
  open.classList.add('image-empty-open');
  empty.appendChild(open);
}

/** "4032×3024 · 63% · LOOK: NOIR" — the open document at a glance. */
function chromeMeta() {
  if (!S.doc) return [];
  const look = (S.doc.looks || []).find((l) => l.key === S.doc.look);
  return [
    S.doc.width && S.doc.height ? `${S.doc.width}×${S.doc.height}` : '',
    `${Math.round(S.zoom * 100)}%`,
    look ? t('plugin.image.status.look', { name: look.name }, `Look: ${look.name}`) : '',
  ];
}

export function updateChrome() {
  if (S.els.appTitle) S.els.appTitle.textContent = S.doc ? S.doc.title : t('plugin.image.title', null, 'Image');
  S.els.appMeta?.set(chromeMeta());
  if (S.els.zoomChip) {
    S.els.zoomChip.textContent = !S.doc ? ''
      : S.fit ? t('plugin.image.toolbar.actualSize', null, '1:1') : t('plugin.image.toolbar.fit', null, 'Fit');
  }
  for (const b of S.els.toolButtons || []) b.disabled = !S.doc;
}

/* ── Zoom & pan ─────────────────────────────────────────────────── */

export function setZoom(z, { focus = null } = {}) {
  const clamped = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, z));
  const area = S.els.area;
  let cx = null;
  let cy = null;
  if (focus && S.doc) {
    const rect = S.els.view.getBoundingClientRect();
    cx = focus.clientX - rect.left;
    cy = focus.clientY - rect.top;
  }
  const relX = area && cx != null ? (area.scrollLeft + cx) / Math.max(1, S.zoom) : null;
  const relY = area && cy != null ? (area.scrollTop + cy) / Math.max(1, S.zoom) : null;
  S.zoom = clamped;
  S.fit = false;
  paintView();
  if (area && relX != null) {
    area.scrollLeft = relX * S.zoom - cx;
    area.scrollTop = relY * S.zoom - cy;
  }
}

export function zoomToFit() {
  if (!S.doc || !S.els.area) return;
  S.fit = true;
  paintView();
}

let keysWired = false;

function onKeyDown(e) {
  const target = e.target;
  if (target instanceof Element && target.closest('input, textarea, select')) return;
  const mod = e.ctrlKey || e.metaKey;
  if (!mod) return;
  if (e.key === '=' || e.key === '+') {
    e.preventDefault();
    setZoom(S.zoom * 1.15);
  } else if (e.key === '-') {
    e.preventDefault();
    setZoom(S.zoom / 1.15);
  } else if (e.key === '0') {
    e.preventDefault();
    zoomToFit();
  }
}

export function wireCanvas(view, area) {
  view.style.touchAction = 'none';
  S.pointers = new Map();
  const toggleFit = () => (S.fit ? setZoom(1) : zoomToFit());
  const endDrag = (pointerId) => {
    S.drag = null;
    view.classList.remove('is-panning');
    try { view.releasePointerCapture(pointerId); } catch (_) { /* released */ }
  };
  const pinchDistance = () => {
    const [a, b] = [...S.pointers.values()];
    return Math.hypot(b.x - a.x, b.y - a.y);
  };
  const pinchCentre = () => {
    const [a, b] = [...S.pointers.values()];
    return { clientX: (a.x + b.x) / 2, clientY: (a.y + b.y) / 2 };
  };
  view.addEventListener('pointerdown', (e) => {
    if (!S.doc || e.button === 2) return;
    view.setPointerCapture(e.pointerId);
    S.pointers.set(e.pointerId, { x: e.clientX, y: e.clientY, type: e.pointerType, moved: false });
    if (S.pointers.size === 2) {
      // A second finger: the pan becomes a pinch around the fingers' midpoint.
      S.drag = null;
      view.classList.remove('is-panning');
      S.pinch = { dist: Math.max(1, pinchDistance()), zoom: S.zoom };
      return;
    }
    if (S.pointers.size > 2) return;
    S.drag = { sx: e.clientX, sy: e.clientY, sl: area.scrollLeft, st: area.scrollTop };
    view.classList.add('is-panning');
  });
  view.addEventListener('pointermove', (e) => {
    const p = S.pointers.get(e.pointerId);
    if (p) {
      if (Math.abs(e.clientX - p.x) > 8 || Math.abs(e.clientY - p.y) > 8) p.moved = true;
      p.x = e.clientX;
      p.y = e.clientY;
    }
    if (!S.doc) return;
    if (S.pinch && S.pointers.size === 2) {
      // Two fingers: zoom by the distance ratio, clamped like the chip.
      const z = S.pinch.zoom * (pinchDistance() / S.pinch.dist);
      if (Math.abs(z - S.zoom) > 0.002) setZoom(z, { focus: pinchCentre() });
      return;
    }
    if (!S.drag) return;
    area.scrollLeft = S.drag.sl - (e.clientX - S.drag.sx);
    area.scrollTop = S.drag.st - (e.clientY - S.drag.sy);
  });
  const end = (e) => {
    const p = S.pointers.get(e.pointerId);
    S.pointers.delete(e.pointerId);
    if (S.pinch) {
      // Lifting either finger ends the pinch; the other finger starts fresh.
      S.pinch = null;
      S.pointers.clear();
      endDrag(e.pointerId);
      return;
    }
    if (S.drag) endDrag(e.pointerId);
    // A touch double-tap toggles fit ↔ 1:1 (touch fires no dblclick).
    if (e.type === 'pointerup' && p && p.type === 'touch' && !p.moved) {
      const now = Date.now();
      const last = S.lastTap;
      if (last && now - last.at < 350 && Math.hypot(e.clientX - last.x, e.clientY - last.y) < 30) {
        S.lastTap = null;
        S.touchToggledAt = now;
        toggleFit();
      } else {
        S.lastTap = { at: now, x: e.clientX, y: e.clientY };
      }
    }
  };
  view.addEventListener('pointerup', end);
  view.addEventListener('pointercancel', end);
  view.addEventListener('dblclick', (e) => {
    // A touch double-tap is handled on pointerup; some browsers fire both.
    if (e.sourceCapabilities?.firesTouchEvents || Date.now() - (S.touchToggledAt || 0) < 500) return;
    toggleFit();
  });
  view.addEventListener('wheel', (e) => {
    if (!S.doc) return;
    e.preventDefault();
    setZoom(S.zoom * (e.deltaY < 0 ? 1.12 : 1 / 1.12), { focus: e });
  }, { passive: false });

  if (!keysWired) {
    window.addEventListener('keydown', onKeyDown);
    keysWired = true;
  }
}

/* ── Apply pipeline ─────────────────────────────────────────────── */

/** Send a request body ({operations} or {settings}) and sync the live state. */
async function send(body, { commit }) {
  const res = await api.rawApply(S.doc.image_id, body, { commit });
  plotRaw(res.w, res.h, res.buf);
  setSettings(res.adjust);
  setActiveLook(res.look);
  // The strip previews render over the settings base, so they follow every
  // commit except a pure look swap (which leaves the base untouched).
  if (commit && !(body.operations && isPureLook(body.operations))) scheduleLooksRefresh();
}

/** Queue a dial-settings update (`{brightness: 30}` — merged server-side). */
export function queueSettings(update, { commit = true } = {}) {
  if (!S.doc) {
    toast(t('plugin.image.toast.openFirst', null, 'Open an image first'), { type: 'error' });
    return;
  }
  S.pending = { body: { settings: update }, commit };
  if (!S.applying) void pump();
}

async function pump() {
  if (!S.doc || S.applying || !S.pending) return;
  S.applying = true;
  const { body, commit } = S.pending;
  S.pending = null;
  try {
    await send(body, { commit });
  } catch (e) {
    toast(e.message || t('plugin.image.toast.editFailed', null, 'Edit failed'), { type: 'error' });
  } finally {
    S.applying = false;
    if (S.pending) void pump();
  }
}

/** Run one operation (or a batch) to completion, awaiting the result. */
export async function applyOp(op, { commit = true } = {}) {
  if (!S.doc) {
    toast(t('plugin.image.toast.openFirst', null, 'Open an image first'), { type: 'error' });
    return;
  }
  const operations = Array.isArray(op) ? op : [op];
  try {
    await send({ operations }, { commit });
  } catch (e) {
    toast(e.message || t('plugin.image.toast.operationFailed', null, 'Operation failed'), { type: 'error' });
  }
}

/* ── Document lifecycle ─────────────────────────────────────────── */

export async function reloadComposite() {
  if (!S.doc) return;
  try {
    const { w, h, buf } = await api.fetchRender(S.doc.image_id);
    plotRaw(w, h, buf);
  } catch (e) {
    toast(e.message || t('plugin.image.toast.renderFailed', null, 'Could not render image'), { type: 'error' });
  }
}

export async function refreshImages() {
  try {
    const r = await api.listImages();
    S.docs = r?.images || [];
  } catch (_) { /* keep the last list */ }
}

export async function openImage(meta) {
  S.doc = {
    image_id: meta.image_id,
    title: meta.title,
    width: meta.width,
    height: meta.height,
    looks: meta.looks || [],
    look: meta.look || null,
    settings: meta.adjust || {},
  };
  S.render = null;
  S.fit = true;
  S.enterPending = true;
  renderLooks();
  setSettings(S.doc.settings);
  await reloadComposite();
  updateChrome();
}

/** Open by id, refreshing the metadata so the look list is present. */
async function openImageById(imageId) {
  try {
    const meta = await api.fetchImageMeta(imageId);
    if (meta && meta.image_id) {
      await openImage(meta);
      return true;
    }
  } catch (_) { /* fall back to the list entry */ }
  return false;
}

export async function openNewest() {
  await refreshImages();
  if (S.docs.length) {
    if (!(await openImageById(S.docs[0].image_id))) await openImage(S.docs[0]);
  } else {
    S.doc = null;
    S.render = null;
    renderLooks();
    paintView();
  }
}

export async function pickFile() {
  const [file] = await pickFiles({ accept: 'image/*' });
  if (file) void uploadFile(file);
}

export async function uploadFile(file) {
  try {
    const created = await api.createImage(file);
    await refreshImages();
    // The create response carries no look table; fetch the full metadata.
    if (!(await openImageById(created.image_id))) await openImage(created);
    toast(t('plugin.image.toast.opened', { title: created.title }, 'Opened {title}'), { type: 'info' });
  } catch (e) {
    toast(e.message || t('plugin.image.toast.uploadFailed', null, 'Upload failed'), { type: 'error' });
  }
}

export async function downloadCurrent() {
  if (!S.doc) return;
  try {
    const blob = await api.fetchPng(S.doc.image_id);
    await saveOrDownload(blob, {
      name: `${S.doc.title || 'image'}.png`,
      dir: 'Pictures',
      app: 'Image',
    });
  } catch (e) {
    toast(e.message || t('plugin.image.toast.saveFailed', null, 'Save failed'), { type: 'error' });
  }
}

export async function deleteCurrent(imageId = S.doc?.image_id, title = S.doc?.title) {
  if (!imageId) return;
  const name = title || t('plugin.image.dialog.thisImage', null, 'this image');
  if (!await confirmDialog(t('plugin.image.dialog.deleteConfirm', { title: name }, 'Delete "{title}"?'))) return;
  try {
    await api.deleteImage(imageId);
    if (S.doc?.image_id === imageId) S.doc = null;
    await openNewest();
    toast(t('plugin.image.toast.deleted', null, 'Image deleted'), { type: 'info' });
  } catch (e) {
    toast(e.message || t('plugin.image.toast.deleteFailed', null, 'Could not delete image'), { type: 'error' });
  }
}

export async function resetCurrent() {
  if (!S.doc) return;
  await applyOp({ op: 'reset' });
  toast(t('plugin.image.toast.reverted', null, 'Reverted to the original image'), { type: 'info' });
}

export async function rotateCurrent(dir) {
  if (!S.doc) return;
  await applyOp({ op: 'rotate', angle: dir === 'left' ? -90 : 90 });
  zoomToFit();
}

export async function flipCurrent(axis) {
  if (!S.doc) return;
  await applyOp({ op: axis === 'vertical' ? 'flip_v' : 'flip_h' });
}

/* ── Mount / unmount ────────────────────────────────────────────── */

export function mountEditor() {
  const tile = buildWorkspace();
  wireDropTarget(S.els.area);
  if (typeof ResizeObserver !== 'undefined' && S.els.area) {
    S.resizeObs = new ResizeObserver(() => {
      if (S.fit) paintView();
    });
    S.resizeObs.observe(S.els.area);
  }
  watchPhone();
  void openNewest();
  return tile;
}

export function getEditorElement() {
  return S.tile;
}

export function unmountEditor() {
  for (const control of S.els.dials || []) control.destroy();
  S.els.sheetTabs?.destroy?.();
  S.sheet?.destroy();
  S.sheet = null;
  S.phone = false;
  unwatchPhone();
  window.removeEventListener('keydown', onKeyDown);
  keysWired = false;
  window.clearTimeout(looksTimer);
  S.resizeObs?.disconnect();
  S.resizeObs = null;
  S.tile?.remove();
  S.tile = null;
  S.els = {};
  S.docs = [];
  S.doc = null;
  S.render = null;
  S.drag = null;
  S.pinch = null;
  S.pointers = null;
  S.lastTap = null;
  if (S.looksUrl) URL.revokeObjectURL(S.looksUrl);
  S.looksUrl = '';
}

/* ── Menus ──────────────────────────────────────────────────────── */

export function openMainMenu(anchor) {
  const rect = anchor?.getBoundingClientRect?.() || { left: 8, bottom: 8 };
  const entries = buildMenu();
  if (entries.length) openContextMenu(entries, rect.left, rect.bottom + 3);
}

function buildMenu() {
  const hasDoc = !!S.doc;
  return [
    { type: 'heading', label: t('plugin.image.menu.file', null, 'File') },
    { type: 'item', label: t('plugin.image.menu.open', null, 'Open…'), icon: 'ui/upload', onClick: () => void pickFile() },
    { type: 'item', label: t('plugin.image.menu.saveCopy', null, 'Save a Copy…'), icon: 'ui/save', disabled: !hasDoc, onClick: () => void downloadCurrent() },
    { type: 'separator' },
    { type: 'item', label: t('plugin.image.menu.delete', null, 'Delete image'), icon: 'ui/trash', danger: true, disabled: !hasDoc, onClick: () => void deleteCurrent() },
    { type: 'separator' },
    { type: 'heading', label: t('plugin.image.menu.image', null, 'Image') },
    { type: 'item', label: t('plugin.image.menu.rotateLeft', null, 'Rotate left'), icon: 'ui/rotate-left', disabled: !hasDoc, onClick: () => void rotateCurrent('left') },
    { type: 'item', label: t('plugin.image.menu.rotateRight', null, 'Rotate right'), icon: 'ui/rotate-right', disabled: !hasDoc, onClick: () => void rotateCurrent('right') },
    { type: 'item', label: t('plugin.image.menu.flipH', null, 'Flip horizontal'), icon: 'ui/flip-h', disabled: !hasDoc, onClick: () => void flipCurrent('horizontal') },
    { type: 'item', label: t('plugin.image.menu.flipV', null, 'Flip vertical'), icon: 'ui/flip-v', disabled: !hasDoc, onClick: () => void flipCurrent('vertical') },
    { type: 'separator' },
    { type: 'item', label: t('plugin.image.menu.revert', null, 'Revert to original'), icon: 'ui/refresh', disabled: !hasDoc, onClick: () => void resetCurrent() },
    { type: 'separator' },
    { type: 'heading', label: t('plugin.image.menu.view', null, 'View') },
    { type: 'item', label: t('plugin.image.menu.zoomIn', null, 'Zoom in'), icon: 'ui/zoom-in', onClick: () => setZoom(S.zoom * 1.15) },
    { type: 'item', label: t('plugin.image.menu.zoomOut', null, 'Zoom out'), icon: 'ui/zoom-out', onClick: () => setZoom(S.zoom / 1.15) },
    { type: 'item', label: t('plugin.image.menu.fit', null, 'Fit'), onClick: zoomToFit },
    { type: 'item', label: t('plugin.image.menu.actualSize', null, 'Actual size'), onClick: () => setZoom(1) },
  ];
}

/* ── Window integration ─────────────────────────────────────────── */

export function wireEditorEvents() {
  if (S.wired) return;
  S.wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  onOpenFromFiles(PLUGIN_NAME, (d) => importFromFiles(d.path, d.name));
}

export async function importFromFiles(path, name) {
  try {
    if (!S.tile) mountEditor();
    const blob = await api.blobFromHome(path);
    if (!blob || blob.size === 0) throw new Error(t('plugin.image.error.emptyFile', null, 'That file is empty'));
    const type = blob.type || '';
    const ext = String(name || '').toLowerCase().split('.').pop();
    const knownImage = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'].includes(ext);
    if (type && !type.startsWith('image/') && !knownImage) {
      const label = name || t('plugin.image.error.unnamedFile', null, 'file');
      throw new Error(t('plugin.image.error.notAnImage', { name: label }, '“{name}” is not an image the editor can open'));
    }
    const file = new File([blob], name || 'image', { type: type || 'image/png' });
    await uploadFile(file);
  } catch (e) {
    toast(e.message || t('plugin.image.toast.openFailed', null, 'Could not open file'), { type: 'error' });
  }
}

function onAgentActions(e) {
  const actions = (e.detail || []).filter((a) => /^image_/.test(a?.action || ''));
  if (!actions.length) return;
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: PLUGIN_NAME } }));
  const touchedId = actions.map((a) => a.data?.image_id).find((id) => !!id);
  const deleted = actions.some((a) => a.action === 'image_delete' && a.result === 'ok');
  void (async () => {
    await refreshImages();
    if (deleted) {
      if (S.doc && !S.docs.some((i) => i.image_id === S.doc.image_id)) await openNewest();
    } else if (touchedId && S.doc?.image_id !== touchedId) {
      const found = S.docs.find((i) => i.image_id === touchedId);
      if (found && !(await openImageById(touchedId))) await openImage(found);
    } else if (S.doc) {
      await reloadComposite();
      await syncDocState();
      scheduleLooksRefresh();
    }
  })();
}

export function editorContextMenu() {
  const hasDoc = !!S.doc;
  return [
    { type: 'item', label: t('plugin.image.ctx.open', null, 'Open image…'), icon: 'ui/upload', onClick: () => void pickFile() },
    { type: 'item', label: t('plugin.image.menu.revert', null, 'Revert to original'), icon: 'ui/refresh', disabled: !hasDoc, onClick: () => void resetCurrent() },
    { type: 'separator' },
    { type: 'item', label: t('plugin.image.menu.delete', null, 'Delete image'), icon: 'ui/trash', danger: true, disabled: !hasDoc, onClick: () => void deleteCurrent() },
  ];
}

/* Drag an image file onto the canvas to open it. */
export function wireDropTarget(area) {
  if (!area) return;
  area.addEventListener('dragover', (e) => { e.preventDefault(); area.classList.add('is-drop'); });
  area.addEventListener('dragleave', () => area.classList.remove('is-drop'));
  area.addEventListener('drop', (e) => {
    e.preventDefault();
    area.classList.remove('is-drop');
    const file = [...(e.dataTransfer?.files || [])].find((f) => f.type.startsWith('image/'));
    if (file) void uploadFile(file);
  });
}
