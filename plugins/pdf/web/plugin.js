import {
  menuSurface, menuItem, confirmDialog, promptDialog, menuButton, toolbar, input as uiInput,
  toolButton, placePopover, controlButton, icon, button, emptyState, toast, spinner, setTileGlow,
  glowFromDrawable, setIcon, listReveal, cardEnter, modal, segmented,
} from '../../ui/index.js';

/**
 * pdf.js — the PDF plugin's window: write documents, read and mark up PDFs.
 *
 * Two surfaces, two jobs:
 *
 * - **Write** (documents made here): an A4 editor (`editor.js`). The PDF is an
 *   *export* — the shell prints the same HTML with Chromium, so the PDF is
 *   exactly what was on screen. Nothing ever edits a PDF's text.
 * - **PDF** (any PDF, imported or exported): pdf.js paints the real document;
 *   markup tools add real PDF comments through PdfCraft on the server
 *   (highlight, comment, text box, drawing, shapes, links), the Comments panel
 *   edits or deletes them, and the rail organises pages. Every change is
 *   undoable.
 *
 * AI wiring: `pdf_*` tool outcomes arrive via `agent:actions`; the window opens
 * what the AI touched.
 */

import { apiFetch } from '../../js/api.js';
import { t, tp } from '../../js/i18n.js';
import { formatMenuDate, showDocumentMenu } from '../../js/officeMenu.js';
import { saveOrDownload, onOpenFromFiles, fileFromHome, pickFiles } from '../../js/files.js';
import { createEditor } from './editor.js';
import { TEMPLATES, templateHtml } from './doc-themes.js';

export const PDF_PLUGIN = 'pdf';

/* ── pdf.js ─────────────────────────────────────────────────── */

let pdfjsLib = null;
let pdfjsLoading = null;
function loadPdfJs() {
  if (pdfjsLib) return Promise.resolve(pdfjsLib);
  if (!pdfjsLoading) {
    pdfjsLoading = import('../../vendor/pdfjs/pdf.min.mjs').then((mod) => {
      mod.GlobalWorkerOptions.workerSrc = '/vendor/pdfjs/pdf.worker.min.mjs';
      pdfjsLib = mod;
      return mod;
    }).catch((e) => {
      pdfjsLoading = null;
      throw new Error(`pdf.js failed to load: ${e.message || e}`);
    });
  }
  return pdfjsLoading;
}
const PDFJS_OPTS = { cMapUrl: '/vendor/pdfjs/cmaps/', cMapPacked: true, standardFontDataUrl: '/vendor/pdfjs/standard_fonts/' };

const ZOOMS = [50, 75, 100, 120, 150, 200, 300];
const DEFAULT_ZOOM = 3;
const enc = (id) => encodeURIComponent(id);

/* ── State ──────────────────────────────────────────────────── */

let tileEl = null;
let titleInput = null;
let docMenuBtn = null;
let statusEl = null;
let railEl = null;
let canvasEl = null;
let mainEl = null;
let panelEl = null;
let pageLabelEl = null;
let zoomLabelEl = null;
let modeStripEl = null;
let undoBtn = null;
let redoBtn = null;
let viewSwitch = null;
let pdfTools = null;
let writeTools = null;
let exportBtn = null;
let freshnessEl = null;
let commentsBtn = null;
let editorToolbarSlot = null;
const modeBtns = {};

let docs = [];
/** { pdf_id, title, page_count, updated_at, editor_html?, pdf_stale?, history? } */
let currentPdf = null;
let currentPage = 0;
let zoomIndex = DEFAULT_ZOOM;
let fitZoom = null;
let history = { undo: 0, redo: 0, undo_label: null };
let lastEntered = '';
/** 'write' | 'pdf' */
let view = 'pdf';
let editor = null;
/** 'select' | 'note' | 'text' | 'draw' | 'shape' | 'link' */
let mode = 'select';
let shapeKind = 'box';
let inkColor = '#e5484d';
let showPanel = false;
let comments = [];
let marks = [];

let pdfDoc = null;
let renderSeq = 0;
let renderTask = null;
let viewport = null;
let pageEl = null;

let docMenuPopup = null;
let docMenuOpen = false;
let menuMode = 'default';
let floatBar = null;
let pageMenu = null;
let dragIndex = null;
let thumbObserver = null;

const zoomPct = () => fitZoom ?? ZOOMS[zoomIndex];
const scale = () => zoomPct() / 100;
const editable = () => currentPdf?.editor_html != null;

/* ── API ────────────────────────────────────────────────────── */

const listPdfs = async () => (await apiFetch('/api/pdfs'))?.data || [];
const fetchPdf = async (id) => (await apiFetch(`/api/pdfs/${enc(id)}`))?.data;

async function post(path, body, method = 'POST') {
  return (await apiFetch(path, { method, body: JSON.stringify(body ?? {}) }))?.data;
}

/**
 * A server edit on the current PDF, then refresh what depends on its bytes.
 * Every edit is undoable: no confirmations, the toast says what happened.
 */
async function edit(path, body, { okMsg, page = currentPage, method = 'POST' } = {}) {
  if (!currentPdf) return null;
  const target = currentPdf;
  setBusy(true);
  try {
    const res = await post(`/api/pdfs/${enc(target.pdf_id)}/${path}`, body, method);
    if (currentPdf !== target) return res;
    if (res?.page_count != null) currentPdf.page_count = res.page_count;
    if (res?.history) applyHistory(res.history);
    currentPage = clampPage(page);
    await reloadPages();
    showSaved();
    if (okMsg) toast(typeof okMsg === 'function' ? okMsg(res) : okMsg, { type: 'info' });
    return res;
  } catch (e) {
    setBusy(false);
    toast(e.message || t('plugin.pdf.toast.editFailed', null, 'Could not apply the change'), { type: 'error' });
    return null;
  }
}

async function reloadPages() {
  dropPdfDoc();
  closeFloatBar();
  const active = document.activeElement;
  if (tileEl && (!active || active === document.body)) tileEl.focus({ preventScroll: true });
  await renderMain();
  renderThumbs();
  if (showPanel) void loadComments();
}

/* ── Undo / redo ────────────────────────────────────────────── */

function applyHistory(h) {
  history = { undo: h?.undo ?? 0, redo: h?.redo ?? 0, undo_label: h?.undo_label ?? null };
  syncHistoryButtons();
}

function syncHistoryButtons() {
  if (!undoBtn) return;
  if (view === 'write') {
    undoBtn.disabled = false;
    redoBtn.disabled = false;
    undoBtn.title = t('plugin.pdf.toolbar.undo', null, 'Undo');
    return;
  }
  undoBtn.disabled = !history.undo;
  redoBtn.disabled = !history.redo;
  const label = history.undo_label ? labelForEdit(history.undo_label) : null;
  undoBtn.title = label ? t('plugin.pdf.toolbar.undoWhat', { what: label }, `Undo: ${label}`) : t('plugin.pdf.toolbar.undo', null, 'Undo');
}

function labelForEdit(label) {
  const key = {
    'Edit text': 'editText', Rotate: 'rotate', 'Reorder pages': 'reorder', 'Delete pages': 'deletePages',
    Merge: 'merge', 'Replace text': 'replace', Annotate: 'annotate', 'Add note': 'note', Draw: 'draw',
    Watermark: 'watermark', 'Header & footer': 'headerFooter', 'Remove marks': 'removeMarks',
    'Insert page': 'insertPage', 'Duplicate page': 'duplicate', 'Edit content': 'editContent', Export: 'export',
    'Delete comment': 'deleteComment', 'Edit comment': 'editComment', 'Move comment': 'moveComment', Reply: 'reply',
  }[label];
  return key ? t(`plugin.pdf.history.${key}`, null, label) : label;
}

async function stepHistory(undo) {
  if (view === 'write') { if (undo) editor?.undo(); else editor?.redo(); return; }
  if (!currentPdf || (undo ? !history.undo : !history.redo)) return;
  const res = await edit(undo ? 'undo' : 'redo', {});
  if (res) {
    const what = labelForEdit(res.label || '');
    toast(undo ? t('plugin.pdf.toast.undone', { what }, `Undone: ${what}`) : t('plugin.pdf.toast.redone', { what }, `Redone: ${what}`), { type: 'info' });
    if (res.label === 'Export' || res.label === 'Edit content') void refreshCurrentMeta();
  }
}

async function refreshCurrentMeta() {
  if (!currentPdf) return;
  try {
    const meta = await fetchPdf(currentPdf.pdf_id);
    if (!meta || meta.pdf_id !== currentPdf.pdf_id) return;
    Object.assign(currentPdf, meta);
    if (titleInput && document.activeElement !== titleInput) titleInput.value = currentPdf.title;
    applyHistory(meta.history);
    syncFreshness();
  } catch (_) { /* keep */ }
}

/* ── Views: Write | PDF ─────────────────────────────────────── */

function setView(next) {
  if (next === 'write' && !editable()) next = 'pdf';
  view = next;
  viewSwitch?.setValue(view, { silent: true });
  if (viewSwitch) viewSwitch.hidden = !editable();
  pdfTools?.classList.toggle('hidden', view !== 'pdf');
  writeTools?.classList.toggle('hidden', !editable());
  tileEl?.classList.toggle('is-writing', view === 'write');
  setMode('select');
  if (view === 'write') {
    if (!editor) {
      editor = createEditor(currentPdf, { onState: syncFreshness });
      editorToolbarSlot.replaceChildren(editor.toolbar);
    }
    mainEl.hidden = true;
    editorToolbarSlot.hidden = false;
    if (!editor.surface.isConnected) mainEl.after(editor.surface);
    editor.surface.hidden = false;
    editor.focus();
  } else {
    if (editor) {
      editor.surface.hidden = true;
      editorToolbarSlot.hidden = true;
    }
    mainEl.hidden = false;
    void renderMain();
  }
  syncHistoryButtons();
  syncFreshness();
}

function closeEditor() {
  editor?.destroy();
  editor = null;
  editorToolbarSlot?.replaceChildren();
}

/** "PDF up to date" / "PDF not updated" next to the export button. */
function syncFreshness(s) {
  if (!freshnessEl || !exportBtn) return;
  const stale = currentPdf?.pdf_stale !== false;
  const busy = s?.exporting || editor?.exporting;
  freshnessEl.textContent = busy
    ? t('plugin.pdf.editor.exporting', null, 'Making the PDF…')
    : stale
      ? t('plugin.pdf.editor.stale', null, 'PDF not updated')
      : t('plugin.pdf.editor.fresh', null, 'PDF up to date');
  freshnessEl.classList.toggle('is-stale', !!stale && !busy);
  exportBtn.disabled = !!busy;
  if (s?.saving) setStatus(t('plugin.pdf.status.saving', null, 'Saving…'));
  else if (s?.lastSaved && view === 'write') {
    const time = s.lastSaved.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    setStatus(t('plugin.pdf.status.textSaved', { time }, `Text saved ${time}`));
  }
}

async function exportCurrentDoc() {
  if (!editor && editable()) setView('write');
  if (!editor || !currentPdf) return;
  try {
    // A new PDF replaces the old one, comments made on it included.
    if (currentPdf.pdf_stale === false || history.undo) {
      const data = (await apiFetch(`/api/pdfs/${enc(currentPdf.pdf_id)}/comments`).catch(() => null))?.data;
      const n = (data?.comments?.length || 0) + (data?.marks?.length || 0);
      if (n && !await confirmDialog(t('plugin.pdf.dialog.exportReplaces', { count: n }, `Updating the PDF replaces the ${n} comments and marks added to it. Undo brings them back. Continue?`))) return;
    }
    let res = await editor.exportPdf();
    if (!res) return; // the browser print dialog path
    // Page numbers are part of the document's settings: stamp them on every
    // export (the print engine has no page counter of its own).
    if (editor.pageNumbers) {
      res = await post(`/api/pdfs/${enc(currentPdf.pdf_id)}/header-footer`, { slots: ['', '', '', '', '<<1>> / <<n>>', ''], font_size: 8.5 }) || res;
    }
    currentPdf.page_count = res.page_count;
    currentPdf.pdf_stale = false;
    applyHistory(res.history);
    syncFreshness();
    dropPdfDoc();
    lastEntered = '';
    toast(t('plugin.pdf.toast.exported', null, 'PDF updated'), { type: 'info' });
    setView('pdf');
    renderThumbs({ reveal: true });
    if (showPanel) void loadComments();
    void refreshDocs();
  } catch (e) {
    toast(e.message || t('plugin.pdf.toast.exportFailed', null, 'Could not make the PDF'), { type: 'error' });
  }
}

/* ── PDF modes (markup tools) ───────────────────────────────── */

const MODES = {
  note: { icon: 'ui/message-circle', fallback: 'Comment — click where the note goes (C)' },
  text: { icon: 'ui/type', fallback: 'Text box — click to type on the page (T)' },
  draw: { icon: 'ui/pencil', fallback: 'Draw — freehand (D)' },
  shape: { icon: 'ui/marquee-rect', fallback: 'Shape — drag a box or an oval (R)' },
  link: { icon: 'ui/link', fallback: 'Link — drag over the area to link (L)' },
};

function setMode(next) {
  if (next === mode) next = 'select';
  if (mode === 'draw' && next !== 'draw') commitInk();
  mode = next;
  closeFloatBar();
  for (const [m, b] of Object.entries(modeBtns)) {
    b.classList.toggle('is-active', m === mode);
    b.setAttribute('aria-pressed', String(m === mode));
  }
  if (canvasEl) canvasEl.dataset.mode = mode;
  renderModeStrip();
  setupInkLayer();
}

function renderModeStrip() {
  if (!modeStripEl) return;
  modeStripEl.innerHTML = '';
  modeStripEl.hidden = mode === 'select' || view !== 'pdf';
  if (modeStripEl.hidden) return;
  const ic = document.createElement('span');
  ic.className = 'pdf-mode-icon';
  ic.appendChild(icon(MODES[mode].icon, { size: 14 }));
  const hint = document.createElement('span');
  hint.className = 'pdf-mode-hint';
  hint.textContent = {
    note: t('plugin.pdf.mode.noteHint', null, 'Click where the comment goes'),
    text: t('plugin.pdf.mode.textHint', null, 'Click on the page and type · Ctrl+Enter adds it'),
    draw: t('plugin.pdf.mode.drawHint', null, 'Draw with the mouse or a finger · Save keeps the drawing'),
    shape: t('plugin.pdf.mode.shapeHint', null, 'Drag to draw the shape'),
    link: t('plugin.pdf.mode.linkHint', null, 'Drag over the text or area to link'),
  }[mode];
  modeStripEl.append(ic, hint);
  if (mode === 'shape') {
    const kinds = segmented({
      label: t('plugin.pdf.mode.shapeKind', null, 'Shape'),
      options: [
        { value: 'box', label: t('plugin.pdf.annot.box', null, 'Box') },
        { value: 'oval', label: t('plugin.pdf.annot.oval', null, 'Oval') },
      ],
      value: shapeKind,
      onChange: (v) => { shapeKind = v; },
    });
    kinds.classList.add('pdf-mode-tools');
    modeStripEl.appendChild(kinds);
  }
  if (mode === 'draw' || mode === 'shape') {
    const color = uiInput();
    color.type = 'color';
    color.classList.add('pdf-edit-color');
    color.value = inkColor;
    color.title = t('plugin.pdf.edit.colour', null, 'Colour');
    color.addEventListener('input', () => { inkColor = color.value; });
    modeStripEl.appendChild(color);
  }
  const spacer = document.createElement('span');
  spacer.className = 'pdf-mode-spacer';
  const done = button({ label: t('plugin.pdf.mode.done', null, 'Done'), variant: 'ghost', size: 'sm', onClick: () => setMode('select') });
  done.classList.add('pdf-mode-done');
  done.title = 'Esc';
  modeStripEl.append(spacer, done);
}

/* ── Rendering ──────────────────────────────────────────────── */

let renderChain = Promise.resolve();
function queueRender(fn) {
  const run = renderChain.then(fn, fn);
  renderChain = run.catch(() => {});
  return run;
}

function dropPdfDoc() {
  renderSeq++;
  try { renderTask?.cancel(); } catch (_) { /* finished */ }
  renderTask = null;
  if (pdfDoc) {
    const doc = pdfDoc;
    pdfDoc = null;
    void doc.destroy().catch(() => {});
  }
}

async function getPdfDoc() {
  if (!currentPdf) throw new Error('No PDF open');
  if (pdfDoc) return pdfDoc;
  const pdfjs = await loadPdfJs();
  const id = currentPdf.pdf_id;
  const doc = await pdfjs.getDocument({ url: `/api/pdfs/${enc(id)}/file`, ...PDFJS_OPTS }).promise;
  if (currentPdf?.pdf_id !== id) { void doc.destroy(); throw new Error('stale'); }
  pdfDoc = doc;
  return doc;
}

function setStatus(text) { if (statusEl) statusEl.textContent = text; }
function setBusy(on) { statusEl?.parentElement?.classList.toggle('is-busy', on); }

function showSaved() {
  const time = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const n = currentPdf?.page_count ?? 0;
  setStatus(tp('plugin.pdf.status.saved', n, { time }, `Saved ${time} · ${n} pages`));
  setBusy(false);
}

function clampPage(p) {
  const max = currentPdf?.page_count ?? 0;
  return max === 0 ? 0 : Math.max(0, Math.min(p, max - 1));
}

function fitWidthPct(page) {
  const base = page.getViewport({ scale: 1, rotation: page.rotate });
  const avail = Math.max(200, (canvasEl?.clientWidth || 800) - 48);
  return Math.max(25, Math.min(400, Math.floor((avail / base.width) * 100)));
}

async function paintCurrentPage() {
  const seq = ++renderSeq;
  try { renderTask?.cancel(); } catch (_) { /* finished */ }
  renderTask = null;
  if (!currentPdf || !canvasEl || view !== 'pdf') return;

  const pageKey = `${currentPdf.pdf_id}:${currentPage}`;
  const samePage = pageKey === lastEntered;
  const keepScroll = samePage ? { top: canvasEl.scrollTop, left: canvasEl.scrollLeft } : null;

  setBusy(true);
  if (!pageEl?.isConnected) {
    canvasEl.innerHTML = '';
    const loading = document.createElement('div');
    loading.className = 'pdf-loading';
    loading.appendChild(spinner({ size: 20 }));
    canvasEl.appendChild(loading);
  }

  let page;
  let vp;
  try {
    const doc = await getPdfDoc();
    page = await doc.getPage(currentPage + 1);
    if (fitZoom != null) fitZoom = fitWidthPct(page);
    vp = page.getViewport({ scale: scale(), rotation: page.rotate });
  } catch (e) {
    if (seq !== renderSeq || e.message === 'stale') return;
    canvasEl.innerHTML = '';
    canvasEl.appendChild(emptyState({ icon: 'ui/warning', title: t('plugin.pdf.empty.renderFailed', null, 'Could not show the page'), body: e.message || '' }));
    pageEl = null;
    setBusy(false);
    updateNav();
    return;
  }
  if (seq !== renderSeq) return;

  const el = document.createElement('div');
  el.className = 'pdf-page';
  el.style.width = `${vp.width}px`;
  el.style.height = `${vp.height}px`;
  el.setAttribute('role', 'img');
  el.setAttribute('aria-label', t('plugin.pdf.page.label', { page: currentPage + 1 }, `Page ${currentPage + 1}`));
  el.style.setProperty('--scale-factor', String(vp.scale));
  el.style.setProperty('--total-scale-factor', String(vp.scale));
  const canvas = document.createElement('canvas');
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.floor(vp.width * dpr));
  canvas.height = Math.max(1, Math.floor(vp.height * dpr));
  canvas.style.width = `${vp.width}px`;
  canvas.style.height = `${vp.height}px`;
  canvas.className = 'pdf-page-canvas';
  const textLayer = document.createElement('div');
  textLayer.className = 'pdf-text-layer';
  el.append(canvas, textLayer);

  try {
    const task = page.render({ canvasContext: canvas.getContext('2d'), viewport: vp, transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : undefined });
    renderTask = task;
    await task.promise;
    renderTask = null;
  } catch (e) {
    if (e?.name === 'RenderingCancelledException' || seq !== renderSeq) return;
    setBusy(false);
    return;
  }
  if (seq !== renderSeq) return;

  canvasEl.innerHTML = '';
  canvasEl.appendChild(el);
  pageEl = el;
  viewport = vp;
  attachPagePointer(el);
  if (!samePage) {
    lastEntered = pageKey;
    void cardEnter(el);
    canvasEl.scrollTop = 0;
  } else if (keepScroll) {
    canvasEl.scrollTop = keepScroll.top;
    canvasEl.scrollLeft = keepScroll.left;
  }
  try {
    const pdfjs = await loadPdfJs();
    const textContent = await page.getTextContent();
    if (seq !== renderSeq) return;
    await new pdfjs.TextLayer({ textContentSource: textContent, container: textLayer, viewport: vp }).render();
  } catch (_) { /* readable without a text layer */ }
  if (seq !== renderSeq) return;
  applyPageGlow(canvas);
  updateNav();
  markActiveThumb();
  setupInkLayer();
  setBusy(false);
}

function applyPageGlow(canvas) {
  if (!tileEl || !canvas.width || !canvas.height) return;
  let css = null;
  try {
    const sample = document.createElement('canvas');
    sample.width = canvas.width;
    sample.height = Math.max(1, Math.round(canvas.height * 0.4));
    sample.getContext('2d').drawImage(canvas, 0, 0, canvas.width, sample.height, 0, 0, sample.width, sample.height);
    css = glowFromDrawable(sample, 96);
  } catch (_) { css = null; }
  setTileGlow(tileEl, css);
}

function renderMain() { return queueRender(paintCurrentPage); }

function updateNav() {
  if (pageLabelEl) {
    const n = currentPdf?.page_count ?? 0;
    const pad = (v) => String(v).padStart(2, '0');
    pageLabelEl.textContent = n === 0 ? '00 / 00' : `${pad(currentPage + 1)} / ${pad(n)}`;
  }
  if (zoomLabelEl) zoomLabelEl.textContent = `${zoomPct()}%`;
}

/* ── PDF ↔ screen ───────────────────────────────────────────── */

function boxToPdfPoint(x, y) { return viewport.convertToPdfPoint(x, y); }
function boxToPdfRect(a, b) {
  const [x1, y1] = boxToPdfPoint(a.x, a.y);
  const [x2, y2] = boxToPdfPoint(b.x, b.y);
  return [Math.min(x1, x2), Math.min(y1, y2), Math.abs(x2 - x1), Math.abs(y2 - y1)];
}
function pdfRectToBox([x0, y0, x1, y1]) {
  const [ax, ay, bx, by] = viewport.convertToViewportRectangle([x0, y0, x1, y1]);
  return { left: Math.min(ax, bx), top: Math.min(ay, by), width: Math.abs(bx - ax), height: Math.abs(by - ay) };
}
function localPoint(e) {
  const r = pageEl.getBoundingClientRect();
  return { x: e.clientX - r.left, y: e.clientY - r.top };
}

/* ── Floating bar ───────────────────────────────────────────── */

function pillButton(label, kind, onClick, { iconName, title, on } = {}) {
  const b = controlButton();
  b.type = 'button';
  b.className = kind === 'apply' ? 'pdf-edit-apply' : kind === 'cancel' ? 'pdf-edit-cancel' : 'pdf-edit-pill';
  if (iconName) b.appendChild(icon(iconName, { size: 14 }));
  else b.textContent = label;
  if (title || iconName) { b.title = title || label; b.setAttribute('aria-label', title || label); }
  if (on) b.classList.add('is-on');
  b.addEventListener('mousedown', (e) => e.preventDefault());
  b.addEventListener('click', (e) => { e.preventDefault(); onClick(b); });
  return b;
}

function separator() {
  const el = document.createElement('span');
  el.className = 'pdf-edit-sep';
  return el;
}

function showFloatBar(anchor, nodes) {
  closeFloatBar({ keepText: true });
  const bar = toolbar([], { className: 'pdf-edit-bar' });
  bar.append(...nodes);
  bar.addEventListener('pointerdown', (e) => e.stopPropagation());
  document.body.appendChild(bar);
  const r = anchor.getBoundingClientRect ? anchor.getBoundingClientRect() : anchor;
  const top = r.top - bar.offsetHeight - 8;
  bar.style.left = `${Math.max(8, Math.min(r.left, window.innerWidth - bar.offsetWidth - 8))}px`;
  bar.style.top = `${top < 8 ? r.bottom + 8 : top}px`;
  floatBar = bar;
  return bar;
}

function closeFloatBar({ keepText = false } = {}) {
  floatBar?.remove();
  floatBar = null;
  if (!keepText) {
    pendingText?.el.remove();
    pendingText = null;
  }
}

/* ── Markup ─────────────────────────────────────────────────── */

const KIND_LABEL = {
  highlight: ['highlight', 'Highlight'], underline: ['underline', 'Underline'], strikeout: ['strikeout', 'Strikethrough'],
  squiggly: ['squiggly', 'Squiggly'], note: ['note', 'Comment'], box: ['box', 'Box'], oval: ['oval', 'Oval'],
  free_text: ['textBox', 'Text box'], link: ['link', 'Link'], ink: ['ink', 'Drawing'],
};
const markLabel = (k) => t(`plugin.pdf.annot.${KIND_LABEL[k]?.[0] || k}`, null, KIND_LABEL[k]?.[1] || k);

function hexToRgb(hex) {
  const h = hex.replace('#', '');
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
}

async function addMark(kind, rects, extra = {}) {
  if (!rects.length) return;
  let text = extra.text ?? '';
  if (kind === 'note' && !text) {
    const v = await promptDialog(t('plugin.pdf.dialog.noteText', null, 'Comment:'), '');
    if (v == null || !v.trim()) return;
    text = v.trim();
  } else if (kind === 'link' && !text) {
    const v = await promptDialog(t('plugin.pdf.dialog.linkUrl', null, 'Link address:'), 'https://');
    if (v == null || !v.trim() || v.trim() === 'https://') return;
    text = v.trim();
  }
  const body = { page: currentPage, kind, rects, text };
  if (extra.color) body.color = hexToRgb(extra.color);
  if (extra.size) body.size = extra.size;
  await edit('annotate', body, { okMsg: t('plugin.pdf.toast.annotAdded', { kind: markLabel(kind) }, `${markLabel(kind)} added`) });
  window.getSelection()?.removeAllRanges();
}

/** The selected text as PDF rects, one per line. */
function selectionRects() {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || !pageEl || !viewport) return [];
  const range = sel.getRangeAt(0);
  if (!pageEl.contains(range.commonAncestorContainer)) return [];
  const pr = pageEl.getBoundingClientRect();
  const lines = [];
  for (const r of range.getClientRects()) {
    if (r.width < 1 || r.height < 1) continue;
    const box = { x0: r.left - pr.left, y0: r.top - pr.top, x1: r.right - pr.left, y1: r.bottom - pr.top };
    const same = lines.find((l) => Math.abs(l.y0 - box.y0) < r.height * 0.5 && Math.abs(l.y1 - box.y1) < r.height * 0.5);
    if (same) { same.x0 = Math.min(same.x0, box.x0); same.x1 = Math.max(same.x1, box.x1); } else lines.push(box);
  }
  return lines.map((l) => boxToPdfRect({ x: l.x0, y: l.y0 }, { x: l.x1, y: l.y1 }));
}

function unionRect(rects) {
  const x0 = Math.min(...rects.map((r) => r[0]));
  const y0 = Math.min(...rects.map((r) => r[1]));
  const x1 = Math.max(...rects.map((r) => r[0] + r[2]));
  const y1 = Math.max(...rects.map((r) => r[1] + r[3]));
  return [x0, y0, x1 - x0, y1 - y0];
}

/** After a text selection: the quick marks, right there. */
function offerSelectionMarks() {
  const rects = selectionRects();
  if (!rects.length) return;
  const sel = window.getSelection();
  const r = sel.getRangeAt(0).getBoundingClientRect();
  const picked = sel.toString().replace(/\s+/g, ' ').trim();
  const nodes = [
    pillButton(markLabel('highlight'), 'pill', () => { closeFloatBar(); void addMark('highlight', rects); }),
    pillButton(markLabel('underline'), 'pill', () => { closeFloatBar(); void addMark('underline', rects); }),
    pillButton(markLabel('strikeout'), 'pill', () => { closeFloatBar(); void addMark('strikeout', rects); }),
    separator(),
    pillButton(markLabel('note'), 'pill', async () => {
      closeFloatBar();
      const v = await promptDialog(t('plugin.pdf.dialog.noteText', null, 'Comment:'), '');
      if (v == null || !v.trim()) return;
      void addMark('highlight', rects, { text: v.trim() });
    }),
    pillButton(markLabel('link'), 'pill', () => { closeFloatBar(); void addMark('link', [unionRect(rects)]); }),
  ];
  if (picked) {
    nodes.push(separator(), pillButton(t('plugin.pdf.annot.copy', null, 'Copy'), 'pill', () => {
      void navigator.clipboard?.writeText(picked);
      closeFloatBar();
    }));
  }
  showFloatBar(r, nodes);
}

/** Drag a rectangle over the page; resolves with a PDF rect (or null). */
function dragRect(e) {
  return new Promise((resolve) => {
    const start = localPoint(e);
    const boxEl = document.createElement('div');
    boxEl.className = `pdf-select-box${mode === 'shape' && shapeKind === 'oval' ? ' is-oval' : ''}`;
    if (mode === 'shape') boxEl.style.borderColor = inkColor;
    pageEl.appendChild(boxEl);
    const layout = (a, b) => Object.assign(boxEl.style, {
      left: `${Math.min(a.x, b.x)}px`, top: `${Math.min(a.y, b.y)}px`, width: `${Math.abs(a.x - b.x)}px`, height: `${Math.abs(a.y - b.y)}px`,
    });
    layout(start, start);
    const move = (ev) => layout(start, localPoint(ev));
    const up = (ev) => {
      window.removeEventListener('pointermove', move, true);
      window.removeEventListener('pointerup', up, true);
      boxEl.remove();
      const end = localPoint(ev);
      if (Math.abs(end.x - start.x) < 6 || Math.abs(end.y - start.y) < 4) { resolve(null); return; }
      resolve(boxToPdfRect(start, end));
    };
    window.addEventListener('pointermove', move, true);
    window.addEventListener('pointerup', up, true);
  });
}

/* Text box: type on the page, Ctrl+Enter adds it as a text-box comment. */
let pendingText = null;
let textSize = 12;
let textColor = '#16191d';

function placeTextBox(e) {
  closeFloatBar();
  const p = localPoint(e);
  const box = document.createElement('div');
  box.className = 'pdf-addtext';
  box.setAttribute('contenteditable', 'plaintext-only');
  if (box.contentEditable !== 'plaintext-only') box.setAttribute('contenteditable', 'true');
  box.style.left = `${p.x}px`;
  box.style.top = `${p.y}px`;
  box.dataset.placeholder = t('plugin.pdf.edit.typeHere', null, 'Type here…');
  const sizeLabel = document.createElement('span');
  sizeLabel.className = 'pdf-edit-label';
  const sync = () => {
    box.style.fontSize = `${textSize * scale()}px`;
    box.style.color = textColor;
    sizeLabel.textContent = `${textSize} pt`;
  };
  box.addEventListener('pointerdown', (ev) => ev.stopPropagation());
  box.addEventListener('keydown', (ev) => {
    if (ev.key === 'Escape') { ev.preventDefault(); ev.stopPropagation(); closeFloatBar(); }
    if (ev.key === 'Enter' && (ev.ctrlKey || ev.metaKey)) { ev.preventDefault(); void commitText(); }
  });
  pageEl.appendChild(box);
  pendingText = { el: box, at: p };
  const color = uiInput();
  color.type = 'color';
  color.classList.add('pdf-edit-color');
  color.value = textColor;
  color.addEventListener('input', () => { textColor = color.value; sync(); });
  showFloatBar(box, [
    sizeLabel,
    pillButton('', 'pill', () => { textSize = Math.max(6, textSize - 1); sync(); }, { iconName: 'ui/minus', title: t('plugin.pdf.edit.smaller', null, 'Smaller') }),
    pillButton('', 'pill', () => { textSize = Math.min(72, textSize + 1); sync(); }, { iconName: 'ui/plus', title: t('plugin.pdf.edit.larger', null, 'Larger') }),
    separator(), color, separator(),
    pillButton(t('plugin.pdf.edit.addText', null, 'Add text'), 'apply', () => void commitText()),
    pillButton(t('plugin.pdf.edit.cancel', null, 'Cancel'), 'cancel', () => closeFloatBar()),
  ]);
  sync();
  box.focus();
}

async function commitText() {
  if (!pendingText) return;
  const text = (pendingText.el.innerText || '').replace(/ /g, ' ').replace(/\n+$/, '');
  if (!text.trim()) { toast(t('plugin.pdf.toast.typeSomeText', null, 'Type some text first'), { type: 'info' }); pendingText.el.focus(); return; }
  const r = pendingText.el.getBoundingClientRect();
  const pr = pageEl.getBoundingClientRect();
  const rect = boxToPdfRect({ x: r.left - pr.left, y: r.top - pr.top }, { x: r.right - pr.left + 8, y: r.bottom - pr.top + 4 });
  const size = textSize;
  const colorHex = textColor;
  closeFloatBar();
  await addMark('free_text', [rect], { text, size, color: colorHex });
}

/* Drawing: strokes collect on an overlay; Save keeps them as one drawing. */
let inkLayer = null;
let inkStrokes = []; // in PDF points

function setupInkLayer() {
  inkLayer?.remove();
  inkLayer = null;
  if (mode !== 'draw' || !pageEl || !viewport) return;
  const c = document.createElement('canvas');
  c.className = 'pdf-ink-layer';
  const dpr = window.devicePixelRatio || 1;
  c.width = Math.floor(pageEl.clientWidth * dpr);
  c.height = Math.floor(pageEl.clientHeight * dpr);
  pageEl.appendChild(c);
  inkLayer = c;
  const ctx = c.getContext('2d');
  ctx.scale(dpr, dpr);
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  const redraw = () => {
    ctx.clearRect(0, 0, c.width, c.height);
    ctx.strokeStyle = inkColor;
    ctx.lineWidth = 2 * scale();
    for (const s of inkStrokes) {
      ctx.beginPath();
      s.forEach(([x, y], i) => {
        const [vx, vy] = viewport.convertToViewportPoint(x, y);
        if (i === 0) ctx.moveTo(vx, vy); else ctx.lineTo(vx, vy);
      });
      ctx.stroke();
    }
  };
  redraw();
  c.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    c.setPointerCapture(e.pointerId);
    const stroke = [];
    inkStrokes.push(stroke);
    const add = (ev) => {
      const p = localPoint(ev);
      stroke.push(boxToPdfPoint(p.x, p.y));
      redraw();
    };
    add(e);
    const move = (ev) => add(ev);
    const up = () => {
      c.removeEventListener('pointermove', move);
      c.removeEventListener('pointerup', up);
      if (stroke.length < 2) inkStrokes.pop();
      offerInkBar();
    };
    c.addEventListener('pointermove', move);
    c.addEventListener('pointerup', up);
  });
}

function offerInkBar() {
  if (!inkStrokes.length || !pageEl) return;
  const r = pageEl.getBoundingClientRect();
  showFloatBar({ left: r.left + 12, top: r.top + 12, bottom: r.top + 12 }, [
    pillButton(t('plugin.pdf.draw.keep', null, 'Save drawing'), 'apply', () => commitInk()),
    pillButton(t('plugin.pdf.draw.discard', null, 'Discard'), 'cancel', () => { inkStrokes = []; setupInkLayer(); closeFloatBar(); }),
  ]);
}

function commitInk() {
  const strokes = inkStrokes.filter((s) => s.length > 1);
  inkStrokes = [];
  closeFloatBar();
  if (!strokes.length || !currentPdf) return;
  void edit('ink', { page: currentPage, strokes, color: inkColor }, { okMsg: t('plugin.pdf.toast.drawingSaved', null, 'Drawing saved') });
}

function attachPagePointer(el) {
  el.addEventListener('pointerdown', async (e) => {
    if (e.button !== 0 || !currentPdf) return;
    // Focusing the window here would cancel a native text selection that is
    // just starting; only the tools take focus (for their shortcuts).
    if (mode !== 'select') tileEl?.focus({ preventScroll: true });
    // A tool's click belongs to the tool: it must not reach the document-level
    // "click outside closes the floating bar" handler.
    if (mode !== 'select' || e.shiftKey) e.stopPropagation();
    if (mode === 'text') { e.preventDefault(); placeTextBox(e); return; }
    if (mode === 'note') {
      e.preventDefault();
      const p = localPoint(e);
      const [x, y] = boxToPdfPoint(p.x, p.y);
      void addMark('note', [[x, y - 20, 20, 20]]);
      return;
    }
    if (mode === 'shape' || mode === 'link' || (mode === 'select' && e.shiftKey)) {
      e.preventDefault();
      const rect = await dragRect(e);
      if (!rect) return;
      if (mode === 'shape') void addMark(shapeKind, [rect], { color: inkColor });
      else if (mode === 'link') void addMark('link', [rect]);
      else void addMark('box', [rect]);
    }
  });
  el.addEventListener('pointerup', (e) => {
    if (mode !== 'select') return;
    setTimeout(() => {
      if (selectionRects().length) { offerSelectionMarks(); return; }
      // A click on a comment shows it in the panel.
      const hit = commentAt(e);
      if (hit) { if (!showPanel) togglePanel(true); focusComment(hit); }
    }, 0);
  });
}

/* ── Comments panel ─────────────────────────────────────────── */

function commentAt(e) {
  if (!pageEl || !viewport || !comments.length) return null;
  const p = localPoint(e);
  return comments.find((c) => {
    if (c.page !== currentPage) return false;
    const b = pdfRectToBox(c.rect);
    return p.x >= b.left - 2 && p.x <= b.left + b.width + 2 && p.y >= b.top - 2 && p.y <= b.top + b.height + 2;
  }) || null;
}

async function loadComments() {
  if (!currentPdf) return;
  const target = currentPdf;
  try {
    const data = (await apiFetch(`/api/pdfs/${enc(target.pdf_id)}/comments`))?.data;
    if (currentPdf !== target) return;
    comments = data?.comments || [];
    marks = data?.marks || [];
  } catch (_) {
    comments = [];
    marks = [];
  }
  renderPanel();
}

function togglePanel(on = !showPanel) {
  showPanel = on;
  commentsBtn?.classList.toggle('is-active', on);
  commentsBtn?.setAttribute('aria-pressed', String(on));
  if (panelEl) panelEl.hidden = !on;
  if (on) void loadComments();
}

const KIND_OF = {
  Highlight: 'highlight', Underline: 'underline', StrikeOut: 'strikeout', Squiggly: 'squiggly', Text: 'note',
  FreeText: 'free_text', Square: 'box', Circle: 'oval', Ink: 'ink', Link: 'link',
};

function renderPanel() {
  if (!panelEl) return;
  panelEl.innerHTML = '';
  const head = document.createElement('div');
  head.className = 'pdf-panel-head';
  const top = comments.filter((x) => !x.parent);
  head.textContent = tp('plugin.pdf.panel.count', top.length, null, `${top.length} comments`);
  panelEl.appendChild(head);

  if (marks.length) {
    const box = document.createElement('div');
    box.className = 'pdf-panel-marks';
    for (const m of marks) {
      const row = document.createElement('div');
      row.className = 'pdf-panel-mark';
      const label = document.createElement('span');
      label.textContent = m === 'watermark' ? t('plugin.pdf.panel.watermark', null, 'Watermark') : m === 'header_footer' ? t('plugin.pdf.panel.headerFooter', null, 'Header & footer') : m;
      const rm = pillButton(t('plugin.pdf.panel.remove', null, 'Remove'), 'pill', () => void edit('marks/remove', { kind: m }, { okMsg: t('plugin.pdf.toast.markRemoved', null, 'Removed') }));
      row.append(label, rm);
      box.appendChild(row);
    }
    panelEl.appendChild(box);
  }

  if (!top.length) {
    const empty = document.createElement('p');
    empty.className = 'pdf-panel-empty';
    empty.textContent = t('plugin.pdf.panel.empty', null, 'No comments yet. Select text to highlight it, or use the comment tool.');
    panelEl.appendChild(empty);
    return;
  }
  const list = document.createElement('div');
  list.className = 'pdf-panel-list';
  let lastPage = -1;
  for (const c of top) {
    if (c.page !== lastPage) {
      lastPage = c.page;
      const ph = document.createElement('div');
      ph.className = 'pdf-panel-page';
      ph.textContent = t('plugin.pdf.page.label', { page: c.page + 1 }, `Page ${c.page + 1}`);
      list.appendChild(ph);
    }
    list.appendChild(commentCard(c));
  }
  panelEl.appendChild(list);
}

function commentCard(c) {
  const card = document.createElement('div');
  card.className = 'pdf-comment';
  card.dataset.key = `${c.page}:${c.index}`;
  card.tabIndex = 0;
  const swatch = document.createElement('span');
  swatch.className = 'pdf-comment-swatch';
  if (c.color) swatch.style.background = `rgb(${c.color.map((v) => Math.round(v * 255)).join(',')})`;
  const kind = document.createElement('span');
  kind.className = 'pdf-comment-kind';
  kind.textContent = markLabel(KIND_OF[c.kind] || c.kind);
  const text = document.createElement('div');
  text.className = 'pdf-comment-text';
  text.textContent = c.text || '';
  const replies = comments.filter((x) => x.parent && x.parent[0] === c.page && x.parent[1] === c.index);
  const actions = document.createElement('div');
  actions.className = 'pdf-comment-actions';
  const act = (label, fn) => {
    const b = controlButton();
    b.type = 'button';
    b.className = 'pdf-comment-btn';
    b.textContent = label;
    b.addEventListener('click', (e) => { e.stopPropagation(); fn(); });
    actions.appendChild(b);
  };
  if (c.kind !== 'Link') {
    act(t('plugin.pdf.panel.edit', null, 'Edit'), async () => {
      const v = await promptDialog(t('plugin.pdf.dialog.noteText', null, 'Comment:'), c.text || '');
      if (v == null) return;
      void edit('comments/edit', { page: c.page, index: c.index, action: 'text', text: v }, { page: c.page });
    });
    act(t('plugin.pdf.panel.reply', null, 'Reply'), async () => {
      const v = await promptDialog(t('plugin.pdf.dialog.reply', null, 'Reply:'), '');
      if (v == null || !v.trim()) return;
      void edit('comments/edit', { page: c.page, index: c.index, action: 'reply', text: v.trim() }, { page: c.page });
    });
  }
  act(t('plugin.pdf.panel.delete', null, 'Delete'), () => void edit('comments/edit', { page: c.page, index: c.index, action: 'delete' }, {
    page: c.page, okMsg: t('plugin.pdf.toast.commentDeleted', null, 'Comment deleted — Ctrl+Z to undo'),
  }));
  card.append(swatch, kind, text);
  for (const r of replies) {
    const rep = document.createElement('div');
    rep.className = 'pdf-comment-reply';
    rep.textContent = r.text || '';
    card.appendChild(rep);
  }
  card.appendChild(actions);
  card.addEventListener('click', () => focusComment(c));
  return card;
}

function focusComment(c) {
  panelEl?.querySelectorAll('.pdf-comment.is-active').forEach((el) => el.classList.remove('is-active'));
  const card = panelEl?.querySelector(`.pdf-comment[data-key="${c.page}:${c.index}"]`);
  card?.classList.add('is-active');
  card?.scrollIntoView({ block: 'nearest' });
  const flash = () => {
    if (!pageEl || !viewport) return;
    const b = pdfRectToBox(c.rect);
    const f = document.createElement('div');
    f.className = 'pdf-comment-flash';
    Object.assign(f.style, { left: `${b.left - 3}px`, top: `${b.top - 3}px`, width: `${b.width + 6}px`, height: `${b.height + 6}px` });
    pageEl.appendChild(f);
    setTimeout(() => f.remove(), 1400);
  };
  if (c.page !== currentPage) { currentPage = c.page; void renderMain().then(flash); } else flash();
}

/* ── Find & replace (PDFs from elsewhere) ───────────────────── */

function openFindReplace(prefill = '') {
  if (!currentPdf) return;
  if (editable()) {
    // Documents made here are changed in the editor, never in the PDF.
    setView('write');
    toast(t('plugin.pdf.toast.useEditor', null, 'Change the text here, then update the PDF'), { type: 'info' });
    return;
  }
  const find = uiInput({ value: prefill, placeholder: t('plugin.pdf.find.find', null, 'Find'), label: t('plugin.pdf.find.find', null, 'Find'), autocomplete: 'off' });
  const repl = uiInput({ value: '', placeholder: t('plugin.pdf.find.replace', null, 'Replace with'), label: t('plugin.pdf.find.replace', null, 'Replace with'), autocomplete: 'off' });
  const scope = segmented({
    label: t('plugin.pdf.find.scope', null, 'Where'),
    options: [
      { value: 'all', label: t('plugin.pdf.find.allPages', null, 'All pages') },
      { value: 'page', label: t('plugin.pdf.find.thisPage', { page: currentPage + 1 }, `Page ${currentPage + 1}`) },
    ],
    value: 'all',
  });
  const note = document.createElement('p');
  note.className = 'ui-hint';
  note.textContent = t('plugin.pdf.find.note', null, 'Replaces the text in place, keeping its font. Letters the PDF does not contain cannot be typed.');
  const form = document.createElement('form');
  form.className = 'ui-stack pdf-find-form';
  const actions = document.createElement('div');
  actions.className = 'ui-row ui-dialog-actions';
  const dlg = modal({ title: t('plugin.pdf.find.title', null, 'Find & replace'), body: form, removeOnClose: true });
  actions.append(
    button({ label: t('plugin.pdf.edit.cancel', null, 'Cancel'), variant: 'ghost', onClick: () => dlg.close() }),
    button({ label: t('plugin.pdf.find.go', null, 'Replace'), type: 'submit' }),
  );
  form.append(find, repl, scope, note, actions);
  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    const old = find.value.trim();
    if (!old) { find.focus(); return; }
    dlg.close();
    const body = scope.value === 'all' ? { old, new: repl.value, all_pages: true } : { old, new: repl.value, page: currentPage };
    await edit('replace-text', body, { okMsg: (res) => tp('plugin.pdf.toast.replaced', res?.replaced ?? 0, null, `Replaced ${res?.replaced ?? 0}`) });
  });
  dlg.open();
  (prefill ? repl : find).focus();
}

/* ── Watermark / header & footer ────────────────────────────── */

async function promptWatermark() {
  if (!currentPdf) return;
  const target = currentPdf;
  const text = await promptDialog(t('plugin.pdf.dialog.watermarkText', null, 'Watermark text (all pages):'), t('plugin.pdf.dialog.watermarkDefault', null, 'DRAFT'));
  if (text == null || !text.trim() || currentPdf !== target) return;
  if (view !== 'pdf') setView('pdf');
  await edit('watermark', { text: text.trim(), all: true }, { okMsg: t('plugin.pdf.toast.watermarkApplied', null, 'Watermark added — remove it from the Comments panel') });
}

function openHeaderFooter() {
  if (!currentPdf) return;
  const presets = [
    ['', '', '', '', '<<1>> / <<n>>', ''],
    ['', '', '', '', t('plugin.pdf.hf.pageOf', null, 'Page <<1>> of <<n>>'), ''],
    [currentPdf.title || '', '', '<<dd.mm.yyyy>>', '', '', '<<1>>'],
  ];
  const names = ['headerLeft', 'headerCenter', 'headerRight', 'footerLeft', 'footerCenter', 'footerRight'];
  const fallback = ['Header left', 'Header centre', 'Header right', 'Footer left', 'Footer centre', 'Footer right'];
  const fields = names.map((k, i) => {
    const el = uiInput({ value: presets[0][i], label: t(`plugin.pdf.hf.${k}`, null, fallback[i]), placeholder: t(`plugin.pdf.hf.${k}`, null, fallback[i]), autocomplete: 'off' });
    el.classList.add('pdf-hf-field');
    return el;
  });
  const preset = segmented({
    label: t('plugin.pdf.hf.preset', null, 'Preset'),
    options: [
      { value: '0', label: '1 / 3' },
      { value: '1', label: t('plugin.pdf.hf.presetPageOf', null, 'Page 1 of 3') },
      { value: '2', label: t('plugin.pdf.hf.presetTitle', null, 'Title · date · number') },
    ],
    value: '0',
    onChange: (v) => fields.forEach((f, i) => { f.value = presets[Number(v)][i]; }),
  });
  const grid = document.createElement('div');
  grid.className = 'pdf-hf-grid';
  const lab = (txt) => { const s = document.createElement('span'); s.className = 'pdf-hf-label'; s.textContent = txt; return s; };
  grid.append(lab(t('plugin.pdf.hf.header', null, 'Header')), fields[0], fields[1], fields[2], lab(t('plugin.pdf.hf.footer', null, 'Footer')), fields[3], fields[4], fields[5]);
  const note = document.createElement('p');
  note.className = 'ui-hint';
  note.textContent = t('plugin.pdf.hf.tokens', null, '<<1>> page number · <<n>> page count · <<dd.mm.yyyy>> date');
  const form = document.createElement('form');
  form.className = 'ui-stack';
  const actions = document.createElement('div');
  actions.className = 'ui-row ui-dialog-actions';
  const dlg = modal({ title: t('plugin.pdf.hf.title', null, 'Header & footer'), body: form, wide: true, removeOnClose: true });
  actions.append(
    button({ label: t('plugin.pdf.edit.cancel', null, 'Cancel'), variant: 'ghost', onClick: () => dlg.close() }),
    button({ label: t('plugin.pdf.hf.apply', null, 'Apply to all pages'), type: 'submit' }),
  );
  form.append(preset, grid, note, actions);
  form.addEventListener('submit', (ev) => {
    ev.preventDefault();
    dlg.close();
    if (view !== 'pdf') setView('pdf');
    void edit('header-footer', { slots: fields.map((f) => f.value), font_size: 9 }, { okMsg: t('plugin.pdf.toast.hfApplied', null, 'Header & footer added') });
  });
  dlg.open();
}

/* ── New document ───────────────────────────────────────────── */

const TEMPLATE_ICON = { blank: 'ui/doc', letter: 'ui/mail', invoice: 'ui/calc', report: 'ui/heading', notes: 'ui/list' };
const TEMPLATE_THEME = { letter: 'classic', report: 'editorial' };
const TEMPLATE_NAME = { blank: 'Blank', letter: 'Letter', invoice: 'Invoice', report: 'Report', notes: 'Notes' };
const TEMPLATE_SUB = { blank: 'An empty page', letter: 'Date, greeting and signature', invoice: 'Items, quantities and total', report: 'Summary, details, conclusions', notes: 'A quick list' };

/** Pick a starting point; the document opens in the editor. */
function openNewDocument() {
  const grid = document.createElement('div');
  grid.className = 'pdf-new-grid';
  const dlg = modal({ title: t('plugin.pdf.new.title', null, 'New document'), body: grid, wide: true, removeOnClose: true });
  const make = async (id) => {
    dlg.close();
    try {
      const title = id === 'blank' ? t('plugin.pdf.toolbar.titlePlaceholder', null, 'Untitled') : t(`plugin.pdf.composer.tpl.${id}`, null, TEMPLATE_NAME[id]);
      const content = `<!--pdfdoc theme=${TEMPLATE_THEME[id] || 'modern'}-->${templateHtml(id, t)}`;
      const created = await post('/api/pdfs', { title, content, format: 'html' });
      await refreshDocs();
      await openDoc(created, { view: 'write' });
    } catch (e) {
      toast(e.message || t('plugin.pdf.toast.createFailed', null, 'Could not create the document'), { type: 'error' });
    }
  };
  const card = (id) => {
    const b = controlButton();
    b.type = 'button';
    b.className = 'pdf-new-card';
    const ic = document.createElement('span');
    ic.className = 'pdf-new-icon';
    ic.appendChild(icon(TEMPLATE_ICON[id], { size: 20 }));
    const tt = document.createElement('span');
    tt.className = 'pdf-new-title';
    tt.textContent = t(`plugin.pdf.composer.tpl.${id}`, null, TEMPLATE_NAME[id]);
    const st = document.createElement('span');
    st.className = 'pdf-new-sub';
    st.textContent = t(`plugin.pdf.new.${id}Sub`, null, TEMPLATE_SUB[id]);
    b.append(ic, tt, st);
    b.addEventListener('click', () => void make(id));
    return b;
  };
  for (const id of TEMPLATES) grid.appendChild(card(id));
  const imp = controlButton();
  imp.type = 'button';
  imp.className = 'pdf-new-card pdf-new-card--import';
  const ic = document.createElement('span');
  ic.className = 'pdf-new-icon';
  ic.appendChild(icon('ui/folder-open', { size: 20 }));
  const it = document.createElement('span');
  it.className = 'pdf-new-title';
  it.textContent = t('plugin.pdf.new.import', null, 'Open a PDF…');
  const is = document.createElement('span');
  is.className = 'pdf-new-sub';
  is.textContent = t('plugin.pdf.new.importSub', null, 'Read, comment and organise an existing PDF');
  imp.append(ic, it, is);
  imp.addEventListener('click', () => { dlg.close(); void pickPdfFile(); });
  grid.appendChild(imp);
  dlg.open();
}

/* ── Thumbnails & page actions ──────────────────────────────── */

async function renderThumbInto(img, index, boxWidth) {
  if (!currentPdf || !img.isConnected) return;
  const doc = await getPdfDoc();
  const page = await doc.getPage(index + 1);
  const base = page.getViewport({ scale: 1, rotation: page.rotate });
  const s = (boxWidth / base.width) * Math.min(2, window.devicePixelRatio || 1);
  const vp = page.getViewport({ scale: s, rotation: page.rotate });
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.floor(vp.width));
  canvas.height = Math.max(1, Math.floor(vp.height));
  await page.render({ canvasContext: canvas.getContext('2d'), viewport: vp }).promise;
  if (img.isConnected) img.src = canvas.toDataURL('image/jpeg', 0.82);
}

function renderThumbs({ reveal = false } = {}) {
  if (!currentPdf || !railEl) return;
  thumbObserver?.disconnect();
  const n = currentPdf.page_count;
  railEl.innerHTML = '';
  const width = railEl.clientWidth || 120;
  thumbObserver = new IntersectionObserver((entries) => {
    for (const en of entries) {
      if (!en.isIntersecting) continue;
      thumbObserver.unobserve(en.target);
      const img = en.target.querySelector('img');
      const i = Number(en.target.dataset.page);
      queueRender(() => renderThumbInto(img, i, width)).catch(() => {});
    }
  }, { root: railEl, rootMargin: '400px 0px' });

  for (let i = 0; i < n; i++) {
    const thumb = document.createElement('div');
    thumb.className = 'pdf-thumb';
    thumb.dataset.page = String(i);
    thumb.draggable = true;
    thumb.tabIndex = 0;
    if (i === currentPage) thumb.classList.add('is-active');
    const frame = document.createElement('span');
    frame.className = 'pdf-thumb-frame';
    const img = document.createElement('img');
    img.className = 'pdf-thumb-img';
    img.alt = t('plugin.pdf.page.label', { page: i + 1 }, `Page ${i + 1}`);
    img.draggable = false;
    const more = controlButton();
    more.type = 'button';
    more.className = 'pdf-thumb-more';
    more.title = t('plugin.pdf.rail.pageActions', null, 'Page actions');
    more.setAttribute('aria-label', t('plugin.pdf.rail.pageActionsAria', { page: i + 1 }, `Actions for page ${i + 1}`));
    more.appendChild(icon('ui/more', { size: 12 }));
    more.addEventListener('click', (e) => { e.stopPropagation(); openPageMenu(i, more); });
    frame.append(img, more);
    const num = document.createElement('span');
    num.className = 'pdf-thumb-num';
    num.textContent = String(i + 1).padStart(2, '0');
    thumb.append(frame, num);
    thumb.addEventListener('click', () => gotoPage(i - currentPage));
    thumb.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      openPageMenu(i, { getBoundingClientRect: () => ({ left: e.clientX, top: e.clientY, bottom: e.clientY, right: e.clientX, width: 0, height: 0 }) });
    });
    thumb.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') gotoPage(i - currentPage);
      if (e.key === 'Delete' || e.key === 'Backspace') void removePage(i);
      if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) { e.preventDefault(); openPageMenu(i, thumb); }
    });
    thumb.addEventListener('dragstart', (e) => {
      dragIndex = i;
      e.dataTransfer.effectAllowed = 'move';
      try { e.dataTransfer.setData('text/plain', String(i)); } catch (_) { /* no-op */ }
      thumb.classList.add('is-dragging');
    });
    thumb.addEventListener('dragend', () => { dragIndex = null; thumb.classList.remove('is-dragging'); });
    thumb.addEventListener('dragover', (e) => {
      if (dragIndex == null) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
      thumb.classList.toggle('is-drop-before', i < dragIndex);
      thumb.classList.toggle('is-drop-after', i > dragIndex);
    });
    thumb.addEventListener('dragleave', () => thumb.classList.remove('is-drop-before', 'is-drop-after'));
    thumb.addEventListener('drop', (e) => {
      thumb.classList.remove('is-drop-before', 'is-drop-after');
      if (dragIndex == null) return;
      e.preventDefault();
      e.stopPropagation();
      const from = dragIndex;
      dragIndex = null;
      if (from === i) return;
      const order = [...Array(n).keys()];
      order.splice(from, 1);
      order.splice(i, 0, from);
      void edit('reorder', { order }, { page: i });
    });
    railEl.appendChild(thumb);
    thumbObserver.observe(thumb);
  }
  const add = controlButton();
  add.type = 'button';
  add.className = 'pdf-thumb-add';
  add.title = t('plugin.pdf.rail.addPage', null, 'Add a blank page at the end');
  add.setAttribute('aria-label', add.title);
  add.appendChild(icon('ui/plus', { size: 14 }));
  add.addEventListener('click', () => void insertBlank(n - 1));
  railEl.appendChild(add);
  if (reveal) void listReveal(railEl.children);
}

function markActiveThumb() {
  railEl?.querySelectorAll('.pdf-thumb').forEach((el) => el.classList.toggle('is-active', Number(el.dataset.page) === currentPage));
  railEl?.querySelector('.pdf-thumb.is-active')?.scrollIntoView({ block: 'nearest' });
}

function closePageMenu() {
  pageMenu?.remove();
  pageMenu = null;
  document.removeEventListener('pointerdown', onPageMenuOutside, true);
}
function onPageMenuOutside(e) { if (pageMenu && !pageMenu.contains(e.target)) closePageMenu(); }

function openPageMenu(page, anchor) {
  closePageMenu();
  const menu = menuSurface({ className: 'pdf-doc-menu pdf-page-menu', onEscape: closePageMenu });
  const item = (label, iconName, onClick, danger = false) => menu.appendChild(docMenuItem(label, iconName, { danger, onClick: () => { closePageMenu(); onClick(); } }));
  item(t('plugin.pdf.page.rotateLeft', null, 'Rotate left'), 'ui/rotate-left', () => void edit('rotate', { pages: [page], degrees: -90 }, { page }));
  item(t('plugin.pdf.page.rotateRight', null, 'Rotate right'), 'ui/rotate-right', () => void edit('rotate', { pages: [page], degrees: 90 }, { page }));
  item(t('plugin.pdf.page.duplicate', null, 'Duplicate page'), 'ui/copy', () => void edit('duplicate-page', { page }, { page: page + 1 }));
  item(t('plugin.pdf.page.insertBlank', null, 'Insert blank page after'), 'ui/plus', () => void insertBlank(page));
  item(t('plugin.pdf.page.extract', null, 'Extract to new PDF'), 'ui/upload', () => void extractPages([page]));
  item(t('plugin.pdf.page.delete', null, 'Delete page'), 'ui/trash', () => void removePage(page), true);
  menu.classList.remove('hidden');
  document.body.appendChild(menu);
  placePopover(menu, anchor);
  menu.focusFirst?.();
  pageMenu = menu;
  setTimeout(() => document.addEventListener('pointerdown', onPageMenuOutside, true), 0);
}

/* ── Documents ──────────────────────────────────────────────── */

async function openDoc(doc, { page = 0, view: wanted } = {}) {
  if (!doc) return;
  try {
    const id = doc.pdf_id || doc.id;
    if (editor) { await editor.saveNow(); closeEditor(); }
    const meta = await fetchPdf(id);
    currentPdf = { ...doc, ...meta, pdf_id: id };
    currentPage = clampPage(page);
    fitZoom = null;
    comments = [];
    marks = [];
    if (titleInput) titleInput.value = currentPdf.title;
    applyHistory(meta?.history);
    dropPdfDoc();
    lastEntered = '';
    // A document whose PDF has not been made yet opens in the editor.
    const next = wanted || (editable() && currentPdf.pdf_stale ? 'write' : 'pdf');
    setView(next);
    if (view === 'pdf') renderThumbs({ reveal: true });
    else if (railEl) railEl.innerHTML = '';
    if (showPanel) void loadComments();
    showSaved();
  } catch (e) {
    toast(e.message || t('plugin.pdf.toast.openFailed', null, 'Could not open the document'), { type: 'error' });
  }
}

async function refreshDocs() {
  try { docs = await listPdfs(); } catch (_) { /* keep */ }
}

function showEmptyWindow() {
  closeEditor();
  currentPdf = null;
  currentPage = 0;
  applyHistory(null);
  setTileGlow(tileEl, null);
  setView('pdf');
  if (titleInput) titleInput.value = '';
  if (railEl) railEl.innerHTML = '';
  if (!canvasEl) return;
  canvasEl.innerHTML = '';
  const actions = document.createElement('div');
  actions.className = 'ui-row pdf-empty-actions';
  actions.append(
    button({ label: t('plugin.pdf.empty.create', null, 'Write a document'), icon: 'ui/plus', onClick: openNewDocument }),
    button({ label: t('plugin.pdf.empty.import', null, 'Open a PDF'), icon: 'ui/folder-open', variant: 'ghost', onClick: pickPdfFile }),
  );
  canvasEl.appendChild(emptyState({
    icon: 'ui/doc',
    title: t('plugin.pdf.empty.noPdfs', null, 'No documents yet'),
    body: t('plugin.pdf.empty.noPdfsBody', null, 'Write a document and make it a PDF, or open a PDF to read and comment on it. You can also drop a .pdf here.'),
    action: actions,
  }));
  updateNav();
  showSaved();
}

async function openNewest() {
  await refreshDocs();
  if (docs.length) await openDoc(docs[0]);
  else showEmptyWindow();
}

async function pickPdfFile() {
  const files = await pickFiles({ accept: '.pdf,application/pdf', multiple: true });
  for (const f of files || []) await importPdf(f);
}

async function importPdf(file) {
  setBusy(true);
  try {
    const form = new FormData();
    form.append('file', file);
    const res = await apiFetch('/api/pdfs/import', { method: 'POST', body: form });
    await refreshDocs();
    await openDoc(res?.data, { view: 'pdf' });
    toast(t('plugin.pdf.toast.imported', { name: file.name }, `Opened ${file.name}`), { type: 'info' });
  } catch (e) {
    toast(e.message || t('plugin.pdf.toast.importFailed', null, 'Could not open — is this a valid .pdf?'), { type: 'error' });
    setBusy(false);
  }
}

async function exportCurrent() {
  if (!currentPdf) return;
  try {
    if (editable() && currentPdf.pdf_stale) await exportCurrentDoc();
    const blob = await apiFetch(`/api/pdfs/${enc(currentPdf.pdf_id)}/export`, { responseType: 'blob' });
    await saveOrDownload(blob, { name: `${currentPdf.title || 'document'}.pdf`, dir: 'Documents', app: 'PDF' });
  } catch (e) {
    toast(e.message || t('plugin.pdf.toast.saveFailed', null, 'Save failed'), { type: 'error' });
  }
}

async function removeCurrent() {
  if (!currentPdf) return;
  const target = currentPdf;
  if (!await confirmDialog(t('plugin.pdf.dialog.deleteDoc', { title: target.title }, `Delete "${target.title}"? This cannot be undone.`))) return;
  if (currentPdf !== target || !tileEl) return;
  try {
    closeEditor();
    await apiFetch(`/api/pdfs/${enc(target.pdf_id)}`, { method: 'DELETE' });
    dropPdfDoc();
    currentPdf = null;
    await openNewest();
  } catch (e) {
    toast(e.message || t('plugin.pdf.toast.deleteFailed', null, 'Could not delete'), { type: 'error' });
  }
}

async function renameCurrent(title) {
  if (!currentPdf) return;
  const clean = title.trim() || t('plugin.pdf.toolbar.titlePlaceholder', null, 'Untitled');
  try {
    const res = await post(`/api/pdfs/${enc(currentPdf.pdf_id)}`, { title: clean }, 'PUT');
    currentPdf.title = res?.title || clean;
    if (titleInput) titleInput.value = currentPdf.title;
    editor?.setTitle(currentPdf.title);
    const entry = docs.find((d) => d.pdf_id === currentPdf.pdf_id);
    if (entry) entry.title = currentPdf.title;
  } catch (e) {
    toast(e.message || t('plugin.pdf.toast.renameFailed', null, 'Could not rename'), { type: 'error' });
  }
}

function removePage(page) {
  if (!currentPdf) return;
  if (currentPdf.page_count <= 1) { toast(t('plugin.pdf.toast.onlyPage', null, 'Cannot delete the only page'), { type: 'error' }); return; }
  return edit('delete-pages', { pages: [page] }, { page, okMsg: t('plugin.pdf.toast.pageDeleted', { page: page + 1 }, `Page ${page + 1} deleted — Ctrl+Z to undo`) });
}

function insertBlank(after) {
  return edit('insert-blank', { after, count: 1 }, { page: after + 1, okMsg: t('plugin.pdf.toast.pageAdded', null, 'Blank page added') });
}

async function extractPages(pages) {
  if (!currentPdf) return;
  try {
    const res = await post(`/api/pdfs/${enc(currentPdf.pdf_id)}/extract`, { pages });
    await refreshDocs();
    toast(t('plugin.pdf.toast.extracted', { title: res.title }, `Saved as "${res.title}"`), { type: 'info' });
  } catch (e) {
    toast(e.message || t('plugin.pdf.toast.extractFailed', null, 'Could not extract the page'), { type: 'error' });
  }
}

function zoomBy(delta) {
  if (fitZoom != null) {
    const i = ZOOMS.findIndex((z) => z >= fitZoom);
    zoomIndex = Math.max(0, Math.min(ZOOMS.length - 1, (i < 0 ? ZOOMS.length : i) + (delta > 0 ? 0 : -1)));
    fitZoom = null;
  } else {
    zoomIndex = Math.max(0, Math.min(ZOOMS.length - 1, zoomIndex + delta));
  }
  updateNav();
  void renderMain();
}

function fitWidth() { fitZoom = 100; void renderMain(); }

function gotoPage(delta) {
  if (!currentPdf) return;
  const next = clampPage(currentPage + delta);
  if (next === currentPage) return;
  if (mode === 'draw') commitInk();
  currentPage = next;
  closeFloatBar();
  void renderMain();
}

/* ── Keyboard ───────────────────────────────────────────────── */

function onTileKey(e) {
  if (view === 'write') return; // the editor owns its keys
  const typing = e.target.closest?.('input, textarea, select, [contenteditable="true"], [contenteditable="plaintext-only"]');
  const mod = e.ctrlKey || e.metaKey;
  if (mod && !typing && e.key.toLowerCase() === 'z') { e.preventDefault(); void stepHistory(!e.shiftKey); return; }
  if (mod && !typing && e.key.toLowerCase() === 'y') { e.preventDefault(); void stepHistory(false); return; }
  if (mod && (e.key === '+' || e.key === '=')) { e.preventDefault(); zoomBy(1); return; }
  if (mod && e.key === '-') { e.preventDefault(); zoomBy(-1); return; }
  if (mod && e.key === '0') { e.preventDefault(); fitWidth(); return; }
  if (mod && e.key.toLowerCase() === 'f' && !typing) { e.preventDefault(); openFindReplace(); return; }
  if (typing || mod) return;
  if (e.key === 'Escape') {
    if (floatBar) { closeFloatBar(); return; }
    if (mode !== 'select') { setMode('select'); return; }
  }
  if (e.key === 'PageDown' || (e.key === 'ArrowRight' && !e.shiftKey)) { e.preventDefault(); gotoPage(1); }
  if (e.key === 'PageUp' || (e.key === 'ArrowLeft' && !e.shiftKey)) { e.preventDefault(); gotoPage(-1); }
  if (e.key === 'Home') { e.preventDefault(); gotoPage(-currentPage); }
  if (e.key === 'End') { e.preventDefault(); gotoPage((currentPdf?.page_count ?? 1) - 1 - currentPage); }
  const keys = { c: 'note', t: 'text', d: 'draw', r: 'shape', l: 'link' };
  if (keys[e.key]) setMode(keys[e.key]);
}

/* ── Doc menu ───────────────────────────────────────────────── */

function ensureDocMenu() {
  if (docMenuPopup) return;
  docMenuPopup = menuSurface({ className: 'pdf-doc-menu', onEscape: closeDocMenu });
  document.body.appendChild(docMenuPopup);
}
function closeDocMenu() {
  if (!docMenuOpen) return;
  docMenuOpen = false;
  docMenuPopup?.classList.add('hidden');
  docMenuBtn?.setAttribute('aria-expanded', 'false');
  document.removeEventListener('pointerdown', onDocMenuOutside, true);
  document.removeEventListener('keydown', onDocMenuKey, true);
}
function onDocMenuOutside(e) {
  if (docMenuPopup && !docMenuPopup.contains(e.target) && docMenuBtn && !docMenuBtn.contains(e.target)) closeDocMenu();
}
function onDocMenuKey(e) { if (e.key === 'Escape') closeDocMenu(); }

function docMenuItem(title, iconName, opts = {}) {
  const item = menuItem({ className: 'pdf-doc-menu-item' });
  if (opts.danger) item.classList.add('pdf-doc-menu-item--danger');
  if (opts.disabled) item.setAttribute('aria-disabled', 'true');
  item.setAttribute('role', 'menuitem');
  const ic = document.createElement('span');
  ic.className = 'pdf-doc-menu-foot-icon';
  item.appendChild(ic);
  void setIcon(ic, iconName, { size: 14 });
  const labelEl = document.createElement('span');
  labelEl.className = 'pdf-doc-menu-title';
  labelEl.textContent = title;
  item.appendChild(labelEl);
  if (opts.time) {
    const time = document.createElement('span');
    time.className = 'pdf-doc-menu-time';
    time.textContent = opts.time;
    item.appendChild(time);
  }
  if (opts.active) {
    const check = document.createElement('span');
    check.className = 'pdf-doc-menu-check';
    item.appendChild(check);
    void setIcon(check, 'ui/check', { size: 13 });
  }
  item.addEventListener('click', () => { if (opts.disabled) return; closeDocMenu(); opts.onClick?.(); });
  return item;
}

function renderDocMenuItems() {
  if (!docMenuPopup) return;
  docMenuPopup.innerHTML = '';
  if (menuMode === 'merge') {
    const others = docs.filter((d) => d.pdf_id !== currentPdf?.pdf_id);
    const head = document.createElement('div');
    head.className = 'pdf-doc-menu-empty';
    head.textContent = others.length ? t('plugin.pdf.menu.mergeInto', null, 'Append to the end of this PDF:') : t('plugin.pdf.menu.noOthersToMerge', null, 'No other PDFs to append');
    docMenuPopup.appendChild(head);
    others.forEach((d) => docMenuPopup.appendChild(docMenuItem(d.title, 'ui/layers', {
      time: formatMenuDate(d.updated_at),
      onClick: () => { if (view !== 'pdf') setView('pdf'); void edit('merge', { other_id: d.pdf_id }, { okMsg: t('plugin.pdf.toast.merged', null, 'Pages appended') }); },
    })));
    const foot = document.createElement('div');
    foot.className = 'pdf-doc-menu-foot';
    foot.appendChild(docMenuItem(t('plugin.pdf.menu.back', null, 'Back'), 'ui/arrow-left', { onClick: () => { menuMode = 'default'; openDocMenu(); } }));
    docMenuPopup.appendChild(foot);
    return;
  }
  if (!docs.length) {
    const empty = document.createElement('div');
    empty.className = 'pdf-doc-menu-empty';
    empty.textContent = t('plugin.pdf.menu.empty', null, 'No documents yet');
    docMenuPopup.appendChild(empty);
  } else {
    docs.forEach((d) => docMenuPopup.appendChild(docMenuItem(d.title, d.editable ? 'ui/pencil' : 'ui/doc', {
      time: formatMenuDate(d.updated_at),
      active: d.pdf_id === currentPdf?.pdf_id,
      onClick: () => { if (d.pdf_id !== currentPdf?.pdf_id) void openDoc(d); },
    })));
  }
  const has = !!currentPdf;
  const foot = document.createElement('div');
  foot.className = 'pdf-doc-menu-foot';
  foot.appendChild(docMenuItem(t('plugin.pdf.menu.newDoc', null, 'New document…'), 'ui/plus', { onClick: openNewDocument }));
  foot.appendChild(docMenuItem(t('plugin.pdf.menu.importPdf', null, 'Open a PDF…'), 'ui/folder-open', { onClick: pickPdfFile }));
  foot.appendChild(docMenuItem(t('plugin.pdf.menu.saveToDocuments', null, 'Save PDF to Documents'), 'ui/upload', { disabled: !has, onClick: () => void exportCurrent() }));
  foot.appendChild(docMenuItem(t('plugin.pdf.menu.mergeFrom', null, 'Append another PDF…'), 'ui/layers', { disabled: !has, onClick: () => { menuMode = 'merge'; openDocMenu(); } }));
  foot.appendChild(docMenuItem(t('plugin.pdf.menu.headerFooter', null, 'Header & footer…'), 'ui/heading', { disabled: !has, onClick: openHeaderFooter }));
  foot.appendChild(docMenuItem(t('plugin.pdf.menu.watermark', null, 'Watermark…'), 'ui/droplet', { disabled: !has, onClick: promptWatermark }));
  if (!editable()) foot.appendChild(docMenuItem(t('plugin.pdf.menu.findReplace', null, 'Find & replace…'), 'ui/search', { disabled: !has, onClick: () => openFindReplace() }));
  foot.appendChild(docMenuItem(t('plugin.pdf.menu.deletePdf', null, 'Delete'), 'ui/trash', { danger: true, disabled: !has, onClick: () => void removeCurrent() }));
  docMenuPopup.appendChild(foot);
}

function openDocMenu() {
  ensureDocMenu();
  renderDocMenuItems();
  docMenuOpen = true;
  showDocumentMenu(docMenuPopup, docMenuBtn, onDocMenuOutside, onDocMenuKey);
}
function toggleDocMenu() {
  if (docMenuOpen) closeDocMenu();
  else { menuMode = 'default'; void refreshDocs().then(openDocMenu); }
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

function toolbarButton(iconName, label, onClick) { return toolButton(iconName, label, onClick, { className: 'pdf-tool' }); }
function cluster(className, ...nodes) {
  const el = document.createElement('div');
  el.className = `pdf-cluster ${className}`;
  el.append(...nodes);
  return el;
}

export function mountPdfTile() {
  if (tileEl) return tileEl;
  tileEl = document.createElement('section');
  tileEl.className = 'tile pdf-tile';
  tileEl.dataset.plugin = PDF_PLUGIN;
  tileEl.tabIndex = -1;
  tileEl.addEventListener('keydown', onTileKey);

  const bar = toolbar([], { className: 'pdf-bar' });
  bar.dataset.windowBar = '';
  docMenuBtn = menuButton({ icon: 'ui/doc', label: t('plugin.pdf.toolbar.documents', null, 'Documents'), onClick: toggleDocMenu, className: 'pdf-doc-btn' });

  titleInput = uiInput();
  titleInput.classList.add('pdf-title');
  titleInput.type = 'text';
  titleInput.placeholder = t('plugin.pdf.toolbar.titlePlaceholder', null, 'Untitled');
  titleInput.maxLength = 120;
  titleInput.autocomplete = 'off';
  titleInput.addEventListener('change', () => void renameCurrent(titleInput.value));
  titleInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') titleInput.blur(); });

  viewSwitch = segmented({
    label: t('plugin.pdf.view.label', null, 'View'),
    options: [
      { value: 'write', label: t('plugin.pdf.view.write', null, 'Write') },
      { value: 'pdf', label: t('plugin.pdf.view.pdf', null, 'PDF') },
    ],
    value: 'pdf',
    onChange: (v) => {
      if (v === 'pdf' && editor?.dirty) void editor.saveNow();
      setView(v);
    },
  });
  viewSwitch.classList.add('pdf-view-switch');
  viewSwitch.hidden = true;

  const prevBtn = toolbarButton('ui/chevron-left', t('plugin.pdf.toolbar.previousPage', null, 'Previous page'), () => gotoPage(-1));
  pageLabelEl = document.createElement('span');
  pageLabelEl.className = 'pdf-page-label';
  const nextBtn = toolbarButton('ui/chevron-right', t('plugin.pdf.toolbar.nextPage', null, 'Next page'), () => gotoPage(1));
  const zoomOut = toolbarButton('ui/zoom-out', t('plugin.pdf.toolbar.zoomOut', null, 'Zoom out'), () => zoomBy(-1));
  zoomLabelEl = controlButton();
  zoomLabelEl.type = 'button';
  zoomLabelEl.className = 'pdf-zoom-label';
  zoomLabelEl.title = t('plugin.pdf.toolbar.fitWidth', null, 'Fit to width (Ctrl+0)');
  zoomLabelEl.addEventListener('click', fitWidth);
  const zoomIn = toolbarButton('ui/zoom-in', t('plugin.pdf.toolbar.zoomIn', null, 'Zoom in'), () => zoomBy(1));

  for (const [m, def] of Object.entries(MODES)) {
    const b = toolbarButton(def.icon, t(`plugin.pdf.toolbar.mode.${m}`, null, def.fallback), () => setMode(m));
    b.classList.add(`pdf-tool--${m}`);
    b.setAttribute('aria-pressed', 'false');
    modeBtns[m] = b;
  }
  commentsBtn = toolbarButton('ui/message-circle', t('plugin.pdf.toolbar.comments', null, 'Comments panel'), () => togglePanel());
  commentsBtn.classList.add('pdf-tool--panel');
  commentsBtn.setAttribute('aria-pressed', 'false');
  pdfTools = cluster('pdf-cluster--pdf',
    cluster('pdf-cluster--nav', prevBtn, pageLabelEl, nextBtn),
    cluster('pdf-cluster--zoom', zoomOut, zoomLabelEl, zoomIn),
    cluster('pdf-cluster--quiet pdf-cluster--modes', modeBtns.note, modeBtns.text, modeBtns.draw, modeBtns.shape, modeBtns.link),
    cluster('pdf-cluster--quiet', commentsBtn));

  undoBtn = toolbarButton('ui/undo', t('plugin.pdf.toolbar.undo', null, 'Undo'), () => void stepHistory(true));
  redoBtn = toolbarButton('ui/redo', t('plugin.pdf.toolbar.redo', null, 'Redo'), () => void stepHistory(false));
  undoBtn.disabled = true;
  redoBtn.disabled = true;
  const hist = cluster('pdf-cluster--quiet pdf-cluster--history', undoBtn, redoBtn);

  freshnessEl = document.createElement('span');
  freshnessEl.className = 'pdf-freshness';
  exportBtn = button({ label: t('plugin.pdf.editor.export', null, 'Update PDF'), icon: 'ui/upload', iconSize: 14, size: 'sm', onClick: () => void exportCurrentDoc() });
  exportBtn.classList.add('pdf-export');
  writeTools = cluster('pdf-cluster--write', freshnessEl, exportBtn);
  writeTools.classList.add('hidden');

  const newBtn = button({ label: t('plugin.pdf.toolbar.new', null, 'New'), icon: 'ui/plus', iconSize: 14, variant: 'ghost', size: 'sm', onClick: openNewDocument });
  newBtn.classList.add('pdf-new');
  newBtn.title = t('plugin.pdf.toolbar.newDoc', null, 'New document');

  bar.append(docMenuBtn, titleInput, viewSwitch, pdfTools, hist, writeTools, newBtn);
  tileEl.appendChild(bar);

  editorToolbarSlot = document.createElement('div');
  editorToolbarSlot.className = 'pdf-editor-toolbar-slot';
  editorToolbarSlot.hidden = true;
  tileEl.appendChild(editorToolbarSlot);

  modeStripEl = document.createElement('div');
  modeStripEl.className = 'pdf-mode-strip';
  modeStripEl.hidden = true;
  tileEl.appendChild(modeStripEl);

  mainEl = document.createElement('div');
  mainEl.className = 'pdf-main';
  railEl = document.createElement('div');
  railEl.className = 'pdf-rail';
  railEl.setAttribute('aria-label', t('plugin.pdf.rail.pages', null, 'Pages'));
  canvasEl = document.createElement('div');
  canvasEl.className = 'pdf-canvas';
  canvasEl.dataset.mode = 'select';
  canvasEl.addEventListener('wheel', (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    e.preventDefault();
    zoomBy(e.deltaY < 0 ? 1 : -1);
  }, { passive: false });
  canvasEl.addEventListener('pointerdown', (e) => { if (e.target === canvasEl) closeFloatBar(); });
  panelEl = document.createElement('aside');
  panelEl.className = 'pdf-panel';
  panelEl.hidden = true;
  panelEl.setAttribute('aria-label', t('plugin.pdf.toolbar.comments', null, 'Comments'));
  mainEl.append(railEl, canvasEl, panelEl);
  tileEl.appendChild(mainEl);

  tileEl.addEventListener('dragover', (e) => {
    if (dragIndex != null || !e.dataTransfer?.types?.includes('Files')) return;
    e.preventDefault();
    tileEl.classList.add('is-dropping');
  });
  tileEl.addEventListener('dragleave', (e) => { if (e.target === tileEl || !tileEl.contains(e.relatedTarget)) tileEl.classList.remove('is-dropping'); });
  tileEl.addEventListener('drop', (e) => {
    tileEl.classList.remove('is-dropping');
    const files = [...(e.dataTransfer?.files || [])].filter((f) => /\.pdf$/i.test(f.name) || f.type === 'application/pdf');
    if (!files.length) return;
    e.preventDefault();
    void (async () => { for (const f of files) await importPdf(f); })();
  });

  const status = document.createElement('div');
  status.className = 'pdf-status';
  statusEl = document.createElement('span');
  status.appendChild(statusEl);
  tileEl.appendChild(status);

  updateNav();
  void openNewest();
  return tileEl;
}

export function unmountPdfTile() {
  closeDocMenu();
  closePageMenu();
  closeFloatBar();
  closeEditor();
  thumbObserver?.disconnect();
  thumbObserver = null;
  tileEl?.remove();
  tileEl = null;
  titleInput = null;
  docMenuBtn = null;
  statusEl = null;
  railEl = null;
  canvasEl = null;
  mainEl = null;
  panelEl = null;
  pageLabelEl = null;
  zoomLabelEl = null;
  modeStripEl = null;
  undoBtn = null;
  redoBtn = null;
  viewSwitch = null;
  pdfTools = null;
  writeTools = null;
  exportBtn = null;
  freshnessEl = null;
  commentsBtn = null;
  editorToolbarSlot = null;
  for (const k of Object.keys(modeBtns)) delete modeBtns[k];
  menuMode = 'default';
  mode = 'select';
  view = 'pdf';
  showPanel = false;
  lastEntered = '';
  inkStrokes = [];
  inkLayer = null;
  dropPdfDoc();
  pageEl = null;
  viewport = null;
}

export function getPdfTileElement() { return tileEl; }

/* ── AI wiring ──────────────────────────────────────────────── */

const MODIFYING = ['pdf_rotate', 'pdf_reorder', 'pdf_delete_pages', 'pdf_merge', 'pdf_replace_text', 'pdf_annotate', 'pdf_add_note', 'pdf_watermark', 'pdf_undo'];

function onAgentActions(e) {
  const actions = (e.detail || []).filter((a) => /^pdf_/.test(a?.action || '') && a.result === 'ok');
  if (!actions.length) return;
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: PDF_PLUGIN } }));
  const created = actions.find((a) => a.action === 'pdf_create');
  const modified = actions.filter((a) => MODIFYING.includes(a.action));
  const deleted = actions.some((a) => a.action === 'pdf_delete' && a.data?.pdf_id === currentPdf?.pdf_id);
  const touchedId = modified.map((a) => a.data?.pdf_id).filter(Boolean).pop();
  const page = modified.map((a) => a.data?.page).filter((p) => Number.isInteger(p)).pop();
  void refreshDocs().then(() => {
    if (created?.data?.pdf_id) void openDoc({ pdf_id: created.data.pdf_id }, { view: 'pdf' });
    else if (deleted) void openNewest();
    else if (touchedId && touchedId !== currentPdf?.pdf_id) void openDoc({ pdf_id: touchedId }, { page: page ?? 0, view: 'pdf' });
    else if (modified.length && currentPdf) void openDoc(currentPdf, { page: page ?? currentPage, view: 'pdf' });
  });
}

async function importFromFiles(path, name) {
  try {
    if (!getPdfTileElement()) mountPdfTile();
    const file = await fileFromHome(path, name, 'application/pdf');
    await importPdf(file);
  } catch (e) {
    toast(e.message || t('plugin.pdf.toast.openFileFailed', null, 'Could not open the file'), { type: 'error' });
  }
}

let wired = false;
export function wirePdfEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  onOpenFromFiles(PDF_PLUGIN, (d) => importFromFiles(d.path, d.name));
  document.addEventListener('pointerdown', (e) => {
    if (floatBar && !floatBar.contains(e.target) && !e.target.closest?.('.pdf-addtext, .pdf-text-layer, .pdf-ink-layer')) closeFloatBar();
  });
}

export function pdfContextMenu() {
  const hasPdf = !!currentPdf;
  const pages = currentPdf?.page_count ?? 0;
  return [
    { type: 'item', label: t('plugin.pdf.ctx.newDoc', null, 'New document…'), icon: 'ui/plus', onClick: openNewDocument },
    { type: 'item', label: t('plugin.pdf.ctx.importPdf', null, 'Open a PDF…'), icon: 'ui/download', onClick: pickPdfFile },
    { type: 'item', label: t('plugin.pdf.ctx.saveToDocuments', null, 'Save PDF to Documents'), icon: 'ui/save', disabled: !hasPdf, onClick: () => void exportCurrent() },
    { type: 'separator' },
    {
      type: 'submenu',
      label: t('plugin.pdf.ctx.page', null, 'Page'),
      icon: 'ui/doc',
      items: [
        { type: 'item', label: t('plugin.pdf.ctx.previousPage', null, 'Previous page'), icon: 'ui/chevron-left', disabled: !hasPdf || currentPage <= 0, onClick: () => gotoPage(-1) },
        { type: 'item', label: t('plugin.pdf.ctx.nextPage', null, 'Next page'), icon: 'ui/chevron-right', disabled: !hasPdf || currentPage >= pages - 1, onClick: () => gotoPage(1) },
        { type: 'separator' },
        { type: 'item', label: t('plugin.pdf.page.rotateRight', null, 'Rotate right'), icon: 'ui/rotate-right', disabled: !hasPdf, onClick: () => void edit('rotate', { pages: [currentPage], degrees: 90 }) },
        { type: 'item', label: t('plugin.pdf.page.duplicate', null, 'Duplicate page'), icon: 'ui/copy', disabled: !hasPdf, onClick: () => void edit('duplicate-page', { page: currentPage }, { page: currentPage + 1 }) },
        { type: 'item', label: t('plugin.pdf.page.insertBlank', null, 'Insert blank page after'), icon: 'ui/plus', disabled: !hasPdf, onClick: () => void insertBlank(currentPage) },
        { type: 'item', label: t('plugin.pdf.page.delete', null, 'Delete page'), icon: 'ui/trash', disabled: !hasPdf || pages <= 1, onClick: () => void removePage(currentPage) },
      ],
    },
    { type: 'separator' },
    { type: 'item', label: t('plugin.pdf.ctx.deletePdf', null, 'Delete'), icon: 'ui/trash', danger: true, disabled: !hasPdf, onClick: () => void removeCurrent() },
  ];
}

export default {
  name: 'pdf',
  icon: 'ui/doc',
  mount: mountPdfTile,
  unmount: unmountPdfTile,
  getElement: getPdfTileElement,
  wireEvents: wirePdfEvents,
  contextMenu: pdfContextMenu,
};
