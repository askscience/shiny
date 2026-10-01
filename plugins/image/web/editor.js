/**
 * editor.js — the Image window: a docked raster-editor workspace.
 *
 * Layout: an application menu bar, a context options bar for the active tool,
 * document tabs, a two-column tool palette, rulers and the document canvas, a
 * panel dock on the right and a status bar. Every pixel change goes through the
 * plugin's Rust engine (adjustments, filters, paint, transforms); this module
 * only builds DOM, tracks state and paints returned RGBA.
 *
 * All window styling lives in the core stylesheet (`web/css/tiles.css`, the
 * `image-*` block) — this file ships no CSS, per the plugin UI contract.
 *
 * NOTE: `../` paths point at the app root (this module lives three levels deep,
 * `plugins/image/web/`).
 */

import {
  button, setIcon, slider, select, toast, emptyState, modal, toggle, row,
} from '../../../ui/index.js';
import { openContextMenu } from '../../../js/contextMenu.js';
import { saveOrDownload, pickFiles, onOpenFromFiles } from '../../../js/files.js';
import * as api from './api.js';
import * as sel from './selection.js';

export const PLUGIN_NAME = 'image';

/* ── Static tables ──────────────────────────────────────────────── */

export const BLEND_MODES = [
  'normal', 'multiply', 'screen', 'overlay', 'darken', 'lighten',
  'difference', 'color_dodge', 'color_burn', 'add', 'subtract',
];

const PRESET_FILTERS = [
  'oceanic', 'islands', 'marine', 'seagreen', 'flagblue', 'diamante', 'liquid',
  'radio', 'twenties', 'rosetint', 'mauve', 'bluechrome', 'vintage', 'perfume',
  'serenity', 'golden', 'pastel_pink', 'cali', 'dramatic', 'firenze', 'obsidian', 'lofi',
];

/**
 * Tool palette groups, in palette order. Each group renders one button that
 * shows its active tool; the corner marker opens the fly-out for the rest.
 */
const TOOL_GROUPS = [
  { icon: 'ui/move', label: 'Move', tools: [{ id: 'move', label: 'Move', icon: 'ui/move', key: 'V' }] },
  {
    icon: 'ui/marquee-rect',
    label: 'Marquee',
    tools: [
      { id: 'marquee-rect', label: 'Rectangular Marquee', icon: 'ui/marquee-rect', key: 'M' },
      { id: 'marquee-ellipse', label: 'Elliptical Marquee', icon: 'ui/marquee-ellipse', key: 'M' },
    ],
  },
  {
    icon: 'ui/lasso',
    label: 'Lasso',
    tools: [
      { id: 'lasso', label: 'Lasso', icon: 'ui/lasso', key: 'L' },
      { id: 'lasso-poly', label: 'Polygonal Lasso', icon: 'ui/paths', key: 'L' },
    ],
  },
  { icon: 'ui/magic-wand', label: 'Magic Wand', tools: [{ id: 'wand', label: 'Magic Wand', icon: 'ui/magic-wand', key: 'W' }] },
  { icon: 'ui/crop', label: 'Crop', tools: [{ id: 'crop', label: 'Crop', icon: 'ui/crop', key: 'C' }] },
  { icon: 'ui/eyedropper', label: 'Eyedropper', tools: [{ id: 'eyedropper', label: 'Eyedropper', icon: 'ui/eyedropper', key: 'I' }] },
  { icon: 'ui/clone-stamp', label: 'Clone Stamp', tools: [{ id: 'clone', label: 'Clone Stamp', icon: 'ui/clone-stamp', key: 'S' }] },
  {
    icon: 'ui/brush',
    label: 'Brush',
    tools: [
      { id: 'brush', label: 'Brush', icon: 'ui/brush', key: 'B' },
      { id: 'pencil', label: 'Pencil', icon: 'ui/pencil', key: 'B' },
    ],
  },
  { icon: 'ui/eraser', label: 'Eraser', tools: [{ id: 'eraser', label: 'Eraser', icon: 'ui/eraser', key: 'E' }] },
  {
    icon: 'ui/gradient',
    label: 'Gradient',
    tools: [
      { id: 'gradient-linear', label: 'Gradient (Linear)', icon: 'ui/gradient', key: 'G' },
      { id: 'gradient-radial', label: 'Gradient (Radial)', icon: 'ui/gradient', key: 'G' },
    ],
  },
  { icon: 'ui/paint-bucket', label: 'Paint Bucket', tools: [{ id: 'bucket', label: 'Paint Bucket', icon: 'ui/paint-bucket', key: 'G' }] },
  {
    icon: 'ui/blur',
    label: 'Blur / Sharpen',
    tools: [
      { id: 'blur-tool', label: 'Blur', icon: 'ui/blur' },
      { id: 'sharpen-tool', label: 'Sharpen', icon: 'ui/sharpen' },
      { id: 'smudge-tool', label: 'Smudge', icon: 'ui/sponge' },
    ],
  },
  {
    icon: 'ui/dodge',
    label: 'Dodge / Burn',
    tools: [
      { id: 'dodge', label: 'Dodge', icon: 'ui/dodge', key: 'O' },
      { id: 'burn', label: 'Burn', icon: 'ui/burn', key: 'O' },
    ],
  },
  { icon: 'ui/type', label: 'Type', tools: [{ id: 'type', label: 'Horizontal Type', icon: 'ui/type', key: 'T' }] },
  {
    icon: 'ui/shape',
    label: 'Shape',
    tools: [
      { id: 'shape-rect', label: 'Rectangle', icon: 'ui/shape', key: 'U' },
      { id: 'shape-ellipse', label: 'Ellipse', icon: 'ui/shape', key: 'U' },
      { id: 'shape-line', label: 'Line', icon: 'ui/shape', key: 'U' },
      { id: 'shape-polygon', label: 'Polygon', icon: 'ui/shape', key: 'U' },
      { id: 'shape-star', label: 'Star', icon: 'ui/shape', key: 'U' },
    ],
  },
  { icon: 'ui/hand', label: 'Hand', tools: [{ id: 'hand', label: 'Hand', icon: 'ui/hand', key: 'H' }] },
  {
    icon: 'ui/zoom-in',
    label: 'Zoom',
    tools: [
      { id: 'zoom', label: 'Zoom', icon: 'ui/zoom-in', key: 'Z' },
      { id: 'zoom-out', label: 'Zoom Out', icon: 'ui/zoom-out', key: 'Z' },
    ],
  },
];

/** A short hint shown in the status bar per tool. */
const TOOL_HINTS = {
  move: 'Drag to move the active layer',
  'marquee-rect': 'Drag to select a rectangle',
  'marquee-ellipse': 'Drag to select an ellipse',
  lasso: 'Drag to draw a freehand selection',
  'lasso-poly': 'Click to add points, double-click to close',
  wand: 'Click a colour to select similar pixels',
  crop: 'Drag a crop box, then press Enter',
  eyedropper: 'Click to sample a colour',
  clone: 'Alt-click to set the source, then paint',
  brush: 'Drag to paint with the foreground colour',
  pencil: 'Drag to paint hard-edged pixels',
  eraser: 'Drag to erase pixels',
  'gradient-linear': 'Drag to draw a linear gradient',
  'gradient-radial': 'Drag to draw a radial gradient',
  bucket: 'Click to flood-fill with the foreground colour',
  'blur-tool': 'Drag to soften pixels',
  'sharpen-tool': 'Drag to sharpen pixels',
  'smudge-tool': 'Drag to smear pixels',
  dodge: 'Drag to lighten pixels',
  burn: 'Drag to darken pixels',
  type: 'Click to place text',
  'shape-rect': 'Drag to draw a rectangle',
  'shape-ellipse': 'Drag to draw an ellipse',
  'shape-line': 'Drag to draw a line',
  'shape-polygon': 'Drag to draw a polygon',
  'shape-star': 'Drag to draw a star',
  hand: 'Drag to pan the document',
  zoom: 'Click to zoom in, Alt-click to zoom out',
  'zoom-out': 'Click to zoom out',
};

const MAX_DIM = 1600;

/* ── State ──────────────────────────────────────────────────────── */

export const S = {
  tile: null,
  els: {},
  docs: [],
  doc: null,
  layers: [],
  activeLayerId: null,
  fg: [0, 0, 0],
  bg: [255, 255, 255],
  tool: 'move',
  groupActive: {},
  toolOptions: {
    marqueeMode: 'new',
    feather: 0,
    tolerance: 32,
    contiguous: true,
    size: 24,
    hardness: 80,
    opacity: 100,
    flow: 100,
    strength: 30,
    antiAlias: true,
    cloneAligned: true,
    font: 'sans-serif',
    fontSize: 48,
    sides: 6,
    innerRatio: 50,
  },
  mask: null,
  maskW: 0,
  maskH: 0,
  edges: [],
  antsPhase: 0,
  antsTimer: null,
  zoom: 1,
  fit: true,
  render: null, // { w, h, data: Uint8ClampedArray }
  pending: null,
  applying: false,
  cloneSource: null,
  cropRect: null,
  drag: null,
  history: [],
  panelCollapsed: {},
  thumbs: [],
  wired: false,
  inline: null,
  resizeObs: null,
};

/* ── Small helpers ──────────────────────────────────────────────── */

export function rgbCss([r, g, b], a = 1) {
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

export function hexOf([r, g, b]) {
  return `#${[r, g, b].map((v) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, '0')).join('')}`;
}

export function parseHex(hex) {
  const s = (hex || '').replace('#', '');
  if (s.length < 6) return null;
  return [parseInt(s.slice(0, 2), 16), parseInt(s.slice(2, 4), 16), parseInt(s.slice(4, 6), 16)];
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

export function iconButton(iconName, label, onClick, { size = 16, className = '', variant = 'ghost' } = {}) {
  const btn = button({ variant, onClick });
  btn.classList.add('ui-btn--icon');
  if (className) btn.classList.add(...className.split(' '));
  btn.title = label;
  btn.setAttribute('aria-label', label);
  const ic = el('span', 'image-icon');
  btn.appendChild(ic);
  void setIcon(ic, iconName, { size });
  return btn;
}

function menuLabel(parent, text) {
  const span = el('span', 'image-menu-title', text);
  parent.appendChild(span);
  return span;
}

/* ── Workspace skeleton ─────────────────────────────────────────── */

export function buildWorkspace() {
  const tile = el('section', 'tile image-tile');
  tile.dataset.plugin = PLUGIN_NAME;
  S.tile = tile;

  /* Application bar: one menu button (core context menu) + document title. */
  const appbar = el('div', 'image-appbar');
  appbar.dataset.windowBar = '';
  const menuBtn = iconButton('ui/list', 'Menu', (e) => openMainMenu(e.currentTarget));
  menuBtn.classList.add('image-logo');
  const appTitle = el('span', 'image-appbar-title');
  S.els.appTitle = appTitle;
  const appRight = el('div', 'image-appbar-right');
  const optsToggle = iconButton('ui/properties', 'Toggle panels', () => {
    tile.classList.toggle('is-dock-hidden');
  });
  appRight.append(optsToggle);
  appbar.append(menuBtn, appTitle, appRight);
  tile.appendChild(appbar);

  /* Tool options bar. */
  const options = el('div', 'image-options');
  S.els.options = options;
  tile.appendChild(options);

  /* Document tab strip. */
  const tabs = el('div', 'image-tabs');
  S.els.tabs = tabs;
  tile.appendChild(tabs);

  /* Main: tool palette + workspace + dock. */
  const main = el('div', 'image-main');

  const tools = el('div', 'image-tools');
  S.els.tools = tools;
  buildToolPalette(tools);
  main.appendChild(tools);

  const workspace = el('div', 'image-workspace');

  const rulerCorner = el('div', 'image-ruler-corner');
  const rulerH = el('canvas', 'image-ruler-h');
  const rulerV = el('canvas', 'image-ruler-v');
  rulerH.width = 1;
  rulerH.height = 18;
  rulerV.width = 18;
  rulerV.height = 1;
  S.els.rulerH = rulerH;
  S.els.rulerV = rulerV;

  const area = el('div', 'image-canvas-area');
  const holder = el('div', 'image-doc');
  const view = el('canvas', 'image-view');
  const overlay = el('canvas', 'image-overlay');
  holder.append(view, overlay);
  area.appendChild(holder);
  S.els.area = area;
  S.els.holder = holder;
  S.els.view = view;
  S.els.overlay = overlay;

  workspace.append(rulerCorner, rulerH, rulerV, area);
  main.appendChild(workspace);

  const dock = el('div', 'image-dock');
  S.els.dock = dock;
  buildDock(dock);
  main.appendChild(dock);

  tile.appendChild(main);

  /* Status bar. */
  const status = el('div', 'image-statusbar');
  const statusTool = el('span', 'image-status-tool');
  const statusZoom = el('button', 'image-status-zoom');
  statusZoom.type = 'button';
  statusZoom.addEventListener('click', () => setZoom(1));
  const statusSize = el('span', 'image-status-size');
  const statusPos = el('span', 'image-status-pos');
  status.append(statusTool, el('span', 'image-status-spacer'), statusPos, statusSize, statusZoom);
  S.els.statusTool = statusTool;
  S.els.statusZoom = statusZoom;
  S.els.statusSize = statusSize;
  S.els.statusPos = statusPos;
  tile.appendChild(status);

  wireCanvas(view, overlay, area);
  return tile;
}

/* ── Tool palette ───────────────────────────────────────────────── */

export function buildToolPalette(container) {
  container.textContent = '';
  TOOL_GROUPS.forEach((group, gi) => {
    const activeId = S.groupActive[gi] ?? group.tools[0].id;
    const tool = group.tools.find((t) => t.id === activeId) || group.tools[0];
    const btn = iconButton(tool.icon, group.label, () => selectTool(tool.id), { className: 'image-tool' });
    btn.dataset.tool = tool.id;
    if (S.tool === tool.id) btn.classList.add('is-active');
    if (group.tools.length > 1) {
      btn.appendChild(el('span', 'image-tool-flyout'));
      btn.addEventListener('contextmenu', (e) => {
        e.preventDefault();
        e.stopPropagation();
        openToolFlyout(group, gi, btn);
      });
      let hold = null;
      btn.addEventListener('pointerdown', () => {
        hold = window.setTimeout(() => openToolFlyout(group, gi, btn), 350);
      });
      const cancel = () => window.clearTimeout(hold);
      btn.addEventListener('pointerup', cancel);
      btn.addEventListener('pointerleave', cancel);
    }
    container.appendChild(btn);
  });
}

function openToolFlyout(group, gi, anchor) {
  const activeId = S.groupActive[gi] ?? group.tools[0].id;
  const rect = anchor.getBoundingClientRect();
  const entries = group.tools.map((t) => ({
    type: 'item',
    label: t.key ? `${t.label}  (${t.key})` : t.label,
    icon: t.icon,
    checked: activeId === t.id,
    onClick: () => selectTool(t.id),
  }));
  openContextMenu(entries, rect.right + 4, rect.top);
}

export function selectTool(id) {
  S.tool = id;
  TOOL_GROUPS.forEach((g, gi) => {
    if (g.tools.some((t) => t.id === id)) S.groupActive[gi] = id;
  });
  S.cropRect = null;
  buildToolPalette(S.els.tools);
  renderOptionsBar();
  updateStatus();
  refreshCursor();
  drawOverlay();
}

/* ── Options bar ────────────────────────────────────────────────── */

const SELECTION_TOOLS = new Set(['marquee-rect', 'marquee-ellipse', 'lasso', 'lasso-poly', 'wand']);
const PAINT_TOOLS = new Set(['brush', 'pencil', 'eraser', 'clone', 'blur-tool', 'sharpen-tool', 'smudge-tool', 'dodge', 'burn']);

function smallField(label, control) {
  const wrap = el('label', 'image-opt-field');
  wrap.appendChild(el('span', 'image-opt-label', label));
  wrap.appendChild(control);
  return wrap;
}

function numberField(label, value, min, max, onChange) {
  const input = el('input', 'image-opt-number');
  input.type = 'number';
  input.min = String(min);
  input.max = String(max);
  input.value = String(value);
  input.addEventListener('change', () => {
    const v = Math.max(min, Math.min(max, Number(input.value) || 0));
    input.value = String(v);
    onChange(v);
  });
  return smallField(label, input);
}

function sliderField(label, value, min, max, onChange, unit = '') {
  const sl = slider({ min, max, step: 1, value });
  const val = el('span', 'image-opt-value', `${value}${unit}`);
  sl.addEventListener('input', () => {
    val.textContent = `${sl.value}${unit}`;
    onChange(Number(sl.value));
  });
  const wrap = el('label', 'image-opt-field image-opt-field--slider');
  wrap.append(el('span', 'image-opt-label', label), sl, val);
  return wrap;
}

function selectField(label, options, value, onChange) {
  const s = select({ options, value });
  s.select.classList.add('image-opt-select');
  s.select.addEventListener('change', () => onChange(s.select.value));
  return smallField(label, s);
}

function toggleField(label, checked, onChange) {
  const wrap = el('label', 'image-opt-field image-opt-field--toggle');
  wrap.appendChild(el('span', 'image-opt-label', label));
  const t = toggle({ checked, onChange });
  wrap.appendChild(t);
  return wrap;
}

function modeButtons() {
  const modes = [
    ['new', 'ui/marquee-rect', 'New selection'],
    ['add', 'ui/plus', 'Add to selection'],
    ['subtract', 'ui/minus', 'Subtract from selection'],
    ['intersect', 'ui/select-all', 'Intersect with selection'],
  ];
  const wrap = el('div', 'image-opt-modes');
  for (const [mode, iconName, label] of modes) {
    const b = iconButton(iconName, label, () => {
      S.toolOptions.marqueeMode = mode;
      renderOptionsBar();
    }, { size: 15 });
    if (S.toolOptions.marqueeMode === mode) b.classList.add('is-active');
    wrap.appendChild(b);
  }
  return wrap;
}

export function renderOptionsBar() {
  const bar = S.els.options;
  if (!bar) return;
  bar.textContent = '';
  const tool = findTool(S.tool);
  const lead = el('div', 'image-options-lead');
  const ic = el('span', 'image-icon');
  void setIcon(ic, tool?.icon || 'ui/image', { size: 16 });
  lead.append(ic, el('span', 'image-options-name', tool?.label || 'Tool'));
  bar.appendChild(lead);

  const opts = S.toolOptions;
  if (SELECTION_TOOLS.has(S.tool)) bar.appendChild(modeButtons());

  if (S.tool === 'marquee-rect' || S.tool === 'marquee-ellipse' || S.tool === 'lasso' || S.tool === 'lasso-poly') {
    bar.appendChild(numberField('Feather', opts.feather, 0, 100, (v) => { opts.feather = v; }));
    bar.appendChild(toggleField('Anti-alias', opts.antiAlias, (v) => { opts.antiAlias = v; }));
  }
  if (S.tool === 'wand') {
    bar.appendChild(sliderField('Tolerance', opts.tolerance, 0, 255, (v) => { opts.tolerance = v; }));
    bar.appendChild(toggleField('Contiguous', opts.contiguous, (v) => { opts.contiguous = v; }));
  }
  if (PAINT_TOOLS.has(S.tool)) {
    bar.appendChild(sliderField('Size', opts.size, 1, 400, (v) => { opts.size = v; }, ' px'));
    if (S.tool !== 'smudge-tool') {
      bar.appendChild(sliderField('Hardness', opts.hardness, 0, 100, (v) => { opts.hardness = v; }, '%'));
    }
    bar.appendChild(sliderField('Opacity', opts.opacity, 1, 100, (v) => { opts.opacity = v; }, '%'));
    if (S.tool === 'brush' || S.tool === 'pencil' || S.tool === 'clone' || S.tool === 'eraser') {
      bar.appendChild(sliderField('Flow', opts.flow, 1, 100, (v) => { opts.flow = v; }, '%'));
    }
    if (['blur-tool', 'sharpen-tool', 'smudge-tool', 'dodge', 'burn'].includes(S.tool)) {
      bar.appendChild(sliderField('Strength', opts.strength, 1, 100, (v) => { opts.strength = v; }, '%'));
    }
    if (S.tool === 'clone') {
      bar.appendChild(toggleField('Aligned', opts.cloneAligned, (v) => { opts.cloneAligned = v; }));
    }
  }
  if (S.tool === 'crop') {
    bar.appendChild(selectField('Ratio', ['Free', '1:1', '16:9', '4:3', '3:2'], 'Free', (v) => { opts.cropRatio = v; }));
  }
  if (S.tool === 'type') {
    bar.appendChild(selectField('Font', ['sans-serif', 'serif', 'monospace'], opts.font, (v) => { opts.font = v; }));
    bar.appendChild(numberField('Size', opts.fontSize, 6, 400, (v) => { opts.fontSize = v; }));
  }
  if (S.tool.startsWith('shape-')) {
    bar.appendChild(numberField('Stroke', opts.shapeStroke ?? 0, 0, 100, (v) => { opts.shapeStroke = v; }));
    if (S.tool === 'shape-polygon') {
      bar.appendChild(numberField('Sides', opts.sides, 3, 24, (v) => { opts.sides = v; }));
    }
    if (S.tool === 'shape-star') {
      bar.appendChild(numberField('Points', opts.starPoints ?? 5, 3, 24, (v) => { opts.starPoints = v; }));
      bar.appendChild(numberField('Inner', opts.innerRatio, 5, 100, (v) => { opts.innerRatio = v; }));
    }
  }
}

export function findTool(id) {
  for (const g of TOOL_GROUPS) {
    const t = g.tools.find((x) => x.id === id);
    if (t) return t;
  }
  return null;
}

/* ── Panel dock ─────────────────────────────────────────────────── */

const SWATCH_PRESETS = [
  '#000000', '#404040', '#808080', '#c0c0c0', '#ffffff',
  '#7f1d1d', '#dc2626', '#f97316', '#facc15', '#84cc16',
  '#16a34a', '#0d9488', '#0ea5e9', '#2563eb', '#4f46e5',
  '#7c3aed', '#c026d3', '#db2777', '#f472b6', '#fbcfe8',
];

/** One collapsible docked panel. */
function dockPanel(title, key, buildBody, { actions = [] } = {}) {
  const sec = el('section', 'image-panel-section');
  const head = el('div', 'image-panel-head');
  const chev = el('span', 'image-panel-chevron');
  void setIcon(chev, 'ui/chevron-down', { size: 12 });
  head.append(chev, el('span', 'image-panel-title', title));
  if (actions.length) {
    const box = el('div', 'image-panel-actions');
    for (const a of actions) box.appendChild(a);
    head.appendChild(box);
  }
  const body = el('div', 'image-panel-body');
  body.appendChild(buildBody());
  if (S.panelCollapsed[key]) sec.classList.add('is-collapsed');
  head.addEventListener('click', (e) => {
    if (e.target.closest('button, select, input, a')) return;
    const collapsed = sec.classList.toggle('is-collapsed');
    S.panelCollapsed[key] = collapsed;
  });
  sec.append(head, body);
  return sec;
}

export function buildDock(dock) {
  dock.textContent = '';

  dock.appendChild(dockPanel('Adjustments', 'adjustments', buildAdjustmentsBody));
  dock.appendChild(dockPanel('Properties', 'properties', buildPropertiesBody));
  dock.appendChild(dockPanel('Color', 'color', buildColorBody, {
    actions: [
      iconButton('ui/palette', 'Default colours', () => {
        S.fg = [0, 0, 0];
        S.bg = [255, 255, 255];
        refreshPanels();
      }, { size: 13 }),
    ],
  }));
  dock.appendChild(dockPanel('Swatches', 'swatches', buildSwatchesBody));
  dock.appendChild(dockPanel('Layers', 'layers', buildLayersBody, {
    actions: [iconButton('ui/plus', 'New layer', () => addLayer(), { size: 13 })],
  }));
  dock.appendChild(dockPanel('Channels', 'channels', buildChannelsBody));
  dock.appendChild(dockPanel('History', 'history', buildHistoryBody, {
    actions: [iconButton('ui/refresh', 'Refresh history', () => refreshHistory(), { size: 13 })],
  }));
  dock.appendChild(dockPanel('Navigator', 'navigator', buildNavigatorBody));
}

/* Adjustments library */
const ADJUSTMENT_ITEMS = [
  ['brightness_contrast', 'Brightness/Contrast', 'ui/brightness'],
  ['levels', 'Levels', 'ui/contrast'],
  ['curves', 'Curves', 'ui/curves'],
  ['hue_saturation', 'Hue/Saturation', 'ui/palette'],
  ['color_balance', 'Colour Balance', 'ui/channels'],
  ['exposure', 'Exposure', 'ui/droplet'],
  ['vibrance', 'Vibrance', 'ui/droplet'],
  ['posterize', 'Posterize', 'ui/threshold'],
  ['gradient_map', 'Gradient Map', 'ui/gradient'],
  ['photo_filter', 'Photo Filter', 'ui/sepia'],
  ['black_and_white', 'Black & White', 'ui/grayscale'],
  ['desaturate', 'Desaturate', 'ui/grayscale'],
  ['auto_tone', 'Auto Tone', 'ui/brightness'],
  ['auto_contrast', 'Auto Contrast', 'ui/contrast'],
  ['invert', 'Invert', 'ui/invert'],
];

function buildAdjustmentsBody() {
  const wrap = el('div', 'image-adjustments');
  const grid = el('div', 'image-adjust-grid');
  for (const [id, label, iconName] of ADJUSTMENT_ITEMS) {
    grid.appendChild(iconButton(iconName, label, () => openAdjustment(id), {
      size: 15,
      className: 'image-adjust-btn',
    }));
  }
  const host = el('div', 'image-inline-host');
  S.els.adjustHost = host;
  wrap.append(grid, host);
  return wrap;
}

/* Properties */
function buildPropertiesBody() {
  const wrap = el('div', 'image-props');
  const blend = select({ options: BLEND_MODES, value: 'normal' });
  blend.select.classList.add('image-prop-select');
  S.els.blendSelect = blend.select;
  blend.select.addEventListener('change', () => withActiveLayer(async () => {
    await api.updateLayer(S.doc.image_id, S.activeLayerId, { blend_mode: blend.select.value });
    await refreshLayers();
    await reloadComposite();
  }));
  wrap.appendChild(smallField('Blend', blend));

  const op = slider({ min: 0, max: 100, step: 1, value: 100 });
  const opVal = el('span', 'image-opt-value', '100%');
  op.addEventListener('input', () => {
    opVal.textContent = `${op.value}%`;
    scheduleOpacity(Number(op.value) / 100);
  });
  const opWrap = el('label', 'image-opt-field image-opt-field--slider');
  opWrap.append(el('span', 'image-opt-label', 'Opacity'), op, opVal);
  S.els.opacitySlider = op;
  S.els.opacityValue = opVal;
  wrap.appendChild(opWrap);

  const posRow = el('div', 'image-prop-pos');
  const xInput = el('input', 'image-opt-number');
  xInput.type = 'number';
  const yInput = el('input', 'image-opt-number');
  yInput.type = 'number';
  const commitPos = async () => {
    if (!S.activeLayerId) return;
    await withActiveLayer(async () => {
      await api.updateLayer(S.doc.image_id, S.activeLayerId, {
        x: Number(xInput.value) || 0,
        y: Number(yInput.value) || 0,
      });
      await refreshLayers();
      await reloadComposite();
    });
  };
  xInput.addEventListener('change', commitPos);
  yInput.addEventListener('change', commitPos);
  posRow.append(smallField('X', xInput), smallField('Y', yInput));
  S.els.posX = xInput;
  S.els.posY = yInput;
  wrap.appendChild(posRow);
  return wrap;
}

/* Color */
function buildColorBody() {
  const wrap = el('div', 'image-color');
  const pair = el('div', 'image-fg-bg');
  const fg = el('button', 'image-fg-swatch');
  fg.type = 'button';
  fg.title = 'Foreground colour';
  const bg = el('button', 'image-bg-swatch');
  bg.type = 'button';
  bg.title = 'Background colour';
  const picker = el('input', 'image-color-input');
  picker.type = 'color';
  picker.value = hexOf(S.fg);
  let pickTarget = 'fg';
  const openPicker = (target) => {
    pickTarget = target;
    picker.value = hexOf(target === 'fg' ? S.fg : S.bg);
    picker.click();
  };
  fg.addEventListener('click', () => openPicker('fg'));
  bg.addEventListener('click', () => openPicker('bg'));
  picker.addEventListener('input', () => {
    const c = parseHex(picker.value);
    if (!c) return;
    if (pickTarget === 'fg') S.fg = c; else S.bg = c;
    refreshPanels();
  });
  fg.style.background = rgbCss(S.fg);
  bg.style.background = rgbCss(S.bg);
  const tools = el('div', 'image-color-tools');
  tools.append(
    iconButton('ui/refresh', 'Default colours', () => { S.fg = [0, 0, 0]; S.bg = [255, 255, 255]; refreshPanels(); }, { size: 13 }),
    iconButton('ui/loop', 'Swap colours', () => { const t = S.fg; S.fg = S.bg; S.bg = t; refreshPanels(); }, { size: 13 }),
  );
  pair.append(fg, tools, bg);
  wrap.append(pair, picker);

  const hex = el('input', 'image-hex');
  hex.type = 'text';
  hex.value = hexOf(S.fg);
  hex.addEventListener('change', () => {
    const c = parseHex(hex.value);
    if (c) { S.fg = c; refreshPanels(); }
  });
  wrap.appendChild(smallField('Hex', hex));

  for (const [i, name] of ['R', 'G', 'B'].entries()) {
    const sl = slider({ min: 0, max: 255, step: 1, value: S.fg[i] });
    const val = el('span', 'image-opt-value', String(S.fg[i]));
    sl.addEventListener('input', () => {
      S.fg[i] = Number(sl.value);
      val.textContent = sl.value;
      fg.style.background = rgbCss(S.fg);
      hex.value = hexOf(S.fg);
    });
    const row = el('label', 'image-opt-field image-opt-field--slider');
    row.append(el('span', 'image-opt-label', name), sl, val);
    wrap.appendChild(row);
  }
  return wrap;
}

/* Swatches */
function buildSwatchesBody() {
  const grid = el('div', 'image-swatches');
  S.els.swatchGrid = grid;
  renderSwatches();
  return grid;
}

function renderSwatches() {
  const grid = S.els.swatchGrid;
  if (!grid) return;
  grid.textContent = '';
  for (const hex of SWATCH_PRESETS) {
    const b = el('button', 'image-swatch');
    b.type = 'button';
    b.style.background = hex;
    b.title = hex;
    b.addEventListener('click', (e) => {
      const c = parseHex(hex);
      if (!c) return;
      if (e.altKey) S.bg = c; else S.fg = c;
      refreshPanels();
    });
    grid.appendChild(b);
  }
}

/* Channels (read-only view of the flattened channels) */
function buildChannelsBody() {
  const wrap = el('div', 'image-channels');
  for (const [label, iconName, checked] of [
    ['RGB', 'ui/eye', true], ['Red', 'ui/eye', true],
    ['Green', 'ui/eye', true], ['Blue', 'ui/eye', true], ['Alpha', 'ui/mask', true],
  ]) {
    const row = el('div', 'image-channel-row');
    const eye = iconButton(iconName, `Toggle ${label}`, () => {}, { size: 13 });
    row.append(eye, el('span', 'image-channel-thumb'), el('span', 'image-channel-name', label));
    if (!checked) row.classList.add('is-off');
    wrap.appendChild(row);
  }
  return wrap;
}

/* History */
function buildHistoryBody() {
  const list = el('div', 'image-history');
  S.els.historyList = list;
  refreshHistory();
  return list;
}

/* Navigator */
function buildNavigatorBody() {
  const wrap = el('div', 'image-navigator');
  const canvas = el('canvas', 'image-nav-canvas');
  canvas.width = 200;
  canvas.height = 130;
  S.els.navCanvas = canvas;
  const zoom = slider({ min: 5, max: 400, step: 1, value: Math.round(S.zoom * 100) });
  zoom.addEventListener('input', () => setZoom(Number(zoom.value) / 100));
  S.els.navZoom = zoom;
  wrap.append(canvas, zoom);
  return wrap;
}

export function refreshPanels() {
  const fg = S.els.dock?.querySelector('.image-fg-swatch');
  const bg = S.els.dock?.querySelector('.image-bg-swatch');
  if (fg) fg.style.background = rgbCss(S.fg);
  if (bg) bg.style.background = rgbCss(S.bg);
  drawNavigator();
}

function drawNavigator() {
  const canvas = S.els.navCanvas;
  if (!canvas || !S.render) return;
  const ctx = canvas.getContext('2d');
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  const { w, h, data } = S.render;
  const tmp = document.createElement('canvas');
  tmp.width = w;
  tmp.height = h;
  tmp.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(data), w, h), 0, 0);
  const scale = Math.min(canvas.width / w, canvas.height / h);
  const dw = w * scale;
  const dh = h * scale;
  ctx.drawImage(tmp, (canvas.width - dw) / 2, (canvas.height - dh) / 2, dw, dh);
  ctx.strokeStyle = 'rgba(0,0,0,0.4)';
  ctx.strokeRect((canvas.width - dw) / 2, (canvas.height - dh) / 2, dw, dh);
}

/* ── Layers ─────────────────────────────────────────────────────── */

export function buildLayersBody() {
  const wrap = el('div', 'image-layers');
  const list = el('div', 'image-layer-list');
  S.els.layerList = list;
  wrap.appendChild(list);
  const foot = el('div', 'image-layers-foot');
  foot.append(
    iconButton('ui/plus', 'New layer', () => addLayer(), { size: 14 }),
    iconButton('ui/folder-plus', 'New group', () => addFolder(), { size: 14 }),
    iconButton('ui/copy', 'Duplicate layer', () => duplicateActive(), { size: 14 }),
    iconButton('ui/chevron-down', 'Merge down', () => mergeDown(), { size: 14 }),
    iconButton('ui/layers', 'Flatten image', () => flattenAll(), { size: 14 }),
    iconButton('ui/trash', 'Delete layer', () => deleteActive(), { size: 14, className: 'image-tool--danger' }),
  );
  wrap.appendChild(foot);
  return wrap;
}

/** Depth-first, top-of-stack first: each group is followed by its contents. */
function displayRows() {
  const out = [];
  const walk = (parentId, depth) => {
    if (depth > 12) return;
    const kids = S.layers
      .filter((l) => (l.group_id || null) === (parentId || null))
      .sort((a, b) => b.position - a.position);
    for (const layer of kids) {
      out.push({ layer, depth });
      if (layer.is_group) walk(layer.layer_id, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

export function renderLayerPanel() {
  const list = S.els.layerList;
  if (!list) return;
  list.textContent = '';
  for (const url of S.thumbs) URL.revokeObjectURL(url);
  S.thumbs = [];

  const rows = displayRows();
  if (!rows.length) {
    list.appendChild(el('div', 'image-layers-empty', 'No layers'));
    syncProperties();
    return;
  }

  for (const { layer, depth } of rows) {
    const row = el('div', 'image-layer-row');
    row.dataset.layerId = layer.layer_id;
    row.draggable = true;
    if (layer.layer_id === S.activeLayerId) row.classList.add('is-active');
    row.style.paddingLeft = `${6 + depth * 14}px`;

    const eye = iconButton('ui/eye', layer.visible ? 'Hide layer' : 'Show layer', async (e) => {
      e.stopPropagation();
      await withActiveLayer(async () => {
        await api.updateLayer(S.doc.image_id, layer.layer_id, { visible: !layer.visible });
        await refreshLayers();
        await reloadComposite();
      }, false);
    }, { size: 13, className: 'image-layer-eye' });
    if (!layer.visible) eye.classList.add('is-eye-off');

    const thumb = el('span', 'image-layer-thumb');
    if (layer.is_group) {
      thumb.classList.add('is-folder');
      const ic = el('span', 'image-icon');
      void setIcon(ic, 'ui/folder', { size: 14 });
      thumb.appendChild(ic);
    } else {
      const img = el('img');
      img.alt = '';
      img.draggable = false;
      thumb.appendChild(img);
      const layerId = layer.layer_id;
      const imageId = S.doc.image_id;
      void (async () => {
        try {
          const blob = await api.fetchLayerThumb(imageId, layerId);
          if (!blob) return;
          const url = URL.createObjectURL(blob);
          S.thumbs.push(url);
          img.src = url;
        } catch (_) { /* thumbnails are best-effort */ }
      })();
    }

    const name = el('span', 'image-layer-name', layer.name || (layer.is_group ? 'Group' : 'Layer'));
    const meta = el('span', 'image-layer-meta');
    if (layer.blend_mode && layer.blend_mode !== 'normal') {
      meta.appendChild(el('span', 'image-layer-blend', layer.blend_mode.replace('_', ' ')));
    }
    if ((layer.opacity ?? 1) < 0.999) {
      meta.appendChild(el('span', 'image-layer-blend', `${Math.round(layer.opacity * 100)}%`));
    }

    row.append(eye, thumb, name, meta);
    row.addEventListener('click', () => {
      S.activeLayerId = layer.layer_id;
      renderLayerPanel();
      syncProperties();
    });
    row.addEventListener('dblclick', () => renameLayer(layer));
    row.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      openLayerMenu(layer, e);
    });

    row.addEventListener('dragstart', (e) => {
      e.dataTransfer.effectAllowed = 'move';
      try { e.dataTransfer.setData('text/plain', layer.layer_id); } catch (_) { /* ignore */ }
    });
    row.addEventListener('dragover', (e) => e.preventDefault());
    row.addEventListener('drop', async (e) => {
      e.preventDefault();
      const draggedId = e.dataTransfer.getData('text/plain');
      if (!draggedId || draggedId === layer.layer_id) return;
      const dragged = S.layers.find((l) => l.layer_id === draggedId);
      if (!dragged || dragged.group_id !== layer.group_id) return;
      const siblings = S.layers
        .filter((l) => l.group_id === dragged.group_id)
        .sort((a, b) => a.position - b.position)
        .map((l) => l.layer_id)
        .filter((id) => id !== draggedId);
      const at = siblings.indexOf(layer.layer_id);
      if (at < 0) return;
      siblings.splice(at, 0, draggedId);
      await withActiveLayer(async () => {
        await api.reorderLayers(S.doc.image_id, siblings);
        await refreshLayers();
        await reloadComposite();
      }, false);
    });

    list.appendChild(row);
  }
  syncProperties();
}

function layerIconButton(iconName, label, fn, danger = false) {
  return iconButton(iconName, label, fn, { size: 13, className: danger ? 'image-tool--danger' : '' });
}

function openLayerMenu(layer, e) {
  openContextMenu([
    { type: 'item', label: 'Rename…', icon: 'ui/pencil', onClick: () => renameLayer(layer) },
    { type: 'item', label: 'Duplicate', icon: 'ui/copy', onClick: () => duplicateActive(layer.layer_id) },
    { type: 'separator' },
    { type: 'heading', label: 'Blend mode' },
    ...BLEND_MODES.map((m) => ({
      type: 'item',
      label: m.replace('_', ' '),
      checked: (layer.blend_mode || 'normal') === m,
      onClick: () => withActiveLayer(async () => {
        await api.updateLayer(S.doc.image_id, layer.layer_id, { blend_mode: m });
        await refreshLayers();
        await reloadComposite();
      }, false),
    })),
    { type: 'separator' },
    { type: 'item', label: 'Merge down', icon: 'ui/chevron-down', disabled: layer.is_group, onClick: () => mergeDown(layer.layer_id) },
    { type: 'item', label: 'Delete layer', icon: 'ui/trash', danger: true, onClick: () => deleteActive(layer.layer_id) },
  ], e.clientX, e.clientY);
}

async function renameLayer(layer) {
  const name = window.prompt('Layer name', layer.name || '');
  if (!name) return;
  await withActiveLayer(async () => {
    await api.updateLayer(S.doc.image_id, layer.layer_id, { name });
    await refreshLayers();
  }, false);
}

async function addLayer() {
  if (!S.doc) return toast('Open an image first', { type: 'error' });
  await withActiveLayer(async () => {
    const created = await api.createLayer(S.doc.image_id, {});
    if (created?.layer_id) S.activeLayerId = created.layer_id;
    await refreshLayers();
    await reloadComposite();
  }, false);
}

async function addFolder() {
  if (!S.doc) return;
  await withActiveLayer(async () => {
    await api.createLayer(S.doc.image_id, { is_group: true, name: 'Group' });
    await refreshLayers();
    await reloadComposite();
  }, false);
}

async function duplicateActive(layerId = S.activeLayerId) {
  if (!layerId) return;
  await withActiveLayer(async () => {
    const created = await api.duplicateLayer(S.doc.image_id, layerId);
    if (created?.layer_id) S.activeLayerId = created.layer_id;
    await refreshLayers();
    await reloadComposite();
  }, false);
}

async function mergeDown(layerId = S.activeLayerId) {
  if (!layerId) return;
  await withActiveLayer(async () => {
    await api.mergeLayer(S.doc.image_id, layerId);
    await refreshLayers();
    await reloadComposite();
  }, false);
}

async function flattenAll() {
  if (!S.doc) return;
  try {
    await api.flattenImage(S.doc.image_id);
    S.activeLayerId = null;
    await refreshLayers();
    await reloadComposite();
    toast('Image flattened', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Could not flatten', { type: 'error' });
  }
}

async function deleteActive(layerId = S.activeLayerId) {
  if (!layerId) return;
  if (!window.confirm('Delete this layer?')) return;
  try {
    await api.deleteLayer(S.doc.image_id, layerId);
    if (S.activeLayerId === layerId) S.activeLayerId = null;
    await refreshLayers();
    await reloadComposite();
  } catch (e) {
    toast(e.message || 'Could not delete layer', { type: 'error' });
  }
}

let opacityTimer = null;

function scheduleOpacity(value) {
  if (!S.activeLayerId) return;
  window.clearTimeout(opacityTimer);
  opacityTimer = window.setTimeout(async () => {
    try {
      await api.updateLayer(S.doc.image_id, S.activeLayerId, { opacity: value });
      await reloadComposite();
      const l = S.layers.find((x) => x.layer_id === S.activeLayerId);
      if (l) l.opacity = value;
    } catch (e) {
      toast(e.message || 'Could not change opacity', { type: 'error' });
    }
  }, 120);
}

export async function refreshLayers() {
  if (!S.doc) {
    S.layers = [];
    S.activeLayerId = null;
    renderLayerPanel();
    return;
  }
  try {
    const r = await api.listLayers(S.doc.image_id);
    S.layers = r?.layers || [];
  } catch (_) {
    S.layers = [];
  }
  const selected = S.layers.find((l) => l.layer_id === S.activeLayerId && !l.is_group);
  if (!selected) {
    const pixels = S.layers.filter((l) => !l.is_group);
    S.activeLayerId = pixels.length ? pixels[pixels.length - 1].layer_id : null;
  }
  renderLayerPanel();
}

function syncProperties() {
  const selected = S.layers.find((l) => l.layer_id === S.activeLayerId) || null;
  if (S.els.blendSelect) {
    S.els.blendSelect.disabled = !selected;
    const mode = selected?.blend_mode;
    S.els.blendSelect.value = BLEND_MODES.includes(mode) ? mode : 'normal';
  }
  if (S.els.opacitySlider) {
    S.els.opacitySlider.disabled = !selected;
    S.els.opacitySlider.value = String(Math.round((selected?.opacity ?? 1) * 100));
  }
  if (S.els.opacityValue) S.els.opacityValue.textContent = `${Math.round((selected?.opacity ?? 1) * 100)}%`;
  if (S.els.posX) S.els.posX.value = String(selected?.x ?? 0);
  if (S.els.posY) S.els.posY.value = String(selected?.y ?? 0);
}

/** Run an action that needs an open document (and optionally an active layer). */
export async function withActiveLayer(fn, needLayer = true) {
  if (!S.doc) {
    toast('Open an image first', { type: 'error' });
    return;
  }
  if (needLayer && !S.activeLayerId) {
    toast('Select a layer first', { type: 'error' });
    return;
  }
  try {
    await fn();
  } catch (e) {
    toast(e.message || 'Action failed', { type: 'error' });
  }
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
  const { view, overlay, area } = S.els;
  if (!view || !overlay || !area) return;
  if (!S.doc || !S.render) {
    view.width = 10;
    view.height = 10;
    overlay.width = 10;
    overlay.height = 10;
    view.style.display = 'none';
    overlay.style.display = 'none';
    renderEmptyState();
    updateStatus();
    drawNavigator();
    return;
  }
  view.style.display = '';
  overlay.style.display = '';
  area.querySelector('.image-empty')?.remove();
  const { w, h } = S.render;
  // First paint after the window is laid out: fit the document to the view.
  if (S.fit && area.clientWidth > 60 && area.clientHeight > 60) {
    const z = Math.min(
      (area.clientWidth - 48) / Math.max(1, w),
      (area.clientHeight - 48) / Math.max(1, h),
      4,
    );
    S.zoom = Math.max(0.05, z);
    if (S.els.navZoom) S.els.navZoom.value = String(Math.round(S.zoom * 100));
  }
  const zw = Math.max(1, Math.round(w * S.zoom));
  const zh = Math.max(1, Math.round(h * S.zoom));
  if (view.width !== zw || view.height !== zh) {
    view.width = zw;
    view.height = zh;
    overlay.width = zw;
    overlay.height = zh;
  }
  const ctx = view.getContext('2d');
  ctx.imageSmoothingEnabled = S.zoom < 1;
  ctx.clearRect(0, 0, zw, zh);
  if (offscreen && offscreen.width) ctx.drawImage(offscreen, 0, 0, zw, zh);
  drawOverlay();
  updateStatus();
  drawRulers();
  drawNavigator();
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
  empty.appendChild(emptyState({ title: 'No image open', body: 'Open or drop an image to start editing.' }));
}

/* ── Zoom & pan ─────────────────────────────────────────────────── */

export function setZoom(z, { focus = null } = {}) {
  const clamped = Math.max(0.05, Math.min(8, z));
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
  if (S.els.navZoom) S.els.navZoom.value = String(Math.round(S.zoom * 100));
}

export function zoomToFit() {
  if (!S.doc || !S.els.area) return;
  const { clientWidth, clientHeight } = S.els.area;
  const z = Math.min(
    (clientWidth - 40) / Math.max(1, S.doc.width),
    (clientHeight - 40) / Math.max(1, S.doc.height),
    4,
  );
  S.zoom = Math.max(0.05, z);
  S.fit = true;
  paintView();
  if (S.els.navZoom) S.els.navZoom.value = String(Math.round(S.zoom * 100));
}

function drawRulers() {
  const { rulerH, rulerV, area } = S.els;
  if (!rulerH || !rulerV || !area || !S.doc) return;
  const ctxH = rulerH.getContext('2d');
  const ctxV = rulerV.getContext('2d');
  const width = Math.max(1, area.clientWidth);
  const height = Math.max(1, area.clientHeight);
  if (rulerH.width !== width) rulerH.width = width;
  if (rulerV.height !== height) rulerV.height = height;
  const css = getComputedStyle(document.documentElement);
  const line = css.getPropertyValue('--glass-border').trim() || '#888';
  const text = css.getPropertyValue('--muted').trim() || '#aaa';
  drawRuler(ctxH, width, 18, 'h', line, text);
  drawRuler(ctxV, 18, height, 'v', line, text);
}

function drawRuler(ctx, w, h, dir, line, text) {
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = line;
  ctx.font = '9px system-ui, sans-serif';
  ctx.fillStyle = text;
  ctx.strokeStyle = line;
  const zoom = S.zoom;
  const scroll = dir === 'h' ? S.els.area.scrollLeft : S.els.area.scrollTop;
  const step = 50;
  for (let doc = 0; doc <= (dir === 'h' ? S.doc.width : S.doc.height) + step; doc += step) {
    const pos = doc * zoom - scroll;
    if (pos < -20 || pos > (dir === 'h' ? w : h) + 20) continue;
    const major = doc % 100 === 0;
    ctx.beginPath();
    if (dir === 'h') {
      ctx.moveTo(pos, major ? 2 : 10);
      ctx.lineTo(pos, h);
    } else {
      ctx.moveTo(major ? 2 : 10, pos);
      ctx.lineTo(w, pos);
    }
    ctx.stroke();
    if (major) {
      if (dir === 'h') ctx.fillText(String(doc), pos + 2, 9);
      else {
        ctx.save();
        ctx.translate(9, pos - 2);
        ctx.rotate(-Math.PI / 2);
        ctx.fillText(String(doc), 0, 0);
        ctx.restore();
      }
    }
  }
}

/* ── Status bar & cursor ────────────────────────────────────────── */

export function updateStatus() {
  const tool = findTool(S.tool);
  if (S.els.statusTool) S.els.statusTool.textContent = tool ? tool.label : '';
  if (S.els.appTitle) S.els.appTitle.textContent = S.doc ? S.doc.title : '';
  if (S.els.statusZoom) S.els.statusZoom.textContent = `${Math.round(S.zoom * 100)}%`;
  if (S.els.statusSize) {
    S.els.statusSize.textContent = S.doc
      ? `${S.doc.width} × ${S.doc.height} px · ${S.layers.filter((l) => !l.is_group).length} layer${S.layers.filter((l) => !l.is_group).length === 1 ? '' : 's'}${S.mask ? ' · selection' : ''}`
      : 'No document';
  }
}

function refreshCursor() {
  const map = {
    move: 'move', hand: 'grab', zoom: 'zoom-in', 'zoom-out': 'zoom-out',
    eyedropper: 'crosshair', bucket: 'crosshair', crop: 'crosshair',
  };
  const c = map[S.tool] || 'crosshair';
  if (S.els.view) S.els.view.style.cursor = c;
}

export function setStatusPos(x, y) {
  if (S.els.statusPos) S.els.statusPos.textContent = `${Math.round(x)}, ${Math.round(y)} px`;
}

/* ── Selection state & overlay ──────────────────────────────────── */

const GEOMETRY_OPS = new Set(['resize', 'image_size', 'crop', 'crop_to_selection', 'rotate', 'flip_h', 'fliph', 'flip_v', 'flipv', 'transform']);

let selectionUploadTimer = null;

export function applySelectionMask(mask, w, h) {
  S.mask = mask;
  S.maskW = w;
  S.maskH = h;
  S.edges = sel.maskEdges(mask, w, h);
  startAnts();
  updateStatus();
  window.clearTimeout(selectionUploadTimer);
  selectionUploadTimer = window.setTimeout(() => {
    void uploadSelection();
  }, 220);
  drawOverlay();
}

async function uploadSelection() {
  if (!S.doc || !S.mask) return;
  try {
    await api.setSelection(S.doc.image_id, S.maskW, S.maskH, S.mask);
  } catch (e) {
    toast(e.message || 'Could not save selection', { type: 'error' });
  }
}

export async function clearSelectionState({ server = true } = {}) {
  S.mask = null;
  S.maskW = 0;
  S.maskH = 0;
  S.edges = [];
  stopAnts();
  updateStatus();
  drawOverlay();
  if (server && S.doc) {
    try { await api.clearSelection(S.doc.image_id); } catch (_) { /* ignore */ }
  }
}

export async function loadSelection() {
  if (!S.doc) return;
  try {
    const r = await api.getSelection(S.doc.image_id);
    if (r?.data && r.width > 0 && r.height > 0) {
      const bin = atob(r.data);
      const mask = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i += 1) mask[i] = bin.charCodeAt(i);
      applySelectionMask(mask, r.width, r.height);
    } else {
      S.mask = null;
      S.edges = [];
      stopAnts();
      drawOverlay();
    }
  } catch (_) { /* no selection */ }
}

export function selectAll() {
  if (!S.doc) return;
  applySelectionMask(sel.maskAll(S.doc.width, S.doc.height), S.doc.width, S.doc.height);
}

export function deselect() {
  void clearSelectionState();
}

export function inverseSelection() {
  if (!S.doc) return;
  const mask = S.mask || sel.newMask(S.doc.width, S.doc.height);
  if (mask.length !== S.doc.width * S.doc.height) {
    applySelectionMask(sel.invertMask(sel.newMask(S.doc.width, S.doc.height)), S.doc.width, S.doc.height);
    return;
  }
  applySelectionMask(sel.invertMask(mask), S.doc.width, S.doc.height);
}

function startAnts() {
  stopAnts();
  S.antsTimer = window.setInterval(() => {
    if (!S.mask) return;
    S.antsPhase = (S.antsPhase + 1) % 8;
    drawOverlay();
  }, 120);
}

function stopAnts() {
  if (S.antsTimer) window.clearInterval(S.antsTimer);
  S.antsTimer = null;
}

export function drawOverlay() {
  const overlay = S.els.overlay;
  if (!overlay) return;
  const ctx = overlay.getContext('2d');
  ctx.clearRect(0, 0, overlay.width, overlay.height);
  if (!S.doc) return;
  const z = S.zoom;
  const accent = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#4f9dff';

  // Marching ants from the boundary segments.
  if (S.mask && S.edges.length) {
    ctx.save();
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 4]);
    ctx.lineDashOffset = -S.antsPhase;
    ctx.strokeStyle = 'rgba(0,0,0,0.8)';
    ctx.beginPath();
    for (const [x0, y0, x1, y1] of S.edges) {
      ctx.moveTo(x0 * z, y0 * z);
      ctx.lineTo(x1 * z, y1 * z);
    }
    ctx.stroke();
    ctx.strokeStyle = '#ffffff';
    ctx.lineDashOffset = -S.antsPhase + 4;
    ctx.stroke();
    ctx.restore();
  }

  // Crop box.
  if (S.cropRect) {
    const { x, y, width, height } = S.cropRect;
    ctx.save();
    ctx.strokeStyle = '#ffffff';
    ctx.lineWidth = 1;
    ctx.setLineDash([5, 3]);
    ctx.strokeRect(x * z, y * z, width * z, height * z);
    ctx.setLineDash([]);
    ctx.strokeStyle = 'rgba(0,0,0,0.6)';
    ctx.strokeRect(x * z + 1, y * z + 1, width * z - 2, height * z - 2);
    // Rule-of-thirds guides + handles.
    ctx.strokeStyle = 'rgba(255,255,255,0.55)';
    for (let i = 1; i < 3; i += 1) {
      ctx.beginPath();
      ctx.moveTo((x + (width * i) / 3) * z, y * z);
      ctx.lineTo((x + (width * i) / 3) * z, (y + height) * z);
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(x * z, (y + (height * i) / 3) * z);
      ctx.lineTo((x + width) * z, (y + (height * i) / 3) * z);
      ctx.stroke();
    }
    ctx.fillStyle = '#ffffff';
    for (const [hx, hy] of handles(x, y, width, height)) {
      ctx.fillRect(hx * z - 4, hy * z - 4, 8, 8);
    }
    ctx.restore();
  }

  // Shape / gradient rubber-band preview.
  if (S.drag && ['shape', 'marquee', 'crop', 'gradient'].includes(S.drag.kind)) {
    const { x0, y0, x1, y1 } = S.drag;
    const rx = Math.min(x0, x1) * z;
    const ry = Math.min(y0, y1) * z;
    const rw = Math.abs(x1 - x0) * z;
    const rh = Math.abs(y1 - y0) * z;
    ctx.save();
    ctx.strokeStyle = accent;
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 3]);
    if (S.drag.kind === 'gradient') {
      ctx.beginPath();
      ctx.moveTo(x0 * z, y0 * z);
      ctx.lineTo(x1 * z, y1 * z);
      ctx.stroke();
    } else if (S.drag.shapeKind === 'ellipse') {
      ctx.beginPath();
      ctx.ellipse(rx + rw / 2, ry + rh / 2, rw / 2, rh / 2, 0, 0, Math.PI * 2);
      ctx.stroke();
    } else {
      ctx.strokeRect(rx, ry, rw, rh);
    }
    ctx.restore();
  }

  // Lasso path preview.
  if (S.drag && S.drag.kind === 'lasso' && S.drag.points.length > 1) {
    ctx.save();
    ctx.strokeStyle = accent;
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 3]);
    ctx.beginPath();
    S.drag.points.forEach(([px, py], i) => {
      if (i === 0) ctx.moveTo(px * z, py * z);
      else ctx.lineTo(px * z, py * z);
    });
    ctx.stroke();
    ctx.restore();
  }

  // Clone source marker.
  if (S.cloneSource) {
    ctx.save();
    ctx.strokeStyle = accent;
    ctx.beginPath();
    ctx.arc(S.cloneSource.x * z, S.cloneSource.y * z, 6, 0, Math.PI * 2);
    ctx.moveTo((S.cloneSource.x - 8) * z, S.cloneSource.y * z);
    ctx.lineTo((S.cloneSource.x + 8) * z, S.cloneSource.y * z);
    ctx.moveTo(S.cloneSource.x * z, (S.cloneSource.y - 8) * z);
    ctx.lineTo(S.cloneSource.x * z, (S.cloneSource.y + 8) * z);
    ctx.stroke();
    ctx.restore();
  }

  // Brush cursor ring.
  if (S.hover && ['brush', 'pencil', 'eraser', 'clone', 'blur-tool', 'sharpen-tool', 'smudge-tool', 'dodge', 'burn'].includes(S.tool)) {
    ctx.save();
    ctx.strokeStyle = 'rgba(255,255,255,0.9)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(S.hover.x * z, S.hover.y * z, (S.toolOptions.size / 2) * z, 0, Math.PI * 2);
    ctx.stroke();
    ctx.restore();
  }
}

function handles(x, y, w, h) {
  return [
    [x, y], [x + w / 2, y], [x + w, y],
    [x, y + h / 2], [x + w, y + h / 2],
    [x, y + h], [x + w / 2, y + h], [x + w, y + h],
  ];
}

/** Encode a raw RGBA buffer (or ImageData) as base64 for a `paste` operation. */
export function base64FromRgba(data) {
  const bytes = data instanceof Uint8ClampedArray || data instanceof Uint8Array
    ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
    : new Uint8Array(data);
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/* ── Apply pipeline ─────────────────────────────────────────────── */

export function queueOp(op, { commit = true, layerId = null } = {}) {
  if (!S.doc) {
    toast('Open an image first', { type: 'error' });
    return;
  }
  S.pending = { op, commit, layerId: layerId ?? S.activeLayerId };
  if (!S.applying) void pump();
}

async function pump() {
  if (!S.doc || S.applying || !S.pending) return;
  S.applying = true;
  const { op, commit, layerId } = S.pending;
  S.pending = null;
  const operations = Array.isArray(op) ? op : [op];
  try {
    const { w, h, buf } = await api.rawApply(S.doc.image_id, operations, { commit, layerId });
    plotRaw(w, h, buf);
    if (commit) {
      recordHistory(operations);
      if (operations.some((o) => GEOMETRY_OPS.has(o.op))) void clearSelectionState({ server: true });
      await refreshLayers();
    }
  } catch (e) {
    toast(e.message || 'Edit failed', { type: 'error' });
  } finally {
    S.applying = false;
    if (S.pending) void pump();
  }
}

/** Run one operation (or a batch) to completion, awaiting the result. */
export async function applyOp(op, { commit = true, refresh = true } = {}) {
  if (!S.doc) {
    toast('Open an image first', { type: 'error' });
    return;
  }
  const operations = Array.isArray(op) ? op : [op];
  try {
    const { w, h, buf } = await api.rawApply(S.doc.image_id, operations, { commit, layerId: S.activeLayerId });
    plotRaw(w, h, buf);
    if (commit) {
      recordHistory(operations);
      if (operations.some((o) => GEOMETRY_OPS.has(o.op))) void clearSelectionState({ server: true });
      if (refresh) await refreshLayers();
    }
  } catch (e) {
    toast(e.message || 'Operation failed', { type: 'error' });
  }
}

function recordHistory(operations) {
  const list = Array.isArray(operations) ? operations : [operations];
  const label = list.map((o) => o.op.replace(/_/g, ' ')).join(' + ');
  S.history.push({ label, at: Date.now() });
  renderHistory();
}

/* ── Pointer plumbing ───────────────────────────────────────────── */

function docPoint(e) {
  const rect = S.els.view.getBoundingClientRect();
  return {
    x: (e.clientX - rect.left) / S.zoom,
    y: (e.clientY - rect.top) / S.zoom,
  };
}

export function wireCanvas(view, overlay, area) {
  overlay.style.touchAction = 'none';

  overlay.addEventListener('pointerdown', (e) => {
    if (e.button === 2) return;
    if (!S.doc) return;
    overlay.setPointerCapture(e.pointerId);
    onPointerDown(e);
  });
  overlay.addEventListener('pointermove', (e) => {
    if (S.doc) {
      const p = docPoint(e);
      setStatusPos(p.x, p.y);
      S.hover = p;
      if (S.drag) onPointerMove(e, p);
      else drawOverlay();
    }
  });
  overlay.addEventListener('pointerleave', () => {
    S.hover = null;
    if (!S.drag) drawOverlay();
  });
  overlay.addEventListener('pointerup', (e) => {
    if (!S.drag) return;
    onPointerUp(e);
    try { overlay.releasePointerCapture(e.pointerId); } catch (_) { /* ignore */ }
  });
  overlay.addEventListener('dblclick', () => {
    if (S.tool === 'lasso-poly' && S.drag?.kind === 'lasso' && S.drag.points.length > 2) {
      finishLasso(true);
    }
  });
  overlay.addEventListener('wheel', (e) => {
    if (e.ctrlKey || e.metaKey || e.altKey) {
      e.preventDefault();
      setZoom(S.zoom * (e.deltaY < 0 ? 1.15 : 1 / 1.15), { focus: e });
    }
  }, { passive: false });

  if (!keyboardWired) {
    window.addEventListener('keydown', onKeyDown);
    keyboardWired = true;
  }
}

let keyboardWired = false;

/* ── Tool behaviour ─────────────────────────────────────────────── */

function onPointerDown(e) {
  const p = docPoint(e);
  const tool = S.tool;
  const alt = e.altKey;

  if (tool === 'hand' || e.button === 1) {
    S.drag = { kind: 'pan', sx: e.clientX, sy: e.clientY, sl: S.els.area.scrollLeft, st: S.els.area.scrollTop };
    return;
  }
  if (tool === 'zoom' || tool === 'zoom-out') {
    S.drag = { kind: 'zoom', p, out: tool === 'zoom-out' };
    return;
  }
  if (tool === 'eyedropper') {
    sampleColor(p);
    return;
  }
  if (tool === 'bucket') {
    queueOp({
      op: 'bucket',
      x: Math.round(p.x),
      y: Math.round(p.y),
      tolerance: S.toolOptions.tolerance,
      contiguous: S.toolOptions.contiguous,
      color: [...S.fg, 255],
      opacity: S.toolOptions.opacity,
    });
    return;
  }
  if (tool === 'move') {
    const layer = S.layers.find((l) => l.layer_id === S.activeLayerId);
    S.drag = { kind: 'move', x0: p.x, y0: p.y, layerX: layer?.x ?? 0, layerY: layer?.y ?? 0 };
    return;
  }
  if (tool === 'marquee-rect' || tool === 'marquee-ellipse') {
    const shape = tool === 'marquee-ellipse' ? 'ellipse' : 'rect';
    S.drag = { kind: 'marquee', x0: p.x, y0: p.y, x1: p.x, y1: p.y, shape, shapeKind: shape };
    return;
  }
  if (tool === 'lasso' || tool === 'lasso-poly') {
    S.drag = { kind: 'lasso', points: [[p.x, p.y]] };
    return;
  }
  if (tool === 'wand') {
    magicWand(p);
    return;
  }
  if (tool === 'crop') {
    S.drag = { kind: 'crop', x0: p.x, y0: p.y, x1: p.x, y1: p.y };
    return;
  }
  if (tool === 'gradient-linear' || tool === 'gradient-radial') {
    S.drag = { kind: 'gradient', x0: p.x, y0: p.y, x1: p.x, y1: p.y };
    return;
  }
  if (tool.startsWith('shape-')) {
    S.drag = { kind: 'shape', shapeKind: tool.replace('shape-', ''), x0: p.x, y0: p.y, x1: p.x, y1: p.y };
    return;
  }
  if (tool === 'type') {
    openTypeTool(p);
    return;
  }
  if (tool === 'clone' && alt) {
    S.cloneSource = p;
    toast('Clone source set', { type: 'info', duration: 1500 });
    return;
  }
  if (['brush', 'pencil', 'eraser', 'clone', 'blur-tool', 'sharpen-tool', 'smudge-tool', 'dodge', 'burn'].includes(tool)) {
    if (tool === 'clone' && !S.cloneSource) {
      toast('Alt-click to set the clone source first', { type: 'error' });
      return;
    }
    S.drag = { kind: 'paint', points: [p], startPoint: p, mode: paintModeFor(tool) };
    return;
  }
}

function onPointerMove(e, p) {
  const d = S.drag;
  if (!d) return;
  if (d.kind === 'pan') {
    S.els.area.scrollLeft = d.sl - (e.clientX - d.sx);
    S.els.area.scrollTop = d.st - (e.clientY - d.sy);
    drawRulers();
    return;
  }
  if (d.kind === 'marquee' || d.kind === 'crop' || d.kind === 'shape' || d.kind === 'gradient') {
    d.x1 = p.x;
    d.y1 = p.y;
    if (e.shiftKey && d.kind === 'marquee') {
      const size = Math.max(Math.abs(d.x1 - d.x0), Math.abs(d.y1 - d.y0));
      d.x1 = d.x0 + Math.sign(d.x1 - d.x0) * size;
      d.y1 = d.y0 + Math.sign(d.y1 - d.y0) * size;
    }
    drawOverlay();
    return;
  }
  if (d.kind === 'lasso') {
    d.points.push([p.x, p.y]);
    drawOverlay();
    return;
  }
  if (d.kind === 'move') {
    d.dx = p.x - d.x0;
    d.dy = p.y - d.y0;
    return;
  }
  if (d.kind === 'paint') {
    const last = d.points[d.points.length - 1];
    const min = Math.max(1, S.toolOptions.size * 0.15);
    if (Math.hypot(p.x - last.x, p.y - last.y) >= min) {
      d.points.push(p);
      if (d.points.length % 3 === 0) previewStroke(d);
    }
  }
}

function onPointerUp() {
  const d = S.drag;
  S.drag = null;
  if (!d) return;
  if (d.kind === 'zoom') {
    setZoom(S.zoom * (d.out ? 1 / 1.5 : 1.5), { focus: null });
    return;
  }
  if (d.kind === 'move') {
    commitMove(d);
    return;
  }
  if (d.kind === 'marquee') {
    finishMarquee(d);
    return;
  }
  if (d.kind === 'lasso') {
    finishLasso(false);
    return;
  }
  if (d.kind === 'crop') {
    S.cropRect = rectFrom(d);
    drawOverlay();
    return;
  }
  if (d.kind === 'gradient') {
    commitGradient(d);
    return;
  }
  if (d.kind === 'shape') {
    commitShape(d);
    return;
  }
  if (d.kind === 'paint') {
    commitStroke(d);
  }
}

function paintModeFor(tool) {
  return {
    brush: 'brush', pencil: 'brush', eraser: 'eraser', clone: 'clone',
    'blur-tool': 'blur', 'sharpen-tool': 'sharpen', 'smudge-tool': 'smudge',
    dodge: 'dodge', burn: 'burn',
  }[tool] || 'brush';
}

function strokeOp(d) {
  const o = S.toolOptions;
  const op = {
    op: 'paint',
    points: d.points.map((p) => [Math.round(p.x * 10) / 10, Math.round(p.y * 10) / 10, 1]),
    radius: Math.max(1, o.size / 2),
    hardness: S.tool === 'pencil' ? 100 : o.hardness,
    opacity: o.opacity,
    flow: o.flow,
    strength: o.strength,
    mode: d.mode,
    color: [...S.fg, 255],
  };
  if (d.mode === 'clone' && S.cloneSource) {
    const s = d.startPoint || d.points[0];
    if (!S.cloneOffset || !o.cloneAligned) {
      S.cloneOffset = [S.cloneSource.x - s.x, S.cloneSource.y - s.y];
    }
    op.offset = S.cloneOffset;
  }
  return op;
}

let strokeTimer = null;

function previewStroke(d) {
  window.clearTimeout(strokeTimer);
  strokeTimer = window.setTimeout(() => {
    if (!S.doc || !d.points.length) return;
    queueOp(strokeOp(d), { commit: false });
  }, 30);
}

function commitStroke(d) {
  window.clearTimeout(strokeTimer);
  if (!S.doc || !d.points.length) return;
  queueOp(strokeOp(d), { commit: true });
}

function commitMove(d) {
  if (!S.activeLayerId || (!d.dx && !d.dy)) return;
  const layer = S.layers.find((l) => l.layer_id === S.activeLayerId);
  if (!layer) return;
  void withActiveLayer(async () => {
    await api.updateLayer(S.doc.image_id, S.activeLayerId, {
      x: Math.round(d.layerX + (d.dx || 0)),
      y: Math.round(d.layerY + (d.dy || 0)),
    });
    await refreshLayers();
    await reloadComposite();
  }, false);
}

function rectFrom(d) {
  const x = Math.min(d.x0, d.x1);
  const y = Math.min(d.y0, d.y1);
  const width = Math.abs(d.x1 - d.x0);
  const height = Math.abs(d.y1 - d.y0);
  return { x, y, width, height };
}

function finishMarquee(d) {
  if (!S.doc) return;
  const r = rectFrom(d);
  if (r.width < 1 || r.height < 1) {
    if (S.toolOptions.marqueeMode === 'new') deselect();
    return;
  }
  const next = sel.maskShape(S.doc.width, S.doc.height, { shape: d.shape, ...r });
  const feather = Math.max(0, S.toolOptions.feather);
  const mask = feather ? sel.featherMask(next, S.doc.width, S.doc.height, feather) : next;
  const combined = sel.combineMask(S.mask, mask, S.doc.width, S.doc.height, S.toolOptions.marqueeMode);
  applySelectionMask(combined, S.doc.width, S.doc.height);
}

function finishLasso(d, fromDouble) {
  const pts = d ? d.points : [];
  if (!S.doc) return;
  if (pts.length > 2 || fromDouble) {
    const next = sel.maskPolygon(S.doc.width, S.doc.height, pts);
    const feather = Math.max(0, S.toolOptions.feather);
    const mask = feather ? sel.featherMask(next, S.doc.width, S.doc.height, feather) : next;
    const combined = sel.combineMask(S.mask, mask, S.doc.width, S.doc.height, S.toolOptions.marqueeMode);
    applySelectionMask(combined, S.doc.width, S.doc.height);
  }
}

function magicWand(p) {
  if (!S.doc || !S.render) return;
  const next = sel.maskWand(S.render.data, S.doc.width, S.doc.height, {
    x: p.x,
    y: p.y,
    tolerance: S.toolOptions.tolerance,
    contiguous: S.toolOptions.contiguous,
  });
  const combined = sel.combineMask(S.mask, next, S.doc.width, S.doc.height, S.toolOptions.marqueeMode);
  applySelectionMask(combined, S.doc.width, S.doc.height);
}

function sampleColor(p) {
  if (!S.render) return;
  const x = Math.max(0, Math.min(S.render.w - 1, Math.round(p.x)));
  const y = Math.max(0, Math.min(S.render.h - 1, Math.round(p.y)));
  const i = (y * S.render.w + x) * 4;
  S.fg = [S.render.data[i], S.render.data[i + 1], S.render.data[i + 2]];
  refreshPanels();
  toast(`Sampled ${hexOf(S.fg)}`, { type: 'info', duration: 1200 });
}

function commitGradient(d) {
  const linear = S.tool === 'gradient-linear';
  queueOp({
    op: 'gradient',
    gradient_type: linear ? 'linear' : 'radial',
    x1: d.x0, y1: d.y0, x2: d.x1, y2: d.y1,
    stops: [
      { pos: 0, color: [...S.bg, 255] },
      { pos: 1, color: [...S.fg, 255] },
    ],
    opacity: S.toolOptions.opacity,
  });
}

function commitShape(d) {
  const r = rectFrom(d);
  if (r.width < 2 || r.height < 2) return;
  const strokeW = S.toolOptions.shapeStroke ?? 0;
  const kinds = { rect: 'rect', ellipse: 'ellipse', line: 'line', polygon: 'polygon', star: 'star' };
  const op = {
    op: 'shape',
    kind: kinds[d.shapeKind] || 'rect',
    x: r.x, y: r.y, width: r.width, height: r.height,
    fill: [...S.fg, 255],
    stroke: strokeW > 0 ? [...S.bg, 255] : null,
    stroke_width: strokeW,
    sides: S.toolOptions.sides,
    points: S.toolOptions.starPoints ?? 5,
    inner_ratio: S.toolOptions.innerRatio,
    opacity: S.toolOptions.opacity,
  };
  if (d.shapeKind === 'line') {
    // A line's "box" runs corner to corner.
    op.x = d.x0;
    op.y = d.y0;
    op.width = d.x1 - d.x0;
    op.height = d.y1 - d.y0;
    op.stroke = [...S.fg, 255];
    op.stroke_width = Math.max(1, S.toolOptions.size / 4);
  }
  queueOp(op);
}

/* ── Type tool ──────────────────────────────────────────────────── */

function openTypeTool(p) {
  const wrap = el('div', 'image-type-panel');
  const area = el('textarea', 'image-type-text');
  area.rows = 2;
  area.placeholder = 'Type here';
  area.value = '';
  const controls = el('div', 'image-type-controls');
  const colorSwatch = el('button', 'image-type-color');
  colorSwatch.type = 'button';
  colorSwatch.style.background = rgbCss(S.fg);
  colorSwatch.title = 'Text colour (foreground)';
  const size = slider({ min: 6, max: 300, step: 1, value: S.toolOptions.fontSize });
  const sizeVal = el('span', 'image-opt-value', String(S.toolOptions.fontSize));
  size.addEventListener('input', () => { sizeVal.textContent = size.value; });
  controls.append(el('span', 'image-opt-label', 'Size'), size, sizeVal, colorSwatch);
  wrap.append(area, controls);

  const dialog = modal({
    title: 'Type',
    body: [wrap, row([
      button({
        label: 'Add to image',
        onClick: async () => {
          const text = area.value;
          dialog.close();
          if (text.trim()) await rasterizeText(text, p, Number(size.value));
        },
      }),
    ])],
  });
  dialog.open();
  requestAnimationFrame(() => area.focus());
}

async function rasterizeText(text, p, fontSize) {
  const lines = text.split('\n');
  const font = `${fontSize}px ${S.toolOptions.font}`;
  const measure = document.createElement('canvas').getContext('2d');
  measure.font = font;
  let width = 0;
  for (const line of lines) width = Math.max(width, measure.measureText(line).width);
  const lineHeight = fontSize * 1.2;
  const pad = Math.ceil(fontSize * 0.2);
  const w = Math.ceil(width) + pad * 2;
  const h = Math.ceil(lineHeight * lines.length) + pad * 2;
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext('2d');
  ctx.font = font;
  ctx.fillStyle = rgbCss(S.fg);
  ctx.textBaseline = 'top';
  lines.forEach((line, i) => ctx.fillText(line, pad, pad + i * lineHeight));
  const data = ctx.getImageData(0, 0, w, h).data;
  queueOp({
    op: 'paste',
    x: Math.round(p.x),
    y: Math.round(p.y),
    width: w,
    height: h,
    data: base64FromRgba(data),
    opacity: 100,
  });
}

/* ── Keyboard ───────────────────────────────────────────────────── */

const TOOL_KEYS = {
  v: 'move', m: 'marquee-rect', l: 'lasso', w: 'wand', c: 'crop', i: 'eyedropper',
  s: 'clone', b: 'brush', e: 'eraser', g: 'bucket', o: 'dodge', t: 'type',
  u: 'shape-rect', h: 'hand', z: 'zoom',
};

function onKeyDown(e) {
  const tag = e.target?.tagName;
  if (tag === 'INPUT' || tag === 'TEXTAREA' || e.target?.isContentEditable) return;
  const mod = e.ctrlKey || e.metaKey;
  if (mod && e.key.toLowerCase() === 'a') { e.preventDefault(); selectAll(); return; }
  if (mod && e.key.toLowerCase() === 'd') { e.preventDefault(); deselect(); return; }
  if (mod && e.shiftKey && e.key.toLowerCase() === 'i') { e.preventDefault(); inverseSelection(); return; }
  if (mod && e.key.toLowerCase() === 'z') { e.preventDefault(); void undo(); return; }
  if (mod && e.shiftKey && e.key.toLowerCase() === 'z') { e.preventDefault(); void redo(); return; }
  if (mod && e.key === '0') { e.preventDefault(); zoomToFit(); return; }
  if (mod && e.key === '1') { e.preventDefault(); setZoom(1); return; }
  if (e.key === 'Enter' && S.cropRect) { e.preventDefault(); commitCrop(); return; }
  if (e.key === 'Escape') {
    S.cropRect = null;
    S.drag = null;
    drawOverlay();
    return;
  }
  if (e.key === 'Delete' || e.key === 'Backspace') {
    e.preventDefault();
    if (S.mask) applyOp({ op: 'clear' });
    return;
  }
  if (e.key === '[') { S.toolOptions.size = Math.max(1, S.toolOptions.size - Math.max(1, Math.round(S.toolOptions.size / 8))); renderOptionsBar(); return; }
  if (e.key === ']') { S.toolOptions.size = Math.min(400, S.toolOptions.size + Math.max(1, Math.round(S.toolOptions.size / 8))); renderOptionsBar(); return; }
  const tool = TOOL_KEYS[e.key.toLowerCase()];
  if (tool && !mod && !e.altKey) selectTool(tool);
}

function commitCrop() {
  const r = S.cropRect;
  if (!r || !S.doc) return;
  S.cropRect = null;
  if (r.width < 1 || r.height < 1) return;
  void documentCrop({
    x: Math.round(r.x),
    y: Math.round(r.y),
    width: Math.round(r.width),
    height: Math.round(r.height),
  }).then(() => { zoomToFit(); });
}

/** Crop the whole document (all layers reflow). */
export async function documentCrop(rect) {
  if (!S.doc) return;
  try {
    await api.cropDocument(S.doc.image_id, rect);
    recordHistory([{ op: 'crop' }]);
    await clearSelectionState({ server: false });
    await refreshLayers();
    await reloadComposite();
  } catch (e) {
    toast(e.message || 'Crop failed', { type: 'error' });
  }
}

async function documentRotate(angle) {
  if (!S.doc) return;
  await withActiveLayer(async () => {
    await api.rotateDocument(S.doc.image_id, angle);
    recordHistory([{ op: 'rotate' }]);
    await clearSelectionState({ server: false });
    await refreshLayers();
    await reloadComposite();
    zoomToFit();
  }, false);
}

async function documentFlip(axis) {
  if (!S.doc) return;
  await withActiveLayer(async () => {
    await api.flipDocument(S.doc.image_id, axis);
    recordHistory([{ op: 'flip' }]);
    await clearSelectionState({ server: false });
    await refreshLayers();
    await reloadComposite();
  }, false);
}

/* ── Document lifecycle ─────────────────────────────────────────── */

export async function reloadComposite() {
  if (!S.doc) return;
  try {
    const { w, h, buf } = await api.fetchRender(S.doc.image_id);
    plotRaw(w, h, buf);
  } catch (e) {
    toast(e.message || 'Could not render image', { type: 'error' });
  }
}

export async function refreshImages() {
  try {
    const r = await api.listImages();
    S.docs = r?.images || [];
  } catch (_) { /* keep the last list */ }
  renderTabs();
}

export async function openImage(meta) {
  S.doc = { image_id: meta.image_id, title: meta.title, width: meta.width, height: meta.height };
  S.mask = null;
  S.edges = [];
  stopAnts();
  S.history = [];
  renderHistory();
  S.fit = true;
  await refreshLayers();
  await reloadComposite();
  await loadSelection();
  renderTabs();
  updateStatus();
}

export async function openNewest() {
  await refreshImages();
  if (S.docs.length) await openImage(S.docs[0]);
  else {
    S.doc = null;
    renderTabs();
    paintView();
  }
}

export function renderTabs() {
  const tabs = S.els.tabs;
  if (!tabs) return;
  tabs.textContent = '';
  for (const d of S.docs) {
    const tab = el('button', 'image-tab');
    tab.type = 'button';
    if (S.doc && d.image_id === S.doc.image_id) tab.classList.add('is-active');
    tab.append(el('span', 'image-tab-name', d.title));
    const close = el('span', 'image-tab-close');
    void setIcon(close, 'ui/close', { size: 11 });
    close.addEventListener('click', (e) => {
      e.stopPropagation();
      void deleteCurrent(d.image_id, d.title);
    });
    tab.append(close);
    tab.addEventListener('click', () => {
      if (!S.doc || d.image_id !== S.doc.image_id) void openImage(d);
    });
    tabs.appendChild(tab);
  }
  const add = iconButton('ui/plus', 'Open image', () => pickFile(), { size: 14 });
  add.classList.add('image-tab-add');
  tabs.appendChild(add);
}

export async function pickFile() {
  const [file] = await pickFiles({ accept: 'image/*' });
  if (file) void uploadFile(file);
}

export async function uploadFile(file) {
  try {
    const created = await api.createImage(file);
    await refreshImages();
    await openImage(created);
    toast(`Opened ${created.title}`, { type: 'info' });
  } catch (e) {
    toast(e.message || 'Upload failed', { type: 'error' });
  }
}

export async function placeEmbedded(file) {
  if (!S.doc) return uploadFile(file);
  await withActiveLayer(async () => {
    const created = await api.uploadLayer(S.doc.image_id, file);
    if (created?.layer_id) S.activeLayerId = created.layer_id;
    await refreshLayers();
    await reloadComposite();
  }, false);
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
    toast(e.message || 'Save failed', { type: 'error' });
  }
}

export async function deleteCurrent(imageId = S.doc?.image_id, title = S.doc?.title) {
  if (!imageId) return;
  if (!window.confirm(`Delete "${title || 'this image'}"?`)) return;
  try {
    await api.deleteImage(imageId);
    if (S.doc?.image_id === imageId) S.doc = null;
    await openNewest();
    toast('Image deleted', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Could not delete image', { type: 'error' });
  }
}

export async function resetCurrent() {
  if (!S.doc) return;
  await applyOp({ op: 'reset' });
  toast('Reverted to original', { type: 'info' });
}

/* ── History (session view; server restore wired in by plugin.js) ─ */

export function renderHistory() {
  const list = S.els.historyList;
  if (!list) return;
  list.textContent = '';
  if (!S.history.length) {
    list.appendChild(el('div', 'image-history-empty', 'No history yet'));
    return;
  }
  S.history.forEach((h, i) => {
    const item = el('button', 'image-history-item');
    item.type = 'button';
    item.append(el('span', 'image-history-label', h.label), el('span', 'image-history-time', new Date(h.at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })));
    if (i === S.history.length - 1) item.classList.add('is-current');
    item.addEventListener('click', () => toast('History restore is not enabled in this build', { type: 'info' }));
    list.appendChild(item);
  });
}

export function refreshHistory() {
  renderHistory();
}

async function undo() {
  toast('Undo is not available in this build', { type: 'info' });
}

async function redo() {
  toast('Redo is not available in this build', { type: 'info' });
}

/* ── Mount / unmount ────────────────────────────────────────────── */

export function mountEditor() {
  const tile = buildWorkspace();
  renderOptionsBar();
  wireDropTarget(S.els.area);
  if (typeof ResizeObserver !== 'undefined' && S.els.area) {
    S.resizeObs = new ResizeObserver(() => {
      if (S.fit) paintView();
      else drawRulers();
    });
    S.resizeObs.observe(S.els.area);
  }
  void openNewest();
  return tile;
}

export function getEditorElement() {
  return S.tile;
}

export function unmountEditor() {
  stopAnts();
  window.removeEventListener('keydown', onKeyDown);
  keyboardWired = false;
  S.inline = null;
  S.resizeObs?.disconnect();
  S.resizeObs = null;
  S.tile?.remove();
  S.tile = null;
  S.els = {};
  S.docs = [];
  S.doc = null;
  S.layers = [];
  S.activeLayerId = null;
  S.mask = null;
  S.edges = [];
  S.render = null;
  S.drag = null;
  S.history = [];
  for (const url of S.thumbs) URL.revokeObjectURL(url);
  S.thumbs = [];
}

/* ── Menus ──────────────────────────────────────────────────────── */

export function openMainMenu(anchor) {
  const rect = anchor?.getBoundingClientRect?.() || { left: 8, bottom: 8 };
  const groups = [
    ['file', 'File'], ['edit', 'Edit'], ['image', 'Image'], ['layer', 'Layer'],
    ['select', 'Select'], ['filter', 'Filter'], ['view', 'View'],
    ['window', 'Window'], ['help', 'Help'],
  ];
  // A flat menu with group headings: the core menu engine supports one level
  // of submenus, so groups that own submenus (Filter, Layer) keep them, but the
  // groups themselves are not nested.
  const entries = [];
  let first = true;
  for (const [key, label] of groups) {
    const items = buildMenu(key);
    if (!items.length) continue;
    if (!first) entries.push({ type: 'separator' });
    first = false;
    entries.push({ type: 'heading', label });
    entries.push(...items);
  }
  if (entries.length) openContextMenu(entries, rect.left, rect.bottom + 3);
}

function buildMenu(name) {
  switch (name) {
    case 'file':
      return [
        { type: 'item', label: 'Open…', icon: 'ui/upload', onClick: pickFile },
        { type: 'item', label: 'Place Embedded…', icon: 'ui/file', disabled: !S.doc, onClick: () => placeEmbeddedFromPicker() },
        { type: 'separator' },
        { type: 'item', label: 'Save a Copy…', icon: 'ui/save', disabled: !S.doc, onClick: () => void downloadCurrent() },
        { type: 'item', label: 'Export as PNG…', icon: 'ui/download', disabled: !S.doc, onClick: () => void downloadCurrent() },
        { type: 'separator' },
        { type: 'item', label: 'Close', disabled: !S.doc, onClick: () => { S.doc = null; paintView(); renderTabs(); } },
        { type: 'item', label: 'Delete Image', icon: 'ui/trash', danger: true, disabled: !S.doc, onClick: () => void deleteCurrent() },
      ];
    case 'edit':
      return [
        { type: 'item', label: 'Undo', icon: 'ui/undo', onClick: () => void undo() },
        { type: 'item', label: 'Redo', icon: 'ui/redo', onClick: () => void redo() },
        { type: 'separator' },
        { type: 'item', label: 'Fill…', icon: 'ui/fill', disabled: !S.doc, onClick: () => applyOp({ op: 'fill', color: [...S.fg, 255], opacity: S.toolOptions.opacity }) },
        { type: 'item', label: 'Stroke…', icon: 'ui/stroke', disabled: !S.doc || !S.mask, onClick: strokeSelection },
        { type: 'item', label: 'Clear', icon: 'ui/eraser', disabled: !S.doc, onClick: () => applyOp({ op: 'clear' }) },
        { type: 'separator' },
        { type: 'item', label: 'Free Transform…', icon: 'ui/move', disabled: !S.doc, onClick: () => openTransformDialog() },
      ];
    case 'image':
      return [
        { type: 'item', label: 'Image Size…', icon: 'ui/image', disabled: !S.doc, onClick: () => openImageSizeDialog() },
        { type: 'heading', label: 'Image Rotation' },
        { type: 'item', label: '90° Clockwise', icon: 'ui/rotate-right', disabled: !S.doc, onClick: () => void documentRotate(90) },
        { type: 'item', label: '90° Counter Clockwise', icon: 'ui/rotate-left', disabled: !S.doc, onClick: () => void documentRotate(-90) },
        { type: 'item', label: '180°', disabled: !S.doc, onClick: () => void documentRotate(180) },
        { type: 'item', label: 'Flip Horizontal', icon: 'ui/flip-h', disabled: !S.doc, onClick: () => void documentFlip('horizontal') },
        { type: 'item', label: 'Flip Vertical', icon: 'ui/flip-v', disabled: !S.doc, onClick: () => void documentFlip('vertical') },
        { type: 'separator' },
        { type: 'item', label: 'Crop to Selection', icon: 'ui/crop', disabled: !S.mask, onClick: cropToSelection },
        { type: 'separator' },
        { type: 'item', label: 'Auto Tone', icon: 'ui/brightness', disabled: !S.doc, onClick: () => applyOp({ op: 'auto_tone' }) },
        { type: 'item', label: 'Auto Contrast', icon: 'ui/contrast', disabled: !S.doc, onClick: () => applyOp({ op: 'auto_contrast' }) },
      ];
    case 'layer':
      return [
        { type: 'item', label: 'New Layer', icon: 'ui/plus', disabled: !S.doc, onClick: () => addLayer() },
        { type: 'item', label: 'New Group', icon: 'ui/folder-plus', disabled: !S.doc, onClick: () => addFolder() },
        { type: 'item', label: 'Duplicate Layer', icon: 'ui/copy', disabled: !S.activeLayerId, onClick: () => duplicateActive() },
        { type: 'item', label: 'Delete Layer', icon: 'ui/trash', danger: true, disabled: !S.activeLayerId, onClick: () => deleteActive() },
        { type: 'separator' },
        { type: 'item', label: 'Merge Down', icon: 'ui/chevron-down', disabled: !S.activeLayerId, onClick: () => mergeDown() },
        { type: 'item', label: 'Flatten Image', icon: 'ui/layers', disabled: !S.doc, onClick: () => flattenAll() },
        { type: 'separator' },
        { type: 'submenu', label: 'Blend Mode', icon: 'ui/layers', items: BLEND_MODES.map((m) => ({
          type: 'item',
          label: m.replace('_', ' '),
          checked: (S.layers.find((l) => l.layer_id === S.activeLayerId)?.blend_mode || 'normal') === m,
          disabled: !S.activeLayerId,
          onClick: () => withActiveLayer(async () => {
            await api.updateLayer(S.doc.image_id, S.activeLayerId, { blend_mode: m });
            await refreshLayers();
            await reloadComposite();
          }),
        })) },
        { type: 'submenu', label: 'New Adjustment', icon: 'ui/fx', items: ADJUSTMENT_ITEMS.map(([id, label]) => ({
          type: 'item',
          label,
          onClick: () => openAdjustment(id),
        })) },
      ];
    case 'select':
      return [
        { type: 'item', label: 'All', icon: 'ui/select-all', disabled: !S.doc, onClick: selectAll },
        { type: 'item', label: 'Deselect', icon: 'ui/deselect', disabled: !S.mask, onClick: deselect },
        { type: 'item', label: 'Inverse', icon: 'ui/inverse', disabled: !S.doc, onClick: inverseSelection },
      ];
    case 'filter':
      return [
        { type: 'submenu', label: 'Blur', items: filterItems(['gaussian_blur', 'box_blur', 'motion_blur', 'radial_blur']) },
        { type: 'submenu', label: 'Sharpen', items: filterItems(['unsharp_mask', 'sharpen_more', 'high_pass']) },
        { type: 'submenu', label: 'Distort', items: filterItems(['twirl', 'ripple', 'wave', 'pinch', 'spherize', 'polar_coordinates']) },
        { type: 'submenu', label: 'Noise', items: filterItems(['noise', 'median', 'despeckle', 'dust']) },
        { type: 'submenu', label: 'Stylize', items: filterItems(['find_edges', 'glowing_edges', 'emboss', 'solarize', 'crystallize', 'fragment']) },
        { type: 'submenu', label: 'Pixelate', items: filterItems(['pixelate', 'mosaic']) },
        { type: 'submenu', label: 'Render', items: filterItems(['clouds', 'vignette']) },
        { type: 'submenu', label: 'Other', items: filterItems(['maximum', 'minimum', 'offset', 'median']) },
        { type: 'separator' },
        { type: 'submenu', label: 'Preset Looks', items: PRESET_FILTERS.map((f) => ({
          type: 'item', label: f, disabled: !S.doc, onClick: () => applyOp({ op: 'filter', name: f }),
        })) },
      ];
    case 'view':
      return [
        { type: 'item', label: 'Zoom In', icon: 'ui/zoom-in', onClick: () => setZoom(S.zoom * 1.25) },
        { type: 'item', label: 'Zoom Out', icon: 'ui/zoom-out', onClick: () => setZoom(S.zoom / 1.25) },
        { type: 'item', label: 'Fit on Screen', icon: 'ui/expand', onClick: zoomToFit },
        { type: 'item', label: 'Actual Pixels', icon: 'ui/grid', onClick: () => setZoom(1) },
      ];
    case 'window':
      return ['adjustments', 'properties', 'color', 'swatches', 'layers', 'channels', 'history', 'navigator'].map((key) => ({
        type: 'item',
        label: key.charAt(0).toUpperCase() + key.slice(1),
        checked: !S.panelCollapsed[key],
        onClick: () => togglePanel(key),
      }));
    case 'help':
      return [
        { type: 'item', label: 'Keyboard Shortcuts', icon: 'ui/keyboard', onClick: () => toast('V move · M marquee · L lasso · B brush · E eraser · G bucket/gradient · T type · U shape · H hand · Z zoom', { type: 'info', duration: 8000 }) },
        { type: 'item', label: 'About Image Editor', icon: 'ui/info', onClick: () => toast('Layered raster editor — adjustments, filters, paint and transforms run on the server engine.', { type: 'info', duration: 6000 }) },
      ];
    default:
      return [];
  }
}

const FILTER_LABELS = {
  gaussian_blur: 'Gaussian Blur…', box_blur: 'Box Blur…', motion_blur: 'Motion Blur…',
  radial_blur: 'Radial Blur…', unsharp_mask: 'Unsharp Mask…', sharpen_more: 'Sharpen More',
  high_pass: 'High Pass…', twirl: 'Twirl…', ripple: 'Ripple…', wave: 'Wave…',
  pinch: 'Pinch…', spherize: 'Spherize…', polar_coordinates: 'Polar Coordinates…',
  noise: 'Add Noise', 'median': 'Median…', despeckle: 'Despeckle', dust: 'Dust & Scratches…',
  find_edges: 'Find Edges', glowing_edges: 'Glowing Edges…', emboss: 'Emboss', solarize: 'Solarize',
  crystallize: 'Crystallize…', fragment: 'Fragment…', pixelate: 'Mosaic…', mosaic: 'Mosaic…',
  clouds: 'Clouds…', vignette: 'Vignette…', maximum: 'Maximum…', minimum: 'Minimum…',
  offset: 'Offset…',
};

function filterItems(ids) {
  return ids.filter((id) => DIALOGS[id] || ['noise', 'emboss', 'solarize', 'find_edges', 'despeckle', 'sharpen_more'].includes(id)).map((id) => ({
    type: 'item',
    label: FILTER_LABELS[id] || id,
    disabled: !S.doc,
    onClick: () => openFilter(id),
  }));
}

function togglePanel(key) {
  S.panelCollapsed[key] = !S.panelCollapsed[key];
  buildDock(S.els.dock);
}

async function placeEmbeddedFromPicker() {
  const [file] = await pickFiles({ accept: 'image/*' });
  if (file) void placeEmbedded(file);
}

/* ── Operation dialogs ──────────────────────────────────────────── */

/** Adjustment/filter specs: `fields` build the controls, `ops(values)` the batch. */
const DIALOGS = {
  brightness_contrast: {
    title: 'Brightness/Contrast',
    fields: [
      { key: 'brightness', type: 'slider', label: 'Brightness', min: -150, max: 150, value: 0 },
      { key: 'contrast', type: 'slider', label: 'Contrast', min: -100, max: 100, value: 0 },
    ],
    ops: (v) => [{ op: 'brightness', amount: v.brightness }, { op: 'contrast', amount: v.contrast }],
  },
  levels: {
    title: 'Levels',
    fields: [
      { key: 'in_black', type: 'slider', label: 'Input Black', min: 0, max: 254, value: 0 },
      { key: 'in_white', type: 'slider', label: 'Input White', min: 1, max: 255, value: 255 },
      { key: 'gamma', type: 'slider', label: 'Gamma', min: 10, max: 300, value: 100, scale: 0.01 },
      { key: 'out_black', type: 'slider', label: 'Output Black', min: 0, max: 255, value: 0 },
      { key: 'out_white', type: 'slider', label: 'Output White', min: 0, max: 255, value: 255 },
    ],
    ops: (v) => [{ op: 'levels', in_black: v.in_black, in_white: v.in_white, gamma: v.gamma * 0.01, out_black: v.out_black, out_white: v.out_white }],
  },
  hue_saturation: {
    title: 'Hue/Saturation',
    fields: [
      { key: 'hue', type: 'slider', label: 'Hue', min: -180, max: 180, value: 0 },
      { key: 'saturation', type: 'slider', label: 'Saturation', min: -100, max: 100, value: 0 },
      { key: 'lightness', type: 'slider', label: 'Lightness', min: -100, max: 100, value: 0 },
      { key: 'colorize', type: 'check', label: 'Colorize', value: false },
    ],
    ops: (v) => [{ op: 'hue_saturation', hue: v.hue, saturation: v.saturation, lightness: v.lightness, colorize: v.colorize }],
  },
  color_balance: {
    title: 'Colour Balance',
    fields: [
      { key: 'cr', type: 'slider', label: 'Cyan — Red', min: -100, max: 100, value: 0 },
      { key: 'mg', type: 'slider', label: 'Magenta — Green', min: -100, max: 100, value: 0 },
      { key: 'yb', type: 'slider', label: 'Yellow — Blue', min: -100, max: 100, value: 0 },
      { key: 'preserve', type: 'check', label: 'Preserve luminosity', value: true },
    ],
    ops: (v) => [{ op: 'color_balance', midtones: [v.cr, v.mg, v.yb], preserve_luminosity: v.preserve }],
  },
  exposure: {
    title: 'Exposure',
    fields: [
      { key: 'exposure', type: 'slider', label: 'Exposure', min: -300, max: 300, value: 0, scale: 0.01 },
      { key: 'offset', type: 'slider', label: 'Offset', min: -50, max: 50, value: 0, scale: 0.01 },
      { key: 'gamma', type: 'slider', label: 'Gamma', min: 10, max: 300, value: 100, scale: 0.01 },
    ],
    ops: (v) => [{ op: 'exposure', exposure: v.exposure * 0.01, offset: v.offset * 0.01, gamma: v.gamma * 0.01 }],
  },
  vibrance: {
    title: 'Vibrance',
    fields: [
      { key: 'vibrance', type: 'slider', label: 'Vibrance', min: -100, max: 100, value: 0 },
      { key: 'saturation', type: 'slider', label: 'Saturation', min: -100, max: 100, value: 0 },
    ],
    ops: (v) => [{ op: 'vibrance', amount: v.vibrance }, { op: 'hue_saturation', saturation: v.saturation }],
  },
  posterize: {
    title: 'Posterize',
    fields: [{ key: 'levels', type: 'slider', label: 'Levels', min: 2, max: 64, value: 6 }],
    ops: (v) => [{ op: 'posterize', levels: v.levels }],
  },
  gradient_map: {
    title: 'Gradient Map',
    fields: [
      { key: 'from', type: 'color', label: 'Shadow', value: [0, 0, 0] },
      { key: 'to', type: 'color', label: 'Highlight', value: [255, 255, 255] },
      { key: 'reverse', type: 'check', label: 'Reverse', value: false },
    ],
    ops: (v) => [{ op: 'gradient_map', colors: [[...v.from, 255], [...v.to, 255]], reverse: v.reverse }],
  },
  photo_filter: {
    title: 'Photo Filter',
    fields: [
      { key: 'color', type: 'color', label: 'Filter', value: [236, 138, 0] },
      { key: 'density', type: 'slider', label: 'Density', min: 0, max: 100, value: 25, unit: '%' },
      { key: 'preserve', type: 'check', label: 'Preserve luminosity', value: true },
    ],
    ops: (v) => [{ op: 'photo_filter', r: v.color[0], g: v.color[1], b: v.color[2], density: v.density, preserve_luminosity: v.preserve }],
  },
  black_and_white: {
    title: 'Black & White',
    fields: [
      { key: 'reds', type: 'slider', label: 'Reds', min: -100, max: 300, value: 40 },
      { key: 'yellows', type: 'slider', label: 'Yellows', min: -100, max: 300, value: 60 },
      { key: 'greens', type: 'slider', label: 'Greens', min: -100, max: 300, value: 40 },
      { key: 'cyans', type: 'slider', label: 'Cyans', min: -100, max: 300, value: 60 },
      { key: 'blues', type: 'slider', label: 'Blues', min: -100, max: 300, value: 20 },
      { key: 'magentas', type: 'slider', label: 'Magentas', min: -100, max: 300, value: 80 },
    ],
    ops: (v) => [{ op: 'black_and_white', reds: v.reds, yellows: v.yellows, greens: v.greens, cyans: v.cyans, blues: v.blues, magentas: v.magentas }],
  },
  curves: {
    title: 'Curves',
    fields: [{ key: 'points', type: 'curve', label: 'Tone curve', value: [[0, 0], [255, 255]] }],
    ops: (v) => [{ op: 'curves', points: v.points }],
  },
  gaussian_blur: {
    title: 'Gaussian Blur',
    fields: [{ key: 'radius', type: 'slider', label: 'Radius', min: 1, max: 100, value: 4, unit: ' px' }],
    ops: (v) => [{ op: 'gaussian_blur', radius: v.radius }],
  },
  box_blur: {
    title: 'Box Blur',
    fields: [{ key: 'radius', type: 'slider', label: 'Radius', min: 1, max: 100, value: 4, unit: ' px' }],
    ops: (v) => [{ op: 'box_blur', radius: v.radius }],
  },
  motion_blur: {
    title: 'Motion Blur',
    fields: [
      { key: 'angle', type: 'slider', label: 'Angle', min: 0, max: 360, value: 0, unit: '°' },
      { key: 'distance', type: 'slider', label: 'Distance', min: 1, max: 100, value: 12, unit: ' px' },
    ],
    ops: (v) => [{ op: 'motion_blur', angle: v.angle, distance: v.distance }],
  },
  radial_blur: {
    title: 'Radial Blur',
    fields: [
      { key: 'amount', type: 'slider', label: 'Amount', min: 1, max: 100, value: 10 },
      { key: 'method', type: 'select', label: 'Method', options: ['spin', 'zoom'], value: 'spin' },
    ],
    ops: (v) => [{ op: 'radial_blur', amount: v.amount, method: v.method }],
  },
  unsharp_mask: {
    title: 'Unsharp Mask',
    fields: [
      { key: 'radius', type: 'slider', label: 'Radius', min: 1, max: 50, value: 1, unit: ' px' },
      { key: 'amount', type: 'slider', label: 'Amount', min: 10, max: 500, value: 100, scale: 0.01, unit: '%' },
      { key: 'threshold', type: 'slider', label: 'Threshold', min: 0, max: 255, value: 0 },
    ],
    ops: (v) => [{ op: 'unsharp_mask', radius: v.radius, amount: v.amount * 0.01, threshold: v.threshold }],
  },
  high_pass: {
    title: 'High Pass',
    fields: [{ key: 'radius', type: 'slider', label: 'Radius', min: 1, max: 50, value: 10, unit: ' px' }],
    ops: (v) => [{ op: 'high_pass', radius: v.radius }],
  },
  twirl: {
    title: 'Twirl',
    fields: [{ key: 'angle', type: 'slider', label: 'Angle', min: -360, max: 360, value: 90, unit: '°' }],
    ops: (v) => [{ op: 'twirl', angle: v.angle }],
  },
  ripple: {
    title: 'Ripple',
    fields: [
      { key: 'amount', type: 'slider', label: 'Amount', min: 1, max: 100, value: 12 },
      { key: 'size', type: 'slider', label: 'Size', min: 1, max: 200, value: 40 },
    ],
    ops: (v) => [{ op: 'ripple', amount: v.amount, size: v.size }],
  },
  wave: {
    title: 'Wave',
    fields: [
      { key: 'amplitude', type: 'slider', label: 'Amplitude', min: 1, max: 60, value: 12 },
      { key: 'wavelength', type: 'slider', label: 'Wavelength', min: 1, max: 200, value: 40 },
      { key: 'phase', type: 'slider', label: 'Phase', min: 0, max: 360, value: 0, unit: '°' },
    ],
    ops: (v) => [{ op: 'wave', amplitude: v.amplitude, wavelength: v.wavelength, phase: v.phase }],
  },
  pinch: {
    title: 'Pinch',
    fields: [{ key: 'amount', type: 'slider', label: 'Amount', min: -100, max: 100, value: 50 }],
    ops: (v) => [{ op: 'pinch', amount: v.amount }],
  },
  spherize: {
    title: 'Spherize',
    fields: [{ key: 'amount', type: 'slider', label: 'Amount', min: -100, max: 100, value: 50 }],
    ops: (v) => [{ op: 'spherize', amount: v.amount }],
  },
  polar_coordinates: {
    title: 'Polar Coordinates',
    fields: [{ key: 'type', type: 'select', label: 'Convert', options: ['polar', 'rect'], value: 'polar' }],
    ops: (v) => [{ op: 'polar_coordinates', type: v.type }],
  },
  median: {
    title: 'Median',
    fields: [{ key: 'radius', type: 'slider', label: 'Radius', min: 1, max: 6, value: 1 }],
    ops: (v) => [{ op: 'median', radius: v.radius }],
  },
  dust: {
    title: 'Dust & Scratches',
    fields: [{ key: 'radius', type: 'slider', label: 'Radius', min: 1, max: 6, value: 2 }],
    ops: (v) => [{ op: 'despeckle', radius: v.radius }],
  },
  maximum: {
    title: 'Maximum',
    fields: [{ key: 'radius', type: 'slider', label: 'Radius', min: 1, max: 20, value: 2 }],
    ops: (v) => [{ op: 'maximum', radius: v.radius }],
  },
  minimum: {
    title: 'Minimum',
    fields: [{ key: 'radius', type: 'slider', label: 'Radius', min: 1, max: 20, value: 2 }],
    ops: (v) => [{ op: 'minimum', radius: v.radius }],
  },
  offset: {
    title: 'Offset',
    fields: [
      { key: 'x', type: 'slider', label: 'Horizontal', min: -500, max: 500, value: 40 },
      { key: 'y', type: 'slider', label: 'Vertical', min: -500, max: 500, value: 40 },
      { key: 'wrap', type: 'check', label: 'Wrap around', value: true },
    ],
    ops: (v) => [{ op: 'offset', x: v.x, y: v.y, wrap: v.wrap }],
  },
  pixelate: {
    title: 'Mosaic',
    fields: [{ key: 'size', type: 'slider', label: 'Cell size', min: 2, max: 200, value: 10, unit: ' px' }],
    ops: (v) => [{ op: 'pixelate', size: v.size }],
  },
  mosaic: {
    title: 'Mosaic',
    fields: [{ key: 'size', type: 'slider', label: 'Cell size', min: 2, max: 200, value: 10, unit: ' px' }],
    ops: (v) => [{ op: 'pixelate', size: v.size }],
  },
  crystallize: {
    title: 'Crystallize',
    fields: [{ key: 'size', type: 'slider', label: 'Cell size', min: 2, max: 200, value: 12, unit: ' px' }],
    ops: (v) => [{ op: 'crystallize', size: v.size }],
  },
  fragment: {
    title: 'Fragment',
    fields: [
      { key: 'x', type: 'slider', label: 'Horizontal', min: 1, max: 30, value: 6 },
      { key: 'y', type: 'slider', label: 'Vertical', min: 1, max: 30, value: 6 },
    ],
    ops: (v) => [{ op: 'fragment', x: v.x, y: v.y }],
  },
  glowing_edges: {
    title: 'Glowing Edges',
    fields: [{ key: 'intensity', type: 'slider', label: 'Intensity', min: 1, max: 100, value: 50 }],
    ops: (v) => [{ op: 'glowing_edges', r: S.fg[0], g: S.fg[1], b: S.fg[2], intensity: v.intensity }],
  },
  clouds: {
    title: 'Clouds',
    fields: [
      { key: 'scale', type: 'slider', label: 'Scale', min: 2, max: 200, value: 64 },
      { key: 'seed', type: 'slider', label: 'Seed', min: 1, max: 999, value: 7 },
    ],
    ops: (v) => [{ op: 'clouds', scale: v.scale, seed: v.seed, r: S.fg[0], g: S.fg[1], b: S.fg[2] }],
  },
  vignette: {
    title: 'Vignette',
    fields: [
      { key: 'amount', type: 'slider', label: 'Amount', min: 0, max: 100, value: 55 },
      { key: 'size', type: 'slider', label: 'Size', min: 0, max: 100, value: 30 },
    ],
    ops: (v) => [{ op: 'vignette', amount: v.amount, size: v.size }],
  },
};

/** One-shot filters with no options. */
const DIRECT_FILTERS = {
  noise: { op: 'noise' },
  emboss: { op: 'emboss' },
  solarize: { op: 'solarize' },
  find_edges: { op: 'find_edges' },
  despeckle: { op: 'despeckle' },
  sharpen_more: { op: 'sharpen_more' },
};

/** One-shot adjustments with no options. */
const DIRECT_ADJUSTMENTS = {
  desaturate: { op: 'desaturate' },
  auto_tone: { op: 'auto_tone' },
  auto_contrast: { op: 'auto_contrast' },
  invert: { op: 'invert' },
};

export function openAdjustment(id) {
  if (DIRECT_ADJUSTMENTS[id]) {
    void applyOp({ ...DIRECT_ADJUSTMENTS[id] });
    return;
  }
  const spec = DIALOGS[id];
  if (spec) openInlinePanel(spec);
}

export function openFilter(id) {
  if (DIRECT_FILTERS[id]) {
    void applyOp({ ...DIRECT_FILTERS[id] });
    return;
  }
  const spec = DIALOGS[id];
  if (spec) openInlinePanel(spec);
}

/** Close the current inline adjustment panel, optionally discarding preview. */
export function closeInlinePanel(revert = false) {
  if (!S.inline) return;
  const { el: panelEl } = S.inline;
  S.inline = null;
  panelEl?.remove();
  if (revert) void reloadComposite();
}

/** Make sure the dock (and the Adjustments panel) is visible before opening. */
function revealAdjustmentsHost() {
  S.tile?.classList.remove('is-dock-hidden');
  S.panelCollapsed.adjustments = false;
  const section = S.els.adjustHost?.closest('.image-panel-section');
  section?.classList.remove('is-collapsed');
}

/**
 * Open an adjustment/filter as an inline panel inside the Adjustments dock —
 * a small window with a title, a close button and live preview, not a modal.
 */
function openInlinePanel(spec) {
  const host = S.els.adjustHost;
  if (!host) {
    openDialog(spec); // fall back to a modal if the dock is unreachable
    return;
  }
  closeInlinePanel(true);
  revealAdjustmentsHost();

  const values = {};
  for (const f of spec.fields) values[f.key] = f.value;

  let previewTimer = null;
  const preview = () => {
    window.clearTimeout(previewTimer);
    previewTimer = window.setTimeout(() => queueOp(spec.ops(values), { commit: false }), 40);
  };

  const panel = el('section', 'image-inline-panel');
  const head = el('div', 'image-inline-head');
  head.appendChild(el('span', 'image-inline-title', spec.title));
  const closeBtn = iconButton('ui/close', 'Close', () => closeInlinePanel(true), { size: 12, className: 'image-inline-close' });
  head.appendChild(closeBtn);

  const body = el('div', 'image-inline-body');
  for (const f of spec.fields) body.appendChild(makeField(f, values, preview));

  const actions = el('div', 'image-inline-actions');
  const reset = button({
    label: 'Reset', variant: 'ghost', size: 'sm',
    onClick: () => { for (const f of spec.fields) values[f.key] = f.value; preview(); },
  });
  const ok = button({
    label: 'Apply', size: 'sm',
    onClick: async () => {
      const ops = spec.ops(values);
      closeInlinePanel(false);
      await applyOp(ops, { commit: true });
    },
  });
  actions.append(reset, ok);

  panel.append(head, body, actions);
  host.appendChild(panel);
  S.inline = { spec, el: panel };
  panel.scrollIntoView({ block: 'nearest' });
}

function makeField(f, values, onChange) {
  const wrap = el('div', 'image-dialog-field');
  wrap.appendChild(el('span', 'image-dialog-label', f.label));

  if (f.type === 'number') {
    const input = el('input', 'image-opt-number');
    input.type = 'number';
    input.value = String(f.value);
    input.addEventListener('change', () => {
      values[f.key] = Number(input.value) || 0;
      onChange();
    });
    wrap.appendChild(input);
  } else if (f.type === 'slider') {
    const sl = slider({ min: f.min, max: f.max, step: f.step ?? 1, value: f.value });
    const val = el('span', 'image-opt-value', `${f.value}${f.unit || ''}`);
    sl.addEventListener('input', () => {
      values[f.key] = Number(sl.value);
      val.textContent = `${sl.value}${f.unit || ''}`;
      onChange();
    });
    wrap.append(sl, val);
  } else if (f.type === 'select') {    const s = select({ options: f.options, value: f.value });
    s.select.addEventListener('change', () => { values[f.key] = s.select.value; onChange(); });
    wrap.appendChild(s);
  } else if (f.type === 'check') {
    const t = toggle({ checked: f.value, onChange: (v) => { values[f.key] = v; onChange(); } });
    wrap.appendChild(t);
  } else if (f.type === 'color') {
    const input = el('input', 'image-dialog-color');
    input.type = 'color';
    input.value = hexOf(f.value);
    input.addEventListener('input', () => {
      const c = parseHex(input.value);
      if (c) { values[f.key] = c; onChange(); }
    });
    wrap.appendChild(input);
  } else if (f.type === 'curve') {
    const canvas = el('canvas', 'image-curves-canvas');
    canvas.width = 240;
    canvas.height = 160;
    wrap.appendChild(canvas);
    const state = { points: f.value.map((p) => [...p]), drag: -1 };
    const pad = 12;
    const toPx = (x) => pad + (x / 255) * (canvas.width - pad * 2);
    const toPy = (y) => canvas.height - pad - (y / 255) * (canvas.height - pad * 2);
    const draw = () => {
      const ctx = canvas.getContext('2d');
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.strokeStyle = 'rgba(128,128,128,0.35)';
      for (let i = 0; i <= 4; i += 1) {
        const x = pad + (i * (canvas.width - pad * 2)) / 4;
        const y = pad + (i * (canvas.height - pad * 2)) / 4;
        ctx.beginPath(); ctx.moveTo(x, pad); ctx.lineTo(x, canvas.height - pad); ctx.stroke();
        ctx.beginPath(); ctx.moveTo(pad, y); ctx.lineTo(canvas.width - pad, y); ctx.stroke();
      }
      ctx.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#4f9dff';
      ctx.lineWidth = 2;
      ctx.beginPath();
      const lut = curveLut(state.points);
      for (let x = 0; x <= 255; x += 1) {
        const px = toPx(x);
        const py = toPy(lut[x]);
        if (x === 0) ctx.moveTo(px, py); else ctx.lineTo(px, py);
      }
      ctx.stroke();
      for (const [x, y] of state.points) {
        ctx.fillStyle = '#fff';
        ctx.beginPath();
        ctx.arc(toPx(x), toPy(y), 4, 0, Math.PI * 2);
        ctx.fill();
      }
    };
    const fromEvent = (e) => {
      const r = canvas.getBoundingClientRect();
      const cx = (e.clientX - r.left) * (canvas.width / r.width);
      const cy = (e.clientY - r.top) * (canvas.height / r.height);
      return [
        Math.max(0, Math.min(255, ((cx - pad) / (canvas.width - pad * 2)) * 255)),
        Math.max(0, Math.min(255, (1 - (cy - pad) / (canvas.height - pad * 2)) * 255)),
      ];
    };
    canvas.addEventListener('pointerdown', (e) => {
      const [x, y] = fromEvent(e);
      let best = -1;
      let bestD = Infinity;
      state.points.forEach((p, i) => {
        const d = (p[0] - x) ** 2 + (p[1] - y) ** 2;
        if (d < bestD) { bestD = d; best = i; }
      });
      if (bestD > 400) {
        state.points.push([x, y]);
        state.points.sort((a, b) => a[0] - b[0]);
        best = state.points.findIndex((p) => p[0] === x && p[1] === y);
      }
      state.drag = best;
      canvas.setPointerCapture(e.pointerId);
      draw();
    });
    canvas.addEventListener('pointermove', (e) => {
      if (state.drag < 0) return;
      const [x, y] = fromEvent(e);
      state.points[state.drag] = [x, y];
      values[f.key] = state.points.map((p) => [Math.round(p[0]), Math.round(p[1])]).sort((a, b) => a[0] - b[0]);
      draw();
      onChange();
    });
    canvas.addEventListener('pointerup', () => { state.drag = -1; });
    draw();
    values[f.key] = f.value;
  }
  return wrap;
}

/** Build a 256-entry tone-curve LUT with monotone cubic interpolation. */
function curveLut(points) {
  const lut = new Array(256);
  const pts = points.slice().sort((a, b) => a[0] - b[0]);
  const n = pts.length;
  const xs = pts.map((p) => p[0]);
  const ys = pts.map((p) => p[1]);
  const clamp = (v) => Math.max(0, Math.min(255, v));
  if (n < 2) { for (let i = 0; i < 256; i += 1) lut[i] = i; return lut; }
  const d = new Array(n - 1);
  for (let i = 0; i < n - 1; i += 1) {
    const dx = xs[i + 1] - xs[i];
    d[i] = Math.abs(dx) < 1e-9 ? 0 : (ys[i + 1] - ys[i]) / dx;
  }
  const m = new Array(n).fill(0);
  if (n === 2) { m[0] = d[0]; m[1] = d[0]; }
  else {
    m[0] = d[0]; m[n - 1] = d[n - 2];
    for (let i = 1; i < n - 1; i += 1) {
      if (d[i - 1] * d[i] <= 0) m[i] = 0;
      else {
        const hp = xs[i] - xs[i - 1];
        const hn = xs[i + 1] - xs[i];
        const w1 = 2 * hn + hp;
        const w2 = hn + 2 * hp;
        m[i] = (w1 + w2) / (w1 / d[i - 1] + w2 / d[i]);
      }
    }
  }
  for (let x = 0; x < 256; x += 1) lut[x] = x;
  for (let seg = 0; seg < n - 1; seg += 1) {
    const x0 = xs[seg]; const x1 = xs[seg + 1];
    const y0 = ys[seg]; const y1 = ys[seg + 1];
    const h = x1 - x0;
    if (Math.abs(h) < 1e-9) continue;
    const i0 = Math.max(0, Math.round(x0));
    const i1 = Math.min(255, Math.round(x1));
    for (let x = i0; x <= i1; x += 1) {
      const t = (x - x0) / h; const t2 = t * t; const t3 = t2 * t;
      const h00 = 2 * t3 - 3 * t2 + 1;
      const h10 = t3 - 2 * t2 + t;
      const h01 = -2 * t3 + 3 * t2;
      const h11 = t3 - t2;
      lut[x] = clamp(h00 * y0 + h10 * h * m[seg] + h01 * y1 + h11 * h * m[seg + 1]);
    }
  }
  return lut;
}

function openDialog(spec) {
  const values = {};
  for (const f of spec.fields) values[f.key] = f.value;

  let previewTimer = null;
  const preview = () => {
    window.clearTimeout(previewTimer);
    previewTimer = window.setTimeout(() => queueOp(spec.ops(values), { commit: false }), 40);
  };

  const controls = spec.fields.map((f) => makeField(f, values, preview));
  const reset = button({
    label: 'Reset', variant: 'ghost',
    onClick: () => { for (const f of spec.fields) values[f.key] = f.value; preview(); },
  });
  const cancel = button({
    label: 'Cancel', variant: 'ghost',
    onClick: () => { dialog.close(); void reloadComposite(); },
  });
  const ok = button({
    label: 'OK',
    onClick: async () => {
      dialog.close();
      await applyOp(spec.ops(values), { commit: true });
    },
  });

  const dialog = modal({
    title: spec.title,
    body: [...controls, row([reset, cancel, ok], { className: 'image-dialog-actions' })],
  });
  dialog.open();
}

/* ── Transform & image size ─────────────────────────────────────── */

function openTransformDialog() {
  const spec = {
    title: 'Free Transform',
    fields: [
      { key: 'rotate', type: 'slider', label: 'Rotate', min: -180, max: 180, value: 0, unit: '°' },
      { key: 'scale_x', type: 'slider', label: 'Scale X', min: 10, max: 300, value: 100, unit: '%' },
      { key: 'scale_y', type: 'slider', label: 'Scale Y', min: 10, max: 300, value: 100, unit: '%' },
      { key: 'skew_x', type: 'slider', label: 'Skew X', min: -45, max: 45, value: 0, unit: '°' },
      { key: 'move_x', type: 'slider', label: 'Move X', min: -500, max: 500, value: 0, unit: ' px' },
      { key: 'move_y', type: 'slider', label: 'Move Y', min: -500, max: 500, value: 0, unit: ' px' },
    ],
    ops: (v) => [{
      op: 'transform',
      rotate: v.rotate,
      scale_x: v.scale_x / 100,
      scale_y: v.scale_y / 100,
      skew_x: v.skew_x,
      move_x: v.move_x,
      move_y: v.move_y,
    }],
  };
  openDialog(spec);
}

function strokeSelection() {
  if (!S.doc || !S.mask) {
    toast('Make a selection first', { type: 'error' });
    return;
  }
  const w = S.doc.width;
  const h = S.doc.height;
  const width = Math.max(1, Math.min(20, Math.round(S.toolOptions.shapeStroke || 1)));
  const mask = S.mask;
  if (mask.length !== w * h) {
    toast('Selection is stale — reselect', { type: 'error' });
    return;
  }
  const inside = (x, y) => mask[y * w + x] > 127;
  let cur = new Uint8Array(w * h);
  for (let y = 0; y < h; y += 1) {
    for (let x = 0; x < w; x += 1) {
      if (!inside(x, y)) continue;
      const edge = x === 0 || y === 0 || x === w - 1 || y === h - 1
        || !inside(x - 1, y) || !inside(x + 1, y) || !inside(x, y - 1) || !inside(x, y + 1);
      if (edge) cur[y * w + x] = 1;
    }
  }
  for (let k = 1; k < width; k += 1) {
    const next = new Uint8Array(w * h);
    for (let y = 0; y < h; y += 1) {
      for (let x = 0; x < w; x += 1) {
        const i = y * w + x;
        if (cur[i]) { next[i] = 1; continue; }
        if ((x > 0 && cur[i - 1]) || (x < w - 1 && cur[i + 1])
          || (y > 0 && cur[i - w]) || (y < h - 1 && cur[i + w])) next[i] = 1;
      }
    }
    cur = next;
  }
  const rgba = new Uint8ClampedArray(w * h * 4);
  for (let i = 0; i < w * h; i += 1) {
    if (!cur[i]) continue;
    rgba[i * 4] = S.fg[0];
    rgba[i * 4 + 1] = S.fg[1];
    rgba[i * 4 + 2] = S.fg[2];
    rgba[i * 4 + 3] = 255;
  }
  queueOp({ op: 'paste', x: 0, y: 0, width: w, height: h, data: base64FromRgba(rgba), opacity: 100 });
}

function cropToSelection() {
  if (!S.doc || !S.mask) return;
  const bounds = sel.maskBounds(S.mask, S.doc.width, S.doc.height);
  if (!bounds) return;
  void documentCrop({
    x: Math.round(bounds.x),
    y: Math.round(bounds.y),
    width: Math.round(bounds.width),
    height: Math.round(bounds.height),
  }).then(() => zoomToFit());
}

function openImageSizeDialog() {
  if (!S.doc) return;
  const wInput = el('input', 'image-opt-number');
  wInput.type = 'number';
  wInput.value = String(S.doc.width);
  const hInput = el('input', 'image-opt-number');
  hInput.type = 'number';
  hInput.value = String(S.doc.height);
  const body = [
    row([smallField('Width', wInput), smallField('Height', hInput)], { className: 'image-prop-pos' }),
  ];
  const dialog = modal({
    title: 'Image Size',
    body: [...body, row([
      button({ label: 'Cancel', variant: 'ghost', onClick: () => dialog.close() }),
      button({
        label: 'OK',
        onClick: async () => {
          dialog.close();
          const w = Math.max(1, Number(wInput.value) || S.doc.width);
          const h = Math.max(1, Number(hInput.value) || S.doc.height);
          await withActiveLayer(async () => {
            await api.resizeDocument(S.doc.image_id, w, h);
            recordHistory([{ op: 'image_size' }]);
            await clearSelectionState({ server: false });
            await refreshLayers();
            await reloadComposite();
            zoomToFit();
          }, false);
        },
      }),
    ], { className: 'image-dialog-actions' })],
  });
  dialog.open();
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
    if (!blob || blob.size === 0) throw new Error('that file is empty');
    const type = blob.type || '';
    const ext = String(name || '').toLowerCase().split('.').pop();
    const knownImage = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'].includes(ext);
    if (type && !type.startsWith('image/') && !knownImage) {
      throw new Error(`“${name || 'file'}” is not an image the editor can open`);
    }
    const file = new File([blob], name || 'image', { type: type || 'image/png' });
    await uploadFile(file);
  } catch (e) {
    toast(e.message || 'Could not open file', { type: 'error' });
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
      if (found) await openImage(found);
    } else if (S.doc) {
      await reloadComposite();
      await refreshLayers();
      await loadSelection();
    }
  })();
}

export function editorContextMenu() {
  const hasDoc = !!S.doc;
  const hasLayer = hasDoc && !!S.activeLayerId;
  return [
    { type: 'item', label: 'Open image…', icon: 'ui/upload', onClick: pickFile },
    { type: 'item', label: 'New layer', icon: 'ui/plus', disabled: !hasDoc, onClick: () => addLayer() },
    { type: 'item', label: 'New group', icon: 'ui/folder-plus', disabled: !hasDoc, onClick: () => addFolder() },
    { type: 'item', label: 'Duplicate layer', icon: 'ui/copy', disabled: !hasLayer, onClick: () => duplicateActive() },
    { type: 'item', label: 'Merge down', icon: 'ui/chevron-down', disabled: !hasLayer, onClick: () => mergeDown() },
    { type: 'item', label: 'Flatten image', icon: 'ui/layers', disabled: !hasDoc, onClick: () => flattenAll() },
    { type: 'separator' },
    { type: 'submenu', label: 'Adjustments', icon: 'ui/fx', items: ADJUSTMENT_ITEMS.map(([id, label]) => ({ type: 'item', label, onClick: () => openAdjustment(id) })) },
    { type: 'separator' },
    { type: 'item', label: 'Select all', icon: 'ui/select-all', disabled: !hasDoc, onClick: selectAll },
    { type: 'item', label: 'Deselect', icon: 'ui/deselect', disabled: !S.mask, onClick: deselect },
    { type: 'separator' },
    { type: 'item', label: 'Revert to original', icon: 'ui/refresh', disabled: !hasLayer, onClick: () => void resetCurrent() },
    { type: 'item', label: 'Delete layer', icon: 'ui/trash', danger: true, disabled: !hasLayer, onClick: () => deleteActive() },
    { type: 'item', label: 'Delete image', icon: 'ui/trash', danger: true, disabled: !hasDoc, onClick: () => void deleteCurrent() },
  ];
}

/* Drag an image file onto the canvas to place it in a new layer. */
export function wireDropTarget(area) {
  if (!area) return;
  area.addEventListener('dragover', (e) => { e.preventDefault(); area.classList.add('is-drop'); });
  area.addEventListener('dragleave', () => area.classList.remove('is-drop'));
  area.addEventListener('drop', (e) => {
    e.preventDefault();
    area.classList.remove('is-drop');
    const file = [...(e.dataTransfer?.files || [])].find((f) => f.type.startsWith('image/'));
    if (!file) return;
    if (S.doc) void placeEmbedded(file); else void uploadFile(file);
  });
}

