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

import { button, emptyState, select, slider, toast } from '/ui/index.js';
import { setIcon } from '/ui/index.js';
import { apiFetch, getToken } from '/js/api.js';

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
let saveDot = null;
let stageEl = null;
let canvasEl = null;
let canvasCtx = null;

let images = [];
let current = null;   // { image_id, title, width, height }
let busy = false;

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

/** Apply operations server-side (Rust) and stream raw RGBA back. */
async function rawApply(id, operations, commit) {
  const token = getToken();
  const headers = { 'Content-Type': 'application/json' };
  if (token) headers.Authorization = `Bearer ${token}`;
  const res = await fetch(
    `/api/images/${encodeURIComponent(id)}/apply?raw=true&commit=${commit ? 'true' : 'false'}`,
    { method: 'POST', headers, body: JSON.stringify({ operations }) },
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
  if (mode === 'saving') {
    statusEl.textContent = 'Applying…';
    saveDot?.classList.add('is-active');
  } else if (mode === 'dirty') {
    statusEl.textContent = 'Unsaved title';
    saveDot?.classList.add('is-active');
  } else {
    const dims = current ? `${current.width}×${current.height}` : 'No image';
    statusEl.textContent = `Saved ${t} · ${dims}`;
    saveDot?.classList.remove('is-active');
  }
}

/* ── Canvas rendering ───────────────────────────────────────── */

function renderStage() {
  if (!stageEl) return;
  stageEl.textContent = '';
  if (!current) {
    stageEl.appendChild(emptyState({ title: 'No image open', body: 'Upload an image to start editing.' }));
    canvasEl = null;
    canvasCtx = null;
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
}

async function loadPixels() {
  if (!current || !canvasCtx) return;
  const blob = await apiFetch(
    `/api/images/${encodeURIComponent(current.image_id)}/data`,
    { responseType: 'blob' },
  );
  const url = URL.createObjectURL(blob);
  const img = new Image();
  await new Promise((resolve, reject) => {
    img.onload = resolve;
    img.onerror = () => reject(new Error('Could not decode image'));
    img.src = url;
  });
  canvasEl.width = img.naturalWidth;
  canvasEl.height = img.naturalHeight;
  canvasCtx.drawImage(img, 0, 0);
  URL.revokeObjectURL(url);
  current.width = img.naturalWidth;
  current.height = img.naturalHeight;
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
  try {
    await loadPixels();
  } catch (e) {
    toast(e.message || 'Could not load image', { type: 'error' });
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
    const { w, h, buf } = await rawApply(current.image_id, [operation], commit);
    current.width = w;
    current.height = h;
    renderRaw(w, h, buf);
    setStatus('saved');
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

function pickFile() {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = 'image/*';
  input.addEventListener('change', () => {
    const file = input.files?.[0];
    if (file) void uploadFile(file);
  });
  input.click();
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
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${current.title || 'image'}.png`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  } catch (e) {
    toast(e.message || 'Download failed', { type: 'error' });
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

function eyebrow(text) {
  const d = document.createElement('div');
  d.className = 'image-eyebrow';
  d.textContent = text;
  return d;
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

  saveDot = document.createElement('span');
  saveDot.className = 'image-save-dot';
  saveDot.setAttribute('aria-hidden', 'true');

  bar.append(imageMenuBtn, titleInput, saveDot);
  tileEl.appendChild(bar);

  /* Main: left rail + canvas + right panel */
  const main = document.createElement('div');
  main.className = 'image-main';

  const rail = document.createElement('div');
  rail.className = 'image-rail';
  for (const [icon, label, op] of TRANSFORMS) {
    rail.appendChild(railBtn(icon, label, () => apply(op)));
  }
  const railDivider = document.createElement('div');
  railDivider.className = 'image-rail-divider';
  rail.appendChild(railDivider);
  rail.append(
    railBtn('ui/upload', 'Upload image', pickFile),
    railBtn('ui/refresh', 'Reset to original', () => resetCurrent()),
    railBtn('ui/download', 'Download', () => void downloadCurrent()),
    railBtn('ui/trash', 'Delete image', () => void removeCurrent(), true),
  );

  const canvasWrap = document.createElement('div');
  canvasWrap.className = 'image-canvas';
  stageEl = document.createElement('div');
  stageEl.className = 'image-stage';
  canvasWrap.appendChild(stageEl);

  const panel = document.createElement('div');
  panel.className = 'image-panel';

  panel.appendChild(eyebrow('Adjust'));
  panel.appendChild(sliderGroup('Brightness', -255, 255, (v) => ({ op: 'brightness', amount: v })));
  panel.appendChild(sliderGroup('Contrast', -255, 255, (v) => ({ op: 'contrast', amount: v })));

  panel.appendChild(eyebrow('Curves'));
  panel.appendChild(buildCurveEditor());
  const curveReset = button({ label: 'Reset curve', variant: 'ghost', size: 'sm', onClick: resetCurve });
  curveReset.classList.add('image-curves-reset');
  panel.appendChild(curveReset);

  panel.appendChild(eyebrow('Effects'));
  const grid = document.createElement('div');
  grid.className = 'image-effects';
  for (const [icon, label, op] of EFFECTS) {
    grid.appendChild(effectBtn(icon, label, () => apply(op)));
  }
  panel.appendChild(grid);

  panel.appendChild(eyebrow('Filter'));
  const filterRow = document.createElement('div');
  filterRow.className = 'image-filter';
  const filterSelect = select({ options: FILTERS, value: 'lofi' });
  filterSelect.select.classList.add('image-filter-select');
  const applyFilterBtn = button({ label: 'Apply', variant: 'ghost', size: 'sm', onClick: () => apply({ op: 'filter', name: filterSelect.select.value }) });
  filterRow.append(filterSelect, applyFilterBtn);
  panel.appendChild(filterRow);

  main.append(rail, canvasWrap, panel);
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
        try { await loadPixels(); } catch (_) { /* ignore */ }
      }
    }
  })();
}

let wired = false;
export function wireImageEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
}

export default {
  name: 'image',
  icon: 'ui/image',
  mount: mountImageTile,
  unmount: unmountImageTile,
  getElement: getImageTileElement,
  wireEvents: wireImageEvents,
};
