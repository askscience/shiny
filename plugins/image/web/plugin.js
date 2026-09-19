/**
 * image.js — the Image plugin's window (photo editor, Photon-backed).
 *
 * Photoshop-inspired layout: hairline top bar, left tool rail, centered
 * canvas, right adjustments panel (brightness/contrast, a draggable Curves
 * editor, effects grid, preset filters). All controls are icon buttons.
 *
 * REAL-TIME: every edit is computed by the plugin's Rust engine (photon-rs),
 * never by CSS. The window streams operations to `POST /api/images/:id/apply
 * ?raw=1` — the server mutates in-memory raw RGBA pixels (no PNG codec) and
 * streams the pixels straight back, which are drawn to a canvas. Dragging a
 * slider/curve previews without committing (`commit=0`); releasing commits
 * the pixels to SQLite.
 *
 * AI wiring: `image_*` outcomes arrive via `agent:actions`; the window
 * refreshes its list and re-opens the image the AI touched.
 */

import { button, emptyState, glowFromDrawable, select, setTileGlow, slider, toast } from '/ui/index.js';
import { setIcon } from '/ui/index.js';
import { apiFetch, getToken } from '/js/api.js';
import { saveOrDownload, onOpenFromFiles, fileFromHome, pickFiles } from '/js/files.js';

export const IMAGE_PLUGIN = 'image';

const FILTERS = [
  'oceanic', 'islands', 'marine', 'seagreen', 'flagblue', 'diamante', 'liquid',
  'radio', 'twenties', 'rosetint', 'mauve', 'bluechrome', 'vintage', 'perfume',
  'serenity', 'golden', 'pastel_pink', 'cali', 'dramatic', 'firenze', 'obsidian', 'lofi',
];

const EFFECTS = [
  ['ui/grayscale', 'Grayscale', { op: 'grayscale' }],
  ['ui/sepia', 'Sepia', { op: 'sepia' }],
  ['ui/invert', 'Invert', { op: 'invert' }],
  ['ui/sharpen', 'Sharpen', { op: 'sharpen' }],
  ['ui/blur', 'Blur', { op: 'blur', radius: 4 }],
  ['ui/edge', 'Edge', { op: 'edge' }],
  ['ui/emboss', 'Emboss', { op: 'emboss' }],
  ['ui/noise', 'Noise', { op: 'noise' }],
  ['ui/solarize', 'Solarize', { op: 'solarize' }],
  ['ui/threshold', 'Threshold', { op: 'threshold', amount: 128 }],
];

const TRANSFORMS = [
  ['ui/rotate-left', 'Rotate −90°', { op: 'rotate', angle: -90 }],
  ['ui/rotate-right', 'Rotate +90°', { op: 'rotate', angle: 90 }],
  ['ui/flip-h', 'Flip horizontal', { op: 'flip_h' }],
  ['ui/flip-v', 'Flip vertical', { op: 'flip_v' }],
];

let tileEl = null;
let imageMenuBtn = null;
let titleInput = null;
let statusEl = null;
let docMetaEl = null;
let saveDot = null;
let stageEl = null;
let canvasEl = null;
let canvasCtx = null;

/* Debounces the ambient-glow refresh while a slider/curve streams frames. */
let glowTimer = null;

let images = [];
let current = null;   // { image_id, title, width, height }
let busy = false;

/* Layer stack (bottom-to-top) + selection */
let layers = [];
let activeLayerId = null;
let layerListEl = null;
let layerBlendEl = null;
let layerOpacityEl = null;
let layerOpacityValueEl = null;
let dragLayerId = null;
let thumbUrls = [];

const BLEND_MODES = [
  'normal', 'multiply', 'screen', 'overlay', 'darken', 'lighten',
  'difference', 'color_dodge', 'color_burn', 'add', 'subtract',
];

/* Real-time apply queue — at most one in-flight, always send the latest. */
let pending = null;
let applying = false;

/* Curves editor state */
let curveCanvas = null;
let curvePoints = [[0, 0], [255, 255]];
let curveDrag = -1;

/* Image menu popup (body-level) */
let imageMenuPopup = null;
let imageMenuOpen = false;

/* ── API ────────────────────────────────────────────────────── */

async function api(path, options = {}) {
  const res = await apiFetch(path, options);
  return res?.data ?? null;
}

function listImages() { return api('/api/images'); }
function fetchImage(id) { return api(`/api/images/${encodeURIComponent(id)}`); }
function deleteImage(id) {
  return api(`/api/images/${encodeURIComponent(id)}`, { method: 'DELETE' });
}
function renameImage(id, title) {
  return api(`/api/images/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify({ title }),
  });
}
async function createImage(file) {
  const form = new FormData();
  form.append('file', file);
  const res = await apiFetch('/api/images', { method: 'POST', body: form });
  return res?.data ?? null;
}

function listLayers(id) { return api(`/api/images/${encodeURIComponent(id)}/layers`); }
function createLayer(id, body) {
  return api(`/api/images/${encodeURIComponent(id)}/layers`, {
    method: 'POST',
    body: JSON.stringify(body || {}),
  });
}
function updateLayer(id, layerId, patch) {
  return api(`/api/images/${encodeURIComponent(id)}/layers/${encodeURIComponent(layerId)}`, {
    method: 'PUT',
    body: JSON.stringify(patch),
  });
}
function deleteLayer(id, layerId) {
  return api(`/api/images/${encodeURIComponent(id)}/layers/${encodeURIComponent(layerId)}`, {
    method: 'DELETE',
  });
}
function duplicateLayer(id, layerId) {
  return api(`/api/images/${encodeURIComponent(id)}/layers/${encodeURIComponent(layerId)}/duplicate`, { method: 'POST' });
}
function mergeLayer(id, layerId) {
  return api(`/api/images/${encodeURIComponent(id)}/layers/${encodeURIComponent(layerId)}/merge`, { method: 'POST' });
}
function reorderLayers(id, ids) {
  return api(`/api/images/${encodeURIComponent(id)}/layers/reorder`, {
    method: 'POST',
    body: JSON.stringify({ ids }),
  });
}
function flattenImage(id) {
  return api(`/api/images/${encodeURIComponent(id)}/flatten`, { method: 'POST' });
}

/** Fetch the flattened composite as raw RGBA. */
async function fetchRender(id) {
  const token = getToken();
  const headers = {};
  if (token) headers.Authorization = `Bearer ${token}`;
  const res = await fetch(
    `/api/images/${encodeURIComponent(id)}/render?raw=true`,
    { headers },
  );
  if (!res.ok) throw new Error(res.statusText || 'Render failed');
  const w = Number(res.headers.get('x-image-width') || 0);
  const h = Number(res.headers.get('x-image-height') || 0);
  const buf = await res.arrayBuffer();
  return { w, h, buf };
}

/** Apply operations server-side (Rust) and stream the composited RGBA back. */
async function rawApply(id, operations, commit, layerId) {
  const token = getToken();
  const headers = { 'Content-Type': 'application/json' };
  if (token) headers.Authorization = `Bearer ${token}`;
  const payload = { operations };
  if (layerId) payload.layer_id = layerId;
  const res = await fetch(
    `/api/images/${encodeURIComponent(id)}/apply?raw=true&commit=${commit ? 'true' : 'false'}`,
    { method: 'POST', headers, body: JSON.stringify(payload) },
  );
  if (!res.ok) {
    const text = await res.text();
    let msg = text;
    try { msg = JSON.parse(text).error || msg; } catch (_) { /* keep text */ }
    throw new Error(msg || res.statusText);
  }
  const w = Number(res.headers.get('x-image-width') || 0);
  const h = Number(res.headers.get('x-image-height') || 0);
  const buf = await res.arrayBuffer();
  return { w, h, buf };
}

/* ── Theme helpers ──────────────────────────────────────────── */

function cssVar(name, fallback = '') {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

/* ── State / status ─────────────────────────────────────────── */

function setStatus(mode) {
  if (!statusEl) return;
  const now = new Date();
  const t = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const dims = current ? `${current.width} × ${current.height} px` : 'No image';
  const layerLabel = current
    ? `${layers.filter((l) => !l.is_group).length} layer${layers.filter((l) => !l.is_group).length === 1 ? '' : 's'}`
    : '';
  if (docMetaEl) {
    docMetaEl.textContent = current ? `${dims} · RGB · ${layerLabel}` : '—';
  }
  if (mode === 'saving') {
    statusEl.textContent = 'Applying…';
    saveDot?.classList.add('is-active');
  } else if (mode === 'dirty') {
    statusEl.textContent = 'Unsaved title';
    saveDot?.classList.add('is-active');
  } else {
    statusEl.textContent = `Saved ${t} · ${dims}`;
    saveDot?.classList.remove('is-active');
  }
}

/* ── Canvas rendering ───────────────────────────────────────── */

/** Mirror the edited photo into the window's ambient glow, debounced. */
function refreshGlowSoon() {
  window.clearTimeout(glowTimer);
  glowTimer = window.setTimeout(() => {
    if (!current || !canvasEl) { setTileGlow(tileEl, null); return; }
    setTileGlow(tileEl, glowFromDrawable(canvasEl));
  }, 120);
}

function renderStage() {
  if (!stageEl) return;
  stageEl.textContent = '';
  if (!current) {
    stageEl.appendChild(emptyState({ title: 'No image open', body: 'Upload an image to start editing.' }));
    canvasEl = null;
    canvasCtx = null;
    setTileGlow(tileEl, null);
    refreshGlowSoon();
    return;
  }
  canvasEl = document.createElement('canvas');
  canvasEl.className = 'image-stage-img';
  canvasCtx = canvasEl.getContext('2d');
  stageEl.appendChild(canvasEl);
}

function renderRaw(w, h, buf) {
  if (!canvasEl || !canvasCtx) return;
  canvasEl.width = w;
  canvasEl.height = h;
  const data = new Uint8ClampedArray(buf);
  canvasCtx.putImageData(new ImageData(data, w, h), 0, 0);
  refreshGlowSoon();
}

async function loadPixels() {
  if (!current || !canvasCtx) return;
  const { w, h, buf } = await fetchRender(current.image_id);
  current.width = w;
  current.height = h;
  renderRaw(w, h, buf);
  setStatus('saved');
}

function renderTitle() {
  if (titleInput) titleInput.value = current?.title || '';
}

function resetCurve() {
  curvePoints = [[0, 0], [255, 255]];
  drawCurve();
}

async function openImage(meta) {
  current = {
    image_id: meta.image_id,
    title: meta.title,
    width: meta.width,
    height: meta.height,
  };
  resetCurve();
  renderTitle();
  renderStage();
  activeLayerId = null;
  try {
    await refreshLayers();
    await loadPixels();
  } catch (e) {
    toast(e.message || 'Could not load image', { type: 'error' });
  }
}

/** Reload the layer stack and repaint the panel. */
async function refreshLayers() {
  if (!current) { layers = []; activeLayerId = null; renderLayerPanel(); return; }
  try {
    const r = await listLayers(current.image_id);
    layers = r?.layers || [];
  } catch (_) {
    layers = [];
  }
  const selected = layers.find((l) => l.layer_id === activeLayerId && !l.is_group);
  if (!selected) {
    const pixel = layers.filter((l) => !l.is_group);
    activeLayerId = pixel.length ? pixel[pixel.length - 1].layer_id : null;
  }
  renderLayerPanel();
  setStatus('saved');
}

/** Push a fresh composite (after any layer or pixel change). */
async function reloadComposite() {
  if (!current) return;
  try {
    const { w, h, buf } = await fetchRender(current.image_id);
    current.width = w;
    current.height = h;
    renderRaw(w, h, buf);
    setStatus('saved');
  } catch (e) {
    toast(e.message || 'Could not render image', { type: 'error' });
  }
}

async function refreshImages() {
  try {
    const r = await listImages();
    images = r?.images || [];
  } catch (_) { /* keep last list */ }
}

async function openNewest() {
  await refreshImages();
  if (images.length) await openImage(images[0]);
  else { current = null; resetCurve(); renderTitle(); renderStage(); setStatus('saved'); }
}

/* ── Real-time apply pipeline ───────────────────────────────── */

function queueOp(operation, commit) {
  pending = { operation, commit };
  if (!applying) void pump();
}

async function pump() {
  if (!current || applying || !pending) return;
  applying = true;
  const { operation, commit } = pending;
  pending = null;
  setStatus('saving');
  try {
    const { w, h, buf } = await rawApply(
      current.image_id, [operation], commit, activeLayerId,
    );
    current.width = w;
    current.height = h;
    renderRaw(w, h, buf);
    setStatus('saved');
    // A committed resize/crop/rotate changed the layer's own rectangle.
    if (commit) void refreshLayers();
  } catch (e) {
    toast(e.message || 'Edit failed', { type: 'error' });
    setStatus('saved');
  } finally {
    applying = false;
    if (pending) void pump();
  }
}

function apply(operation, { commit = true } = {}) {
  if (!current) {
    toast('Upload an image first', { type: 'error' });
    return;
  }
  queueOp(operation, commit);
}

/* ── Upload / reset / download / delete ─────────────────────── */

async function pickFile() {
  const [file] = await pickFiles({ accept: 'image/*' });
  if (file) void uploadFile(file);
}

async function uploadFile(file) {
  setStatus('saving');
  try {
    const created = await createImage(file);
    await refreshImages();
    await openImage(created);
    toast(`Opened ${created.title}`, { type: 'info' });
  } catch (e) {
    toast(e.message || 'Upload failed — is this a valid image?', { type: 'error' });
    setStatus('saved');
  }
}

function resetCurrent() {
  if (!current) return;
  apply({ op: 'reset' });
  toast('Reverted to original', { type: 'info' });
}

async function downloadCurrent() {
  if (!current) return;
  try {
    const blob = await apiFetch(
      `/api/images/${encodeURIComponent(current.image_id)}/data`,
      { responseType: 'blob' },
    );
    await saveOrDownload(blob, { name: `${current.title || 'image'}.png`, dir: 'Pictures', app: 'Image' });
  } catch (e) {
    toast(e.message || 'Save failed', { type: 'error' });
  }
}

async function removeCurrent() {
  if (!current) return;
  if (!window.confirm(`Delete "${current.title}"?`)) return;
  try {
    await deleteImage(current.image_id);
    current = null;
    await openNewest();
    toast('Image deleted', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Could not delete image', { type: 'error' });
  }
}

/* ── Image menu popup ───────────────────────────────────────── */

function ensureMenu() {
  if (imageMenuPopup) return;
  imageMenuPopup = document.createElement('div');
  imageMenuPopup.className = 'image-menu hidden';
  imageMenuPopup.setAttribute('role', 'menu');
  document.body.appendChild(imageMenuPopup);
}

function closeMenu() {
  if (!imageMenuOpen) return;
  imageMenuOpen = false;
  imageMenuPopup?.classList.add('hidden');
  imageMenuBtn?.setAttribute('aria-expanded', 'false');
  document.removeEventListener('pointerdown', onMenuOutside, true);
  document.removeEventListener('keydown', onMenuKey, true);
}

function onMenuOutside(e) {
  if (imageMenuPopup && !imageMenuPopup.contains(e.target)
    && imageMenuBtn && !imageMenuBtn.contains(e.target)) {
    closeMenu();
  }
}

function onMenuKey(e) {
  if (e.key === 'Escape') closeMenu();
}

function renderMenu() {
  if (!imageMenuPopup) return;
  imageMenuPopup.innerHTML = '';

  if (!images.length) {
    const empty = document.createElement('div');
    empty.className = 'image-menu-empty';
    empty.textContent = 'No images yet';
    imageMenuPopup.appendChild(empty);
  } else {
    images.forEach((img) => {
      const item = document.createElement('button');
      item.type = 'button';
      item.className = 'image-menu-item';
      if (img.image_id === current?.image_id) item.classList.add('is-active');
      item.setAttribute('role', 'menuitem');

      const title = document.createElement('span');
      title.className = 'image-menu-title';
      title.textContent = img.title;
      const dims = document.createElement('span');
      dims.className = 'image-menu-time';
      dims.textContent = `${img.width}×${img.height}`;
      const check = document.createElement('span');
      check.className = 'image-menu-check';
      item.append(title, dims, check);
      void setIcon(check, 'ui/check', { size: 13 });

      item.addEventListener('click', () => {
        closeMenu();
        if (img.image_id !== current?.image_id) void openImage(img);
      });
      imageMenuPopup.appendChild(item);
    });
  }

  const foot = document.createElement('div');
  foot.className = 'image-menu-foot';
  const footItem = (iconName, label, danger, onClick) => {
    const item = document.createElement('button');
    item.type = 'button';
    item.className = 'image-menu-item';
    if (danger) item.classList.add('image-menu-item--danger');
    item.setAttribute('role', 'menuitem');
    const ic = document.createElement('span');
    ic.className = 'image-menu-foot-icon';
    item.appendChild(ic);
    void setIcon(ic, iconName, { size: 14 });
    const labelEl = document.createElement('span');
    labelEl.className = 'image-menu-title';
    labelEl.textContent = label;
    item.appendChild(labelEl);
    item.addEventListener('click', () => {
      closeMenu();
      onClick();
    });
    foot.appendChild(item);
  };
  footItem('ui/upload', 'Upload image', false, pickFile);
  footItem('ui/trash', 'Delete image', true, () => void removeCurrent());
  imageMenuPopup.appendChild(foot);
}

function openMenu() {
  ensureMenu();
  void refreshImages().then(renderMenu);
  imageMenuPopup.classList.remove('hidden');
  const r = imageMenuBtn.getBoundingClientRect();
  const left = Math.max(12, Math.min(r.left, window.innerWidth - 300 - 12));
  imageMenuPopup.style.left = `${left}px`;
  imageMenuPopup.style.top = `${r.bottom + 8}px`;
  imageMenuOpen = true;
  imageMenuBtn.setAttribute('aria-expanded', 'true');
  document.addEventListener('pointerdown', onMenuOutside, true);
  document.addEventListener('keydown', onMenuKey, true);
}

function toggleMenu() {
  if (imageMenuOpen) closeMenu();
  else openMenu();
}

/* ── Curves editor ──────────────────────────────────────────── */

function buildCurveLUT(points) {
  const lut = new Array(256);
  const n = points.length;
  if (n === 0) { for (let i = 0; i < 256; i++) lut[i] = i; return lut; }
  const xs = points.map((p) => p[0]);
  const ys = points.map((p) => p[1]);
  const d = new Array(n - 1);
  for (let i = 0; i < n - 1; i++) {
    const dx = xs[i + 1] - xs[i];
    d[i] = Math.abs(dx) < 1e-9 ? 0 : (ys[i + 1] - ys[i]) / dx;
  }
  const m = new Array(n).fill(0);
  if (n === 2) { m[0] = d[0]; m[1] = d[0]; }
  else {
    m[0] = d[0]; m[n - 1] = d[n - 2];
    for (let i = 1; i < n - 1; i++) {
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
  const clamp = (v) => Math.max(0, Math.min(255, v));
  for (let seg = 0; seg < n - 1; seg++) {
    const x0 = xs[seg], x1 = xs[seg + 1], y0 = ys[seg], y1 = ys[seg + 1];
    const h = x1 - x0;
    if (Math.abs(h) < 1e-9) continue;
    const i0 = Math.max(0, Math.round(x0));
    const i1 = Math.min(255, Math.round(x1));
    for (let x = i0; x <= i1; x++) {
      const t = (x - x0) / h, t2 = t * t, t3 = t2 * t;
      const h00 = 2 * t3 - 3 * t2 + 1;
      const h10 = t3 - 2 * t2 + t;
      const h01 = -2 * t3 + 3 * t2;
      const h11 = t3 - t2;
      lut[x] = clamp(h00 * y0 + h10 * h * m[seg] + h01 * y1 + h11 * h * m[seg + 1]);
    }
  }
  const first = clamp(ys[0]);
  const last = clamp(ys[n - 1]);
  for (let x = 0; x < 256; x++) {
    if (x < xs[0]) lut[x] = first;
    else if (x > xs[n - 1]) lut[x] = last;
  }
  return lut;
}

function drawCurve() {
  if (!curveCanvas) return;
  const ctx = curveCanvas.getContext('2d');
  const W = curveCanvas.width;
  const H = curveCanvas.height;
  const PAD = 12;
  ctx.clearRect(0, 0, W, H);

  const grid = cssVar('--glass-border', 'rgba(128,128,128,0.3)');
  const muted = cssVar('--muted', '#888');
  const accent = cssVar('--accent', '#7aa2f7');
  const text = cssVar('--text', '#eee');

  ctx.strokeStyle = grid;
  ctx.lineWidth = 1;
  for (let i = 0; i <= 4; i++) {
    const x = PAD + (i * (W - 2 * PAD)) / 4;
    const y = PAD + (i * (H - 2 * PAD)) / 4;
    ctx.beginPath(); ctx.moveTo(x, PAD); ctx.lineTo(x, H - PAD); ctx.stroke();
    ctx.beginPath(); ctx.moveTo(PAD, y); ctx.lineTo(W - PAD, y); ctx.stroke();
  }

  ctx.strokeStyle = muted;
  ctx.setLineDash([3, 3]);
  ctx.beginPath(); ctx.moveTo(PAD, H - PAD); ctx.lineTo(W - PAD, PAD); ctx.stroke();
  ctx.setLineDash([]);

  const px = (x) => PAD + (x / 255) * (W - 2 * PAD);
  const py = (y) => H - PAD - (y / 255) * (H - 2 * PAD);

  const lut = buildCurveLUT(curvePoints);
  ctx.strokeStyle = accent;
  ctx.lineWidth = 2;
  ctx.beginPath();
  for (let x = 0; x <= 255; x++) {
    const xx = px(x); const yy = py(lut[x]);
    if (x === 0) ctx.moveTo(xx, yy); else ctx.lineTo(xx, yy);
  }
  ctx.stroke();

  for (const [x, y] of curvePoints) {
    ctx.fillStyle = text;
    ctx.strokeStyle = accent;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.arc(px(x), py(y), 4, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
  }
}

function curveFromEvent(e) {
  const r = curveCanvas.getBoundingClientRect();
  const cx = (e.clientX - r.left) * (curveCanvas.width / r.width);
  const cy = (e.clientY - r.top) * (curveCanvas.height / r.height);
  const PAD = 12;
  const x = Math.max(0, Math.min(255, ((cx - PAD) / (curveCanvas.width - 2 * PAD)) * 255));
  const y = Math.max(0, Math.min(255, (1 - (cy - PAD) / (curveCanvas.height - 2 * PAD)) * 255));
  return [x, y];
}

function nearestCurvePoint(e) {
  const [x, y] = curveFromEvent(e);
  let best = -1;
  let bestDist = Infinity;
  curvePoints.forEach((p, i) => {
    const dx = p[0] - x;
    const dy = p[1] - y;
    const dist = dx * dx + dy * dy;
    if (dist < bestDist) { bestDist = dist; best = i; }
  });
  return bestDist < 400 ? best : -1;
}

function curveOp() {
  const pts = curvePoints
    .map((p) => [Math.round(p[0]), Math.round(p[1])])
    .sort((a, b) => a[0] - b[0]);
  return { op: 'curves', points: pts };
}

function buildCurveEditor() {
  const wrap = document.createElement('div');
  wrap.className = 'image-curves';

  curveCanvas = document.createElement('canvas');
  curveCanvas.className = 'image-curves-canvas';
  curveCanvas.width = 248;
  curveCanvas.height = 160;
  wrap.appendChild(curveCanvas);

  curveCanvas.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    const hit = nearestCurvePoint(e);
    if (hit >= 0) {
      curveDrag = hit;
      curveCanvas.setPointerCapture(e.pointerId);
    } else {
      const [x, y] = curveFromEvent(e);
      curvePoints.push([x, y]);
      curvePoints.sort((a, b) => a[0] - b[0]);
      curveDrag = curvePoints.findIndex((p) => p[0] === x && p[1] === y);
      curveCanvas.setPointerCapture(e.pointerId);
      drawCurve();
    }
  });

  curveCanvas.addEventListener('pointermove', (e) => {
    if (curveDrag < 0) return;
    const [x, y] = curveFromEvent(e);
    curvePoints[curveDrag] = [x, y];
    drawCurve();
    if (current) apply(curveOp(), { commit: false });
  });

  curveCanvas.addEventListener('pointerup', () => {
    if (curveDrag < 0) return;
    curveDrag = -1;
    if (current) apply(curveOp(), { commit: true });
  });

  curveCanvas.addEventListener('dblclick', (e) => {
    if (curvePoints.length <= 2) return;
    const hit = nearestCurvePoint(e);
    if (hit >= 0) {
      curvePoints.splice(hit, 1);
      drawCurve();
      if (current) apply(curveOp(), { commit: true });
    }
  });

  drawCurve();
  return wrap;
}

/* ── Panel builders ─────────────────────────────────────────── */

/**
 * A Photoshop-style docked panel: a title bar with a disclosure chevron,
 * optional header actions, and a body. Clicking the title collapses it.
 */
function panelSection(title, buildBody, opts = {}) {
  const sec = document.createElement('section');
  sec.className = 'image-panel-section';

  const head = document.createElement('div');
  head.className = 'image-panel-head';
  const chev = document.createElement('span');
  chev.className = 'image-panel-chevron';
  void setIcon(chev, 'ui/chevron-down', { size: 12 });
  const t = document.createElement('span');
  t.className = 'image-panel-title';
  t.textContent = title;
  head.append(chev, t);

  if (opts.actions && opts.actions.length) {
    const actions = document.createElement('div');
    actions.className = 'image-panel-actions';
    for (const a of opts.actions) actions.appendChild(a);
    head.appendChild(actions);
  }

  const body = document.createElement('div');
  body.className = 'image-panel-body';
  const content = buildBody();
  if (content) body.appendChild(content);

  head.addEventListener('click', (e) => {
    if (e.target.closest('button, select, input, a')) return;
    sec.classList.toggle('is-collapsed');
  });

  sec.append(head, body);
  return sec;
}

function railBtn(iconName, label, onClick, danger = false) {
  const btn = button({ icon: iconName, variant: 'ghost', onClick });
  btn.classList.add('ui-btn--icon', 'image-tool');
  if (danger) btn.classList.add('image-tool--danger');
  btn.title = label;
  btn.setAttribute('aria-label', label);
  return btn;
}

function effectBtn(iconName, label, onClick) {
  const btn = button({ icon: iconName, variant: 'ghost', onClick });
  btn.classList.add('ui-btn--icon', 'image-effect');
  btn.title = label;
  btn.setAttribute('aria-label', label);
  return btn;
}

function sliderGroup(label, min, max, opFn) {
  const group = document.createElement('div');
  group.className = 'image-slider';
  const lab = document.createElement('span');
  lab.className = 'image-slider-label';
  lab.textContent = label;
  const sl = slider({ min, max, step: 1, value: 0 });
  const val = document.createElement('span');
  val.className = 'image-slider-value';
  val.textContent = '0';
  sl.addEventListener('input', () => {
    val.textContent = sl.value;
    apply(opFn(Number(sl.value)), { commit: false });
  });
  sl.addEventListener('change', () => {
    if (Number(sl.value) !== 0) apply(opFn(Number(sl.value)), { commit: true });
  });
  group.append(lab, sl, val);
  return group;
}

/* ── Layers panel ───────────────────────────────────────────── */

/** Depth-first, top-of-stack first: each folder is followed by its contents. */
function displayRows() {
  const out = [];
  const walk = (parentId, depth) => {
    if (depth > 12) return;
    const kids = layers
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

function layerIcon(kind, visible) {
  const ic = document.createElement('span');
  ic.className = 'image-layer-icon';
  void setIcon(ic, kind, { size: 13 });
  if (!visible) ic.classList.add('is-hidden');
  return ic;
}

let opacityPutTimer = null;

function scheduleOpacity(layerId, value) {
  window.clearTimeout(opacityPutTimer);
  opacityPutTimer = window.setTimeout(async () => {
    if (!current) return;
    try {
      await updateLayer(current.image_id, layerId, { opacity: value });
      await reloadComposite();
    } catch (e) {
      toast(e.message || 'Could not change opacity', { type: 'error' });
    }
  }, 110);
}

function renderLayerPanel() {
  if (!layerListEl) return;

  /* Properties reflect the selected layer. */
  const selected = layers.find((l) => l.layer_id === activeLayerId) || null;
  if (layerBlendEl) {
    layerBlendEl.disabled = !selected;
    const mode = selected?.blend_mode;
    layerBlendEl.value = BLEND_MODES.includes(mode) ? mode : 'normal';
  }
  if (layerOpacityEl) {
    layerOpacityEl.disabled = !selected;
    layerOpacityEl.value = String(Math.round((selected?.opacity ?? 1) * 100));
  }
  if (layerOpacityValueEl) {
    layerOpacityValueEl.textContent = `${Math.round((selected?.opacity ?? 1) * 100)}%`;
  }

  layerListEl.textContent = '';
  for (const url of thumbUrls) URL.revokeObjectURL(url);
  thumbUrls = [];

  /* Display top-to-bottom: each folder is followed by its contents. */
  const ordered = displayRows();
  if (!ordered.length) {
    const empty = document.createElement('div');
    empty.className = 'image-layers-empty';
    empty.textContent = 'No layers';
    layerListEl.appendChild(empty);
    return;
  }

  for (const { layer, depth } of ordered) {
    const row = document.createElement('div');
    row.className = 'image-layer-row';
    row.dataset.layerId = layer.layer_id;
    row.draggable = true;
    if (layer.layer_id === activeLayerId) row.classList.add('is-active');
    row.style.paddingLeft = `${6 + depth * 14}px`;

    const eye = document.createElement('button');
    eye.type = 'button';
    eye.className = 'image-layer-eye';
    eye.title = layer.visible ? 'Hide layer' : 'Show layer';
    eye.setAttribute('aria-label', eye.title);
    eye.appendChild(layerIcon('ui/eye', layer.visible));
    eye.addEventListener('click', async (e) => {
      e.stopPropagation();
      try {
        await updateLayer(current.image_id, layer.layer_id, { visible: !layer.visible });
        await refreshLayers();
        await reloadComposite();
      } catch (err) {
        toast(err.message || 'Could not toggle layer', { type: 'error' });
      }
    });

    const thumb = document.createElement('span');
    thumb.className = 'image-layer-thumb';
    if (layer.is_group) {
      thumb.classList.add('is-folder');
      thumb.appendChild(layerIcon('ui/folder', true));
    } else {
      const img = document.createElement('img');
      img.alt = '';
      img.draggable = false;
      thumb.appendChild(img);
      const layerId = layer.layer_id;
      const imageId = current.image_id;
      void (async () => {
        try {
          const blob = await apiFetch(
            `/api/images/${encodeURIComponent(imageId)}/layers/${encodeURIComponent(layerId)}/thumb`,
            { responseType: 'blob' },
          );
          if (!blob) return;
          const url = URL.createObjectURL(blob);
          thumbUrls.push(url);
          img.src = url;
        } catch (_) { /* thumbnail is best-effort */ }
      })();
    }

    const name = document.createElement('span');
    name.className = 'image-layer-name';
    name.textContent = layer.name || (layer.is_group ? 'Folder' : 'Layer');

    const meta = document.createElement('span');
    meta.className = 'image-layer-meta';
    if (layer.is_group) {
      meta.appendChild(layerIcon('ui/folder', true));
    }
    if (layer.blend_mode && layer.blend_mode !== 'normal') {
      const badge = document.createElement('span');
      badge.className = 'image-layer-blend';
      badge.textContent = layer.blend_mode.replace('_', ' ');
      meta.appendChild(badge);
    }
    if ((layer.opacity ?? 1) < 0.999) {
      const pct = document.createElement('span');
      pct.className = 'image-layer-blend';
      pct.textContent = `${Math.round(layer.opacity * 100)}%`;
      meta.appendChild(pct);
    }

    row.append(eye, thumb, name, meta);
    row.addEventListener('click', () => {
      activeLayerId = layer.layer_id;
      renderLayerPanel();
    });

    /* Reorder within the same parent folder. */
    row.addEventListener('dragstart', (e) => {
      dragLayerId = layer.layer_id;
      e.dataTransfer.effectAllowed = 'move';
      try { e.dataTransfer.setData('text/plain', layer.layer_id); } catch (_) { /* ignore */ }
    });
    row.addEventListener('dragover', (e) => e.preventDefault());
    row.addEventListener('drop', async (e) => {
      e.preventDefault();
      const draggedId = dragLayerId;
      dragLayerId = null;
      if (!draggedId || draggedId === layer.layer_id) return;
      const dragged = layers.find((l) => l.layer_id === draggedId);
      if (!dragged || dragged.group_id !== layer.group_id) return;
      const siblings = layers
        .filter((l) => l.group_id === dragged.group_id)
        .sort((a, b) => a.position - b.position)
        .map((l) => l.layer_id)
        .filter((id) => id !== draggedId);
      const at = siblings.indexOf(layer.layer_id);
      if (at < 0) return;
      siblings.splice(at, 0, draggedId);
      try {
        await reorderLayers(current.image_id, siblings);
        await refreshLayers();
        await reloadComposite();
      } catch (err) {
        toast(err.message || 'Could not reorder layers', { type: 'error' });
      }
    });

    layerListEl.appendChild(row);
  }
}

async function runLayerAction(fn) {
  if (!current || !activeLayerId) {
    toast('Select a layer first', { type: 'error' });
    return;
  }
  try {
    await fn();
  } catch (e) {
    toast(e.message || 'Layer action failed', { type: 'error' });
  }
}

/** Build the Layers panel: blend + opacity, the stack, and a footer toolbar. */
function buildLayersPanel() {
  return panelSection('Layers', () => {
    const body = document.createElement('div');
    body.className = 'image-layers';

    const props = document.createElement('div');
    props.className = 'image-layer-props';

    const blendWrap = select({ options: BLEND_MODES, value: 'normal' });
    layerBlendEl = blendWrap.select;
    blendWrap.classList.add('image-layer-blend-select');
    layerBlendEl.addEventListener('change', () => runLayerAction(async () => {
      await updateLayer(current.image_id, activeLayerId, { blend_mode: layerBlendEl.value });
      await refreshLayers();
      await reloadComposite();
    }));

    const opacityRow = document.createElement('div');
    opacityRow.className = 'image-slider';
    const opacityLabel = document.createElement('span');
    opacityLabel.className = 'image-slider-label';
    opacityLabel.textContent = 'Opacity';
    layerOpacityEl = slider({ min: 0, max: 100, step: 1, value: 100 });
    layerOpacityValueEl = document.createElement('span');
    layerOpacityValueEl.className = 'image-slider-value';
    layerOpacityValueEl.textContent = '100%';
    layerOpacityEl.addEventListener('input', () => {
      layerOpacityValueEl.textContent = `${layerOpacityEl.value}%`;
      if (activeLayerId) scheduleOpacity(activeLayerId, Number(layerOpacityEl.value) / 100);
    });
    opacityRow.append(opacityLabel, layerOpacityEl, layerOpacityValueEl);

    props.append(blendWrap, opacityRow);
    body.appendChild(props);

    layerListEl = document.createElement('div');
    layerListEl.className = 'image-layer-list';
    body.appendChild(layerListEl);

    const foot = document.createElement('div');
    foot.className = 'image-layers-foot';
    foot.append(
      railBtn('ui/plus', 'New layer', () => runLayerAction(async () => {
        const created = await createLayer(current.image_id, {});
        if (created?.layer_id) activeLayerId = created.layer_id;
        await refreshLayers();
        await reloadComposite();
      })),
      railBtn('ui/folder-plus', 'New folder', () => runLayerAction(async () => {
        await createLayer(current.image_id, { is_group: true, name: 'Folder' });
        await refreshLayers();
        await reloadComposite();
      })),
      railBtn('ui/copy', 'Duplicate layer', () => runLayerAction(async () => {
        const created = await duplicateLayer(current.image_id, activeLayerId);
        if (created?.layer_id) activeLayerId = created.layer_id;
        await refreshLayers();
        await reloadComposite();
      })),
      railBtn('ui/chevron-down', 'Merge down', () => runLayerAction(async () => {
        await mergeLayer(current.image_id, activeLayerId);
        await refreshLayers();
        await reloadComposite();
      })),
      railBtn('ui/grid', 'Flatten image', () => runLayerAction(async () => {
        await flattenImage(current.image_id);
        await refreshLayers();
        await reloadComposite();
      })),
      railBtn('ui/trash', 'Delete layer', () => runLayerAction(async () => {
        await deleteLayer(current.image_id, activeLayerId);
        activeLayerId = null;
        await refreshLayers();
        await reloadComposite();
      }), true),
    );
    body.appendChild(foot);
    return body;
  });
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

/** Create the Image tile element (the plugin's window container). */
export function mountImageTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile image-tile';
  tileEl.dataset.plugin = IMAGE_PLUGIN;

  /* Top bar: image menu + title + save indicator */
  const bar = document.createElement('div');
  bar.className = 'image-bar';

  imageMenuBtn = document.createElement('button');
  imageMenuBtn.type = 'button';
  imageMenuBtn.className = 'image-menu-btn';
  imageMenuBtn.setAttribute('aria-haspopup', 'menu');
  imageMenuBtn.setAttribute('aria-expanded', 'false');
  imageMenuBtn.title = 'Images';
  const menuIcon = document.createElement('span');
  menuIcon.className = 'image-menu-btn-icon';
  imageMenuBtn.appendChild(menuIcon);
  void setIcon(menuIcon, 'ui/image', { size: 15 });
  const chevron = document.createElement('span');
  chevron.className = 'image-menu-btn-chevron';
  imageMenuBtn.appendChild(chevron);
  void setIcon(chevron, 'ui/chevron-down', { size: 12 });
  imageMenuBtn.addEventListener('click', toggleMenu);

  titleInput = document.createElement('input');
  titleInput.className = 'image-title';
  titleInput.type = 'text';
  titleInput.placeholder = 'Untitled';
  titleInput.maxLength = 120;
  titleInput.autocomplete = 'off';
  titleInput.addEventListener('change', () => {
    if (!current) return;
    const t = titleInput.value.trim();
    if (t && t !== current.title) {
      setStatus('dirty');
      void renameImage(current.image_id, t).then(() => {
        current.title = t;
        setStatus('saved');
      }).catch((e) => toast(e.message || 'Rename failed', { type: 'error' }));
    }
  });

  docMetaEl = document.createElement('span');
  docMetaEl.className = 'image-doc-meta';
  docMetaEl.textContent = '—';

  saveDot = document.createElement('span');
  saveDot.className = 'image-save-dot';
  saveDot.setAttribute('aria-hidden', 'true');

  // File actions move into the top bar (Studio-style); the transform tools
  // stay in the left rail as an editing palette.
  bar.append(
    imageMenuBtn,
    titleInput,
    docMetaEl,
    railBtn('ui/upload', 'Upload image', pickFile),
    railBtn('ui/refresh', 'Reset to original', () => resetCurrent()),
    railBtn('ui/save', 'Save to Pictures', () => void downloadCurrent()),
    railBtn('ui/trash', 'Delete image', () => void removeCurrent(), true),
    saveDot,
  );
  tileEl.appendChild(bar);

  /* Main: left rail + canvas + right panel */
  const main = document.createElement('div');
  main.className = 'image-main';

  const rail = document.createElement('div');
  rail.className = 'image-rail';
  for (const [icon, label, op] of TRANSFORMS) {
    rail.appendChild(railBtn(icon, label, () => apply(op)));
  }

  const canvasWrap = document.createElement('div');
  canvasWrap.className = 'image-canvas';
  stageEl = document.createElement('div');
  stageEl.className = 'image-stage';
  canvasWrap.appendChild(stageEl);

  const layersWrap = document.createElement('div');
  layersWrap.className = 'image-layers-panel';
  layersWrap.appendChild(buildLayersPanel());

  const panel = document.createElement('div');
  panel.className = 'image-panel';

  panel.appendChild(panelSection('Adjustments', () => {
    const stack = document.createElement('div');
    stack.className = 'image-panel-stack';
    stack.append(
      sliderGroup('Brightness', -255, 255, (v) => ({ op: 'brightness', amount: v })),
      sliderGroup('Contrast', -255, 255, (v) => ({ op: 'contrast', amount: v })),
    );
    return stack;
  }));

  panel.appendChild(panelSection('Curves', () => {
    const stack = document.createElement('div');
    stack.className = 'image-panel-stack';
    stack.appendChild(buildCurveEditor());
    const curveReset = button({ label: 'Reset curve', variant: 'ghost', size: 'sm', onClick: resetCurve });
    curveReset.classList.add('image-curves-reset');
    stack.appendChild(curveReset);
    return stack;
  }));

  panel.appendChild(panelSection('Effects', () => {
    const grid = document.createElement('div');
    grid.className = 'image-effects';
    for (const [icon, label, op] of EFFECTS) {
      grid.appendChild(effectBtn(icon, label, () => apply(op)));
    }
    return grid;
  }));

  panel.appendChild(panelSection('Filter', () => {
    const filterRow = document.createElement('div');
    filterRow.className = 'image-filter';
    const filterSelect = select({ options: FILTERS, value: 'lofi' });
    filterSelect.select.classList.add('image-filter-select');
    const applyFilterBtn = button({ label: 'Apply', variant: 'ghost', size: 'sm', onClick: () => apply({ op: 'filter', name: filterSelect.select.value }) });
    filterRow.append(filterSelect, applyFilterBtn);
    return filterRow;
  }));

  main.append(rail, canvasWrap, layersWrap, panel);
  tileEl.appendChild(main);

  /* Status line */
  const status = document.createElement('div');
  status.className = 'image-status';
  statusEl = document.createElement('span');
  status.appendChild(statusEl);
  tileEl.appendChild(status);

  void openNewest();
  return tileEl;
}

/** Deactivated: drop the window. */
export function unmountImageTile() {
  closeMenu();
  tileEl?.remove();
  tileEl = null;
  imageMenuBtn = null;
  titleInput = null;
  statusEl = null;
  saveDot = null;
  stageEl = null;
  canvasEl = null;
  canvasCtx = null;
  curveCanvas = null;
  imageMenuPopup = null;
  for (const url of thumbUrls) URL.revokeObjectURL(url);
  thumbUrls = [];
  layers = [];
  activeLayerId = null;
  layerListEl = null;
  layerBlendEl = null;
  layerOpacityEl = null;
  layerOpacityValueEl = null;
  docMetaEl = null;
}

/** The tile element (or null when the Image window is not mounted). */
export function getImageTileElement() {
  return tileEl;
}

/* ── AI wiring ──────────────────────────────────────────────── */

function onAgentActions(e) {
  const actions = e.detail || [];
  const imageActions = actions.filter((a) => /^image_/.test(a?.action || ''));
  if (!imageActions.length) return;

  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: IMAGE_PLUGIN } }));

  const touchedId = imageActions
    .map((a) => a.data?.image_id)
    .find((id) => !!id);
  const deleted = imageActions.some((a) => a.action === 'image_delete' && a.result === 'ok');

  void (async () => {
    await refreshImages();
    if (deleted) {
      if (current && !images.some((i) => i.image_id === current.image_id)) {
        await openNewest();
      }
    } else if (touchedId && current?.image_id !== touchedId) {
      const found = images.find((i) => i.image_id === touchedId);
      if (found) await openImage(found);
    } else if (current) {
      const full = await fetchImage(current.image_id).catch(() => null);
      if (full) {
        current = { ...current, ...full };
        renderStage();
        try {
          await refreshLayers();
          await loadPixels();
        } catch (_) { /* ignore */ }
      }
    }
  })();
}

async function importFromFiles(path, name) {
  try {
    if (!getImageTileElement()) mountImageTile();
    const file = await fileFromHome(path, name, 'application/octet-stream');
    await uploadFile(file);
  } catch (e) {
    toast(e.message || 'Could not open file', { type: 'error' });
  }
}

let wired = false;
export function wireImageEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  onOpenFromFiles(IMAGE_PLUGIN, (d) => importFromFiles(d.path, d.name));
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators + window management. */
export function imageContextMenu() {
  const hasImage = !!current;
  const hasLayer = hasImage && !!activeLayerId;
  const newLayer = () => runLayerAction(async () => {
    const created = await createLayer(current.image_id, {});
    if (created?.layer_id) activeLayerId = created.layer_id;
    await refreshLayers();
    await reloadComposite();
  });
  return [
    { type: 'item', label: 'Upload image', icon: 'ui/upload', onClick: pickFile },
    { type: 'item', label: 'New layer', icon: 'ui/plus', disabled: !hasImage, onClick: newLayer },
    { type: 'item', label: 'New folder', icon: 'ui/folder-plus', disabled: !hasImage, onClick: () => runLayerAction(async () => {
      await createLayer(current.image_id, { is_group: true, name: 'Folder' });
      await refreshLayers();
      await reloadComposite();
    }) },
    { type: 'item', label: 'Duplicate layer', icon: 'ui/copy', disabled: !hasLayer, onClick: () => runLayerAction(async () => {
      const created = await duplicateLayer(current.image_id, activeLayerId);
      if (created?.layer_id) activeLayerId = created.layer_id;
      await refreshLayers();
      await reloadComposite();
    }) },
    { type: 'item', label: 'Merge down', icon: 'ui/chevron-down', disabled: !hasLayer, onClick: () => runLayerAction(async () => {
      await mergeLayer(current.image_id, activeLayerId);
      await refreshLayers();
      await reloadComposite();
    }) },
    { type: 'item', label: 'Flatten image', icon: 'ui/grid', disabled: !hasImage, onClick: () => runLayerAction(async () => {
      await flattenImage(current.image_id);
      await refreshLayers();
      await reloadComposite();
    }) },
    { type: 'separator' },
    { type: 'item', label: 'Grayscale', icon: 'ui/grayscale', disabled: !hasLayer, onClick: () => apply({ op: 'grayscale' }) },
    { type: 'item', label: 'Sepia', icon: 'ui/sepia', disabled: !hasLayer, onClick: () => apply({ op: 'sepia' }) },
    { type: 'separator' },
    { type: 'item', label: 'Reset to original', icon: 'ui/refresh', disabled: !hasLayer, onClick: () => resetCurrent() },
    { type: 'item', label: 'Delete layer', icon: 'ui/trash', danger: true, disabled: !hasLayer, onClick: () => runLayerAction(async () => {
      await deleteLayer(current.image_id, activeLayerId);
      activeLayerId = null;
      await refreshLayers();
      await reloadComposite();
    }) },
    { type: 'item', label: 'Delete image', icon: 'ui/trash', danger: true, disabled: !hasImage, onClick: () => void removeCurrent() },
  ];
}

export default {
  name: 'image',
  icon: 'ui/image',
  mount: mountImageTile,
  unmount: unmountImageTile,
  getElement: getImageTileElement,
  wireEvents: wireImageEvents,
  contextMenu: imageContextMenu,
};
