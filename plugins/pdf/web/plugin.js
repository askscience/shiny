/**
 * pdf.js — the PDF plugin's window (pdf_oxide viewer + editor).
 *
 * Layout: a hairline top bar (doc menu + title + page nav + zoom + rotate +
 * file actions), a thumbnail rail on the left, and a scrollable page canvas.
 * Pages are rendered server-side to PNG by pdf_oxide and shown as images; the
 * rail doubles as the reorder/delete surface (drag to reorder, × to delete).
 *
 * AI wiring: `pdf_*` tool outcomes arrive via the `agent:actions` event — the
 * window refreshes its document list and opens what the AI touched.
 */

import {
  icon, button, emptyState, toast, spinner,
  setTileGlow, glowFromDrawable,
} from '/ui/index.js';
import { setIcon } from '/ui/index.js';
import { apiFetch } from '/js/api.js';
import { saveOrDownload, onOpenFromFiles, fileFromHome, pickFiles } from '/js/files.js';

export const PDF_PLUGIN = 'pdf';

/* ── pdf.js engine ──────────────────────────────────────────── */
/*
 * The window renders the real document in the browser rather than showing a
 * server-rendered bitmap: pages are painted to a canvas and a text layer is
 * laid over them, so text is selectable and zoom stays crisp at any level
 * without a round-trip per zoom step.
 *
 * The engine is a vendored dependency (web/vendor/pdfjs), loaded on first use
 * so the rest of the app never pays for it.
 */
let pdfjsLib = null;
let pdfjsLoading = null;

function loadPdfJs() {
  if (pdfjsLib) return Promise.resolve(pdfjsLib);
  if (!pdfjsLoading) {
    pdfjsLoading = import('/vendor/pdfjs/pdf.min.mjs').then((mod) => {
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

const ZOOMS = [75, 100, 120, 150, 200, 300];
const DEFAULT_ZOOM = 2; // 120%

let tileEl = null;
let titleInput = null;
let docMenuBtn = null;
let statusEl = null;
let saveDot = null;
let railEl = null;
let canvasEl = null;
let pageLabelEl = null;
let zoomLabelEl = null;
let editToggleBtn = null;

let docs = [];
let currentPdf = null; // { pdf_id, title, page_count, updated_at }
let currentPage = 0;
let zoomIndex = DEFAULT_ZOOM;

/* Popup (body-level, the tile clips overflow) */
let docMenuPopup = null;
let docMenuOpen = false;
let menuMode = 'default'; // 'default' | 'merge'

/* Loaded pdf.js document for `currentPdf` (null until opened). */
let pdfDoc = null;
/** Bumped on every main render so a stale async paint can be discarded. */
let renderSeq = 0;
let renderTask = null;
/** The rendered page's PDF-space size in points (bottom-left origin). */
let pageSizePt = null;
/** The element the rendered page lives in; selection maths measures this. */
let pageEl = null;
let textLayerEl = null;

let dragIndex = null;

/* Region selection + annotation state */
let selOverlay = null;
let selStart = null;         // {x,y} in displayed px relative to the page element
let selDisplayRect = null;   // viewport rect used to position the action bar
let annotBar = null;
let annotRect = null;        // [x, y, w, h] in PDF points (bottom-left origin)

/* ── API ────────────────────────────────────────────────────── */

async function listPdfs() {
  const res = await apiFetch('/api/pdfs');
  return res?.data || [];
}

async function fetchPdf(id) {
  const res = await apiFetch(`/api/pdfs/${encodeURIComponent(id)}`);
  return res?.data;
}

async function createPdf() {
  const res = await apiFetch('/api/pdfs', {
    method: 'POST',
    body: JSON.stringify({ title: 'Untitled', text: ' ' }),
  });
  return res?.data;
}

async function renamePdf(id, title) {
  await apiFetch(`/api/pdfs/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify({ title }),
  });
}

async function deletePdf(id) {
  await apiFetch(`/api/pdfs/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

async function importPdfFile(file) {
  const form = new FormData();
  form.append('file', file);
  const res = await apiFetch('/api/pdfs/import', { method: 'POST', body: form });
  return res?.data;
}

async function rotatePage(pages, degrees) {
  const res = await apiFetch(`/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/rotate`, {
    method: 'POST',
    body: JSON.stringify({ pages, degrees }),
  });
  return res?.data;
}

async function reorderPages(order) {
  const res = await apiFetch(`/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/reorder`, {
    method: 'POST',
    body: JSON.stringify({ order }),
  });
  return res?.data;
}

async function deletePages(pages) {
  const res = await apiFetch(`/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/delete-pages`, {
    method: 'POST',
    body: JSON.stringify({ pages }),
  });
  return res?.data;
}

async function mergeOther(otherId) {
  const res = await apiFetch(`/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/merge`, {
    method: 'POST',
    body: JSON.stringify({ other_id: otherId }),
  });
  return res?.data;
}

async function annotatePdf(kind, rect, text, format = null) {
  const body = { page: currentPage, kind, rect, text };
  // Formatting only means anything for text we add: inserted text carries its
  // own font operator, so size/weight/colour apply to it and to nothing else.
  if (format) {
    body.size = format.size;
    body.bold = format.bold;
    body.italic = format.italic;
    if (format.color) body.text_color = format.color;
  }
  const res = await apiFetch(`/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/annotate`, {
    method: 'POST',
    body: JSON.stringify(body),
  });
  return res?.data;
}

async function replaceTextApi(page, oldText, newText) {
  const res = await apiFetch(`/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/replace-text`, {
    method: 'POST',
    body: JSON.stringify({ page, old: oldText, new: newText }),
  });
  return res?.data;
}

async function watermarkApi(text) {
  const res = await apiFetch(`/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/watermark`, {
    method: 'POST',
    body: JSON.stringify({ text, all: true }),
  });
  return res?.data;
}

/** Every editable text run on a page, with geometry + real font styling. */
async function fetchTextRuns(page) {
  const res = await apiFetch(
    `/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/runs/${page}`,
  );
  return res?.data?.runs || [];
}

/** Apply in-place text edits to the current page. */
async function editTextRuns(edits) {
  const res = await apiFetch(`/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/edit-text`, {
    method: 'POST',
    body: JSON.stringify({ page: currentPage, edits }),
  });
  return res?.data;
}

/* ── Inline text editing ────────────────────────────────────── */
/*
 * The professional-editor flow: the page is rendered normally, and in edit mode
 * every text run becomes a clickable box drawn exactly over the text it
 * represents. Clicking one selects that line and raises a floating toolbar.
 * Editing happens in place, and "Apply changes" writes the new text back to the
 * PDF in one transaction.
 *
 * The overlay is deliberately its own layer rather than reusing pdf.js's text
 * layer: the text layer's spans are sized for glyph metrics, not for letting a
 * user click a whole line and retype it.
 */

let editMode = false;
/** Place-a-text-box mode: click the page to drop editable text at that spot. */
let addTextMode = false;
let addTextBox = null;
let addTextPos = null;   // PDF points {x, y} of the box's top-left
let addTextFmt = { size: 12, bold: false, italic: false, color: '#000000' };
let addTextBtn = null;
let editLayer = null;
let editRuns = [];          // runs as last fetched (the match anchors)
let selectedRun = null;     // the run being edited
let selectedEl = null;
let editBar = null;

/** PDF point -> CSS px for the current zoom (identity of the viewport scale). */
const ptToPx = () => ZOOMS[zoomIndex] / 100;

async function toggleEditMode() {
  editMode = !editMode;
  editToggleBtn?.classList.toggle('is-active', editMode);
  editToggleBtn?.setAttribute('aria-pressed', String(editMode));
  canvasEl?.classList.toggle('is-editing', editMode);
  if (!editMode) closeRunEditor();
  await renderMain();
}

/** Build the overlay boxes for the runs currently on the page. */
async function paintEditOverlay(pageElNow) {
  if (editLayer) { editLayer.remove(); editLayer = null; }
  editRuns = [];
  selectedRun = null;
  selectedEl = null;
  closeEditBar();
  if (!editMode || !currentPdf) return;

  let runs = [];
  try {
    runs = await fetchTextRuns(currentPage);
  } catch (e) {
    toast(e.message || 'Could not read the page text', { type: 'error' });
    return;
  }
  editRuns = runs;

  const layer = document.createElement('div');
  layer.className = 'pdf-edit-layer';
  const k = ptToPx();
  const pageH = pageSizePt?.h ?? 0;

  for (const run of runs) {
    const [x, y, w, h] = run.rect;
    const box = document.createElement('div');
    box.className = 'pdf-run';
    // PDF origin is bottom-left; the overlay is top-left.
    box.style.left = `${x * k}px`;
    box.style.top = `${(pageH - (y + h)) * k}px`;
    box.style.width = `${w * k}px`;
    box.style.height = `${h * k}px`;
    box.style.fontSize = `${run.size * k}px`;
    box.style.color = run.color || '#000';
    box.dataset.idx = String(editRuns.indexOf(run));
    box.setAttribute('role', 'button');
    box.setAttribute('tabindex', '0');
    box.title = run.text;
    const label = document.createElement('span');
    label.className = 'pdf-run-text';
    label.textContent = run.text;
    box.appendChild(label);
    box.addEventListener('click', (e) => {
      e.stopPropagation();
      selectRun(run, box);
    });
    box.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); selectRun(run, box); }
    });
    layer.appendChild(box);
  }
  pageElNow.appendChild(layer);
  editLayer = layer;
}

function selectRun(run, box) {
  if (selectedEl) selectedEl.classList.remove('is-selected');
  selectedRun = run;
  selectedEl = box;
  box.classList.add('is-selected');
  box.setAttribute('contenteditable', 'true');
  box.focus();
  // Put the caret in the text without selecting every character, so typing
  // replaces nothing until the user asks it to.
  const range = document.createRange();
  range.selectNodeContents(box);
  range.collapse(false);
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
  showEditBar(run, box);
}

function closeRunEditor() {
  if (selectedEl) {
    selectedEl.removeAttribute('contenteditable');
    selectedEl.classList.remove('is-selected');
  }
  selectedRun = null;
  selectedEl = null;
  closeEditBar();
}

function closeEditBar() {
  editBar?.remove();
  editBar = null;
}

/**
 * The floating toolbar for the selected run.
 *
 * It offers what the engine actually implements: retype the line in place, then
 * Apply or Cancel. Formatting controls (bold / size / colour) are deliberately
 * NOT here. The engine can only replace text; restyling a run would mean
 * rewriting the PDF's inherited graphics state (its `Tf`/`rg` apply to every
 * following text object), which is not implemented. Buttons that reported
 * success while changing nothing, or that restyled the rest of the page, would
 * be worse than no buttons at all.
 */
function showEditBar(run, box) {
  closeEditBar();
  const bar = document.createElement('div');
  bar.className = 'pdf-edit-bar';

  const apply = document.createElement('button');
  apply.type = 'button';
  apply.className = 'pdf-edit-apply';
  apply.textContent = 'Apply changes';
  const cancel = document.createElement('button');
  cancel.type = 'button';
  cancel.className = 'pdf-edit-cancel';
  cancel.textContent = 'Cancel';
  bar.append(apply, cancel);

  cancel.addEventListener('click', () => {
    // Undo any typing by re-painting the page from the PDF.
    void renderMain();
  });

  apply.addEventListener('click', () => void applyRunEdit({ run, box }));

  document.body.appendChild(bar);
  const r = box.getBoundingClientRect();
  bar.style.left = `${Math.max(8, Math.min(r.left, window.innerWidth - bar.offsetWidth - 8))}px`;
  bar.style.top = `${Math.max(8, r.top - bar.offsetHeight - 8)}px`;
  editBar = bar;
}

async function applyRunEdit({ run, box }) {
  const newText = box.textContent;
  if (newText === run.text) {
    toast('No changes to apply', { type: 'info' });
    closeRunEditor();
    return;
  }
  const edit = {
    match_text: run.text,
    match_rect: run.rect,
    text: newText,
  };

  setBusy(true);
  try {
    await editTextRuns([edit]);
    dropPdfDoc();              // bytes changed: reload the document
    closeRunEditor();
    await renderThumbs();
    await renderMain();
    showSaved();
    toast('Page text updated', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Could not apply the edit', { type: 'error' });
    setBusy(false);
  }
}

/* ── Adding text ─────────────────────────────────────────────── */
/*
 * Text the user *adds* is a different proposition from retyping an existing run.
 * A PDF content stream carries no per-run styling — `Tf` is inherited state that
 * applies to everything after it — so restyling existing text means rewriting a
 * shared operator and is fragile. Inserted text is new, carries its own font
 * operator, and therefore takes size, weight and colour cleanly. That is why
 * formatting lives here and not in the line editor.
 */

async function toggleAddText() {
  addTextMode = !addTextMode;
  if (addTextMode) {
    // The two modes are exclusive: their click targets overlap.
    editMode = false;
    editToggleBtn?.classList.remove('is-active');
    canvasEl?.classList.remove('is-editing');
    closeRunEditor();
    await renderMain();
  }
  addTextBtn?.classList.toggle('is-active', addTextMode);
  addTextBtn?.setAttribute('aria-pressed', String(addTextMode));
  canvasEl?.classList.toggle('is-placing', addTextMode);
  if (!addTextMode) cancelAddText();
}

function cancelAddText() {
  addTextBox?.remove();
  addTextBox = null;
  addTextPos = null;
  closeEditBar();
}

/** Turn a click on the page into a fresh editable text box at that point. */
function placeTextBox(pageElNow, ev) {
  cancelAddText();
  const r = pageElNow.getBoundingClientRect();
  const k = ptToPx();
  const localX = ev.clientX - r.left;
  const localY = ev.clientY - r.top;
  const box = document.createElement('div');
  box.className = 'pdf-addtext';
  box.contentEditable = 'true';
  box.style.left = `${localX}px`;
  box.style.top = `${localY}px`;
  box.dataset.placeholder = 'Type here…';
  pageElNow.appendChild(box);

  // PDF origin is bottom-left, and the room we have below the click point is
  // what the server needs as a height for the text run.
  const pageH = pageSizePt?.h ?? 0;
  addTextPos = { x: localX / k, y: pageH - localY / k, w: 0, h: 0 };
  addTextBox = box;
  box.focus();
  showAddTextBar(box);
}

/** Formatting toolbar for the pending text box. */
function showAddTextBar(box) {
  closeEditBar();
  const bar = document.createElement('div');
  bar.className = 'pdf-edit-bar';

  const pill = (label, title, on, onClick) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'pdf-edit-pill';
    b.title = title;
    b.textContent = label;
    b.classList.toggle('is-on', !!on);
    b.addEventListener('click', (e) => { e.preventDefault(); onClick(b); });
    bar.appendChild(b);
    return b;
  };
  const sep = () => {
    const el = document.createElement('span');
    el.className = 'pdf-edit-sep';
    bar.appendChild(el);
  };
  const sync = () => {
    box.style.fontSize = `${addTextFmt.size * ptToPx()}px`;
    box.style.fontWeight = addTextFmt.bold ? '700' : '400';
    box.style.fontStyle = addTextFmt.italic ? 'italic' : 'normal';
    box.style.color = addTextFmt.color;
    sizeLabel.textContent = `${addTextFmt.size} pt`;
  };

  const boldBtn = pill('B', 'Bold', addTextFmt.bold, (b) => {
    addTextFmt.bold = !addTextFmt.bold; b.classList.toggle('is-on', addTextFmt.bold); sync();
  });
  const italBtn = pill('I', 'Italic', addTextFmt.italic, (b) => {
    addTextFmt.italic = !addTextFmt.italic; b.classList.toggle('is-on', addTextFmt.italic); sync();
  });
  sep();

  const sizeLabel = document.createElement('span');
  sizeLabel.className = 'pdf-edit-label';
  const minus = pill('A−', 'Smaller', false, () => {
    addTextFmt.size = Math.max(4, Math.round((addTextFmt.size - 1) * 10) / 10); sync();
  });
  const plus = pill('A+', 'Larger', false, () => {
    addTextFmt.size = Math.min(144, Math.round((addTextFmt.size + 1) * 10) / 10); sync();
  });
  bar.append(sizeLabel, minus, plus);
  sep();

  const colorInput = document.createElement('input');
  colorInput.type = 'color';
  colorInput.className = 'pdf-edit-color';
  colorInput.title = 'Text colour';
  colorInput.value = addTextFmt.color;
  colorInput.addEventListener('input', () => { addTextFmt.color = colorInput.value; sync(); });
  bar.appendChild(colorInput);
  sep();

  const apply = document.createElement('button');
  apply.type = 'button';
  apply.className = 'pdf-edit-apply';
  apply.textContent = 'Add text';
  const cancel = document.createElement('button');
  cancel.type = 'button';
  cancel.className = 'pdf-edit-cancel';
  cancel.textContent = 'Cancel';
  bar.append(apply, cancel);
  cancel.addEventListener('click', () => void renderMain());
  apply.addEventListener('click', () => void commitAddText(box));

  document.body.appendChild(bar);
  const r = box.getBoundingClientRect();
  bar.style.left = `${Math.max(8, Math.min(r.left, window.innerWidth - bar.offsetWidth - 8))}px`;
  bar.style.top = `${Math.max(8, r.top - bar.offsetHeight - 8)}px`;
  editBar = bar;
  boldBtn.classList.toggle('is-on', addTextFmt.bold);
  italBtn.classList.toggle('is-on', addTextFmt.italic);
  sync();
}

/** Send the pending text box to the server as a formatted `free_text` run. */
async function commitAddText(box) {
  const text = (box.textContent || '').trim();
  if (!text) {
    toast('Type some text first', { type: 'info' });
    return;
  }
  if (!addTextPos || !currentPdf) return;
  // Size the text run from its own metrics so the server has a sensible box:
  // the box is placed at the click point and grows down-right from there.
  const rect = [addTextPos.x, addTextPos.y - addTextFmt.size, addTextPos.w, addTextFmt.size * 1.4];
  setBusy(true);
  try {
    await annotatePdf('free_text', rect, text, addTextFmt);
    dropPdfDoc();
    closeEditBar();
    addTextBox = null;
    addTextPos = null;
    await renderThumbs();
    await renderMain();
    showSaved();
    toast('Text added', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Could not add the text', { type: 'error' });
    setBusy(false);
  }
}

/* ── Page rendering (pdf.js) ────────────────────────────────── */

/** The pdf.js worker is single-threaded per document; one paint at a time. */
let renderChain = Promise.resolve();

/** Render serialization: paints never overlap and never outlive a page change. */
function queueRender(fn) {
  const run = renderChain.then(fn, fn);
  renderChain = run.catch(() => {});
  return run;
}

/** Drop the loaded document (after an edit, or when switching documents). */
function dropPdfDoc() {
  renderSeq++;
  try { renderTask?.cancel(); } catch (_) { /* already finished */ }
  renderTask = null;
  if (pdfDoc) {
    const doc = pdfDoc;
    pdfDoc = null;
    void doc.destroy().catch(() => {});
  }
}

/** The loaded pdf.js document for `currentPdf`, opening it on first use. */
async function getPdfDoc() {
  if (!currentPdf) throw new Error('No PDF open');
  if (pdfDoc) return pdfDoc;
  const pdfjs = await loadPdfJs();
  const doc = await pdfjs.getDocument({
    // Served by the plugin's own route: the stored bytes, inline.
    url: `/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/file`,
    cMapUrl: '/vendor/pdfjs/cmaps/',
    cMapPacked: true,
    standardFontDataUrl: '/vendor/pdfjs/standard_fonts/',
  }).promise;
  pdfDoc = doc;
  return doc;
}

async function getPageView(index, zoom) {
  const doc = await getPdfDoc();
  const page = await doc.getPage(index + 1);
  return { page, viewport: page.getViewport({ scale: zoom, rotation: page.rotate }) };
}

/**
 * Paint page `index` into an `<img>` for the thumbnail rail.
 *
 * `pass` scales the work: a fast first pass fills the rail, a second sharper
 * pass replaces it. Both come from the loaded document, so there is no server
 * render (and no per-page network cost) at all.
 */
async function renderThumbInto(img, index, boxWidth, pass) {
  if (!currentPdf || !img.isConnected) return;
  const { page } = await getPageView(index, 1);
  const base = page.getViewport({ scale: 1, rotation: page.rotate });
  const scale = (boxWidth / base.width) * pass;
  const viewport = page.getViewport({ scale, rotation: page.rotate });
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.floor(viewport.width));
  canvas.height = Math.max(1, Math.floor(viewport.height));
  await page.render({ canvasContext: canvas.getContext('2d'), viewport }).promise;
  if (!img.isConnected) return;
  img.src = canvas.toDataURL('image/jpeg', 0.8);
}

/* ── Status ─────────────────────────────────────────────────── */

function setStatus(text) {
  if (statusEl) statusEl.textContent = text;
}

function setBusy(on) {
  saveDot?.classList.toggle('is-active', on);
}

function showSaved() {
  const now = new Date();
  const t = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const n = currentPdf?.page_count ?? 0;
  setStatus(`Saved ${t} · ${n} ${n === 1 ? 'page' : 'pages'}`);
  setBusy(false);
}

/* ── Rendering the window ───────────────────────────────────── */

function clampPage(p) {
  const max = currentPdf?.page_count ?? 0;
  if (max === 0) return 0;
  return Math.max(0, Math.min(p, max - 1));
}

/**
 * Paint the current page into `canvasEl` with pdf.js, plus its text layer.
 *
 * The page is drawn at `zoom * devicePixelRatio` device pixels but laid out at
 * `zoom` CSS pixels, so it stays sharp on hi-dpi screens and at high zoom
 * without asking the server for anything.
 */
async function paintCurrentPage() {
  const seq = ++renderSeq;
  if (renderTask) {
    try { renderTask.cancel(); } catch (_) { /* already finished */ }
    renderTask = null;
  }
  if (!currentPdf || !canvasEl) return;

  canvasEl.innerHTML = '';
  if (currentPdf.page_count === 0) {
    setTileGlow(tileEl, null);
    canvasEl.appendChild(emptyState({ icon: 'ui/doc', title: 'Empty PDF', body: 'This document has no pages.' }));
    return;
  }

  setBusy(true);
  const loading = document.createElement('div');
  loading.className = 'pdf-loading';
  loading.appendChild(spinner({ size: 20 }));
  canvasEl.appendChild(loading);

  let page = null;
  let viewport = null;
  const zoom = ZOOMS[zoomIndex] / 100;
  try {
    ({ page, viewport } = await getPageView(currentPage, zoom));
  } catch (e) {
    if (seq !== renderSeq) return;
    canvasEl.innerHTML = '';
    canvasEl.appendChild(emptyState({
      icon: 'ui/warning',
      title: 'Could not render page',
      body: e.message || '',
    }));
    setBusy(false);
    return;
  }

  pageSizePt = { w: viewport.width / (ZOOMS[zoomIndex] / 100), h: viewport.height / (ZOOMS[zoomIndex] / 100) };

  const el = document.createElement('div');
  el.className = 'pdf-page';
  el.style.width = `${viewport.width}px`;
  el.style.height = `${viewport.height}px`;
  el.setAttribute('role', 'img');
  el.setAttribute('aria-label', `Page ${currentPage + 1}`);

  const canvas = document.createElement('canvas');
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.floor(viewport.width * dpr));
  canvas.height = Math.max(1, Math.floor(viewport.height * dpr));
  canvas.style.width = `${viewport.width}px`;
  canvas.style.height = `${viewport.height}px`;
  canvas.className = 'pdf-page-canvas';
  el.appendChild(canvas);

  const textLayerElNew = document.createElement('div');
  textLayerElNew.className = 'pdf-text-layer';
  el.appendChild(textLayerElNew);

  // The text layer positions spans with `--scale-factor`; `--total-scale-factor`
  // covers builds that read the composed factor.
  el.style.setProperty('--scale-factor', String(zoom));
  el.style.setProperty('--total-scale-factor', String(zoom));

  canvasEl.innerHTML = '';
  canvasEl.appendChild(el);
  pageEl = el;
  textLayerEl = textLayerElNew;
  attachSelection(el);

  try {
    const task = page.render({
      canvasContext: canvas.getContext('2d'),
      viewport,
      transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : undefined,
    });
    renderTask = task;
    await task.promise;
    renderTask = null;
  } catch (e) {
    if (e?.name === 'RenderingCancelledException' || seq !== renderSeq) return;
    setBusy(false);
    return;
  }
  if (seq !== renderSeq) return;

  // Text layer: real selectable text over the painted page.
  try {
    const pdfjs = await loadPdfJs();
    const textContent = await page.getTextContent();
    if (seq !== renderSeq) return;
    const layer = new pdfjs.TextLayer({
      textContentSource: textContent,
      container: textLayerElNew,
      viewport,
    });
    await layer.render();
  } catch (_) {
    // The page is still perfectly readable without a text layer; selection is
    // the only thing lost.
  }

  if (seq !== renderSeq) return;
  canvasEl.scrollTop = 0;
  applyPageGlow(canvas);
  // Edit mode draws the clickable text-run overlay on top of the page.
  await paintEditOverlay(el);
  setBusy(false);
  updateNav();
}

/**
 * Mirror the painted page into the window's ambient glow.
 *
 * The blur must be *baked*, not applied live: `glowFromDrawable()` downsamples
 * and pre-blurs once (PLUGINS.md §19 "Window background"), so the window keeps a
 * soft recognisable background for free instead of paying a `blur()` per resize
 * frame. Only the top slice of the page is sampled — that is the part visible
 * behind the window chrome, and the whole sheet is far more than a 64px glow
 * needs.
 */
function applyPageGlow(canvas) {
  if (!tileEl || !canvas.width || !canvas.height) return;
  let css = null;
  try {
    const sample = document.createElement('canvas');
    sample.width = canvas.width;
    sample.height = Math.max(1, Math.round(canvas.height * 0.4));
    sample.getContext('2d').drawImage(
      canvas, 0, 0, canvas.width, sample.height, 0, 0, sample.width, sample.height,
    );
    css = glowFromDrawable(sample, 96);
  } catch (_) {
    css = null;   // tainted canvas — fall through to the colour glow
  }
  // Passing null restores the Tier 0 colour glow rather than leaving the window
  // with no ambient light at all.
  setTileGlow(tileEl, css);
}

function renderMain() {
  return queueRender(paintCurrentPage);
}

function updateNav() {
  if (pageLabelEl) {
    const n = currentPdf?.page_count ?? 0;
    pageLabelEl.textContent = n === 0 ? '0 / 0' : `${currentPage + 1} / ${n}`;
  }
  if (zoomLabelEl) zoomLabelEl.textContent = `${ZOOMS[zoomIndex]}%`;
}

async function renderThumbs() {
  if (!currentPdf || !railEl) return;
  const n = currentPdf.page_count;
  railEl.innerHTML = '';
  for (let i = 0; i < n; i++) {
    const thumb = document.createElement('div');
    thumb.className = 'pdf-thumb';
    thumb.dataset.page = String(i);
    thumb.draggable = true;
    if (i === currentPage) thumb.classList.add('is-active');

    const num = document.createElement('span');
    num.className = 'pdf-thumb-num';
    num.textContent = String(i + 1);

    const del = document.createElement('button');
    del.type = 'button';
    del.className = 'pdf-thumb-del';
    del.title = 'Delete page';
    del.setAttribute('aria-label', `Delete page ${i + 1}`);
    del.appendChild(icon('ui/close', { size: 11 }));
    del.addEventListener('click', (e) => {
      e.stopPropagation();
      void removePage(i);
    });

    const img = document.createElement('img');
    img.className = 'pdf-thumb-img';
    img.loading = 'lazy';
    img.alt = `Page ${i + 1}`;
    img.draggable = false;
    // Two passes: a low-res pass so the rail fills immediately, then a sharper
    // one — all served from the already-loaded document, no server round-trip.
    const width = railEl.clientWidth || 120;
    queueRender(() => renderThumbInto(img, i, width, 0.45))
      .catch(() => {})
      .finally(() => { queueRender(() => renderThumbInto(img, i, width, 1.2)).catch(() => {}); });

    thumb.append(num, img, del);
    thumb.addEventListener('click', () => {
      currentPage = i;
      void renderMain();
      markActiveThumb();
    });
    thumb.addEventListener('dragstart', (e) => {
      dragIndex = i;
      e.dataTransfer.effectAllowed = 'move';
      try { e.dataTransfer.setData('text/plain', String(i)); } catch (_) { /* no-op */ }
      thumb.classList.add('is-dragging');
    });
    thumb.addEventListener('dragend', () => {
      dragIndex = null;
      thumb.classList.remove('is-dragging');
    });
    thumb.addEventListener('dragover', (e) => {
      e.preventDefault();
      e.dataTransfer.dropEffect = 'move';
    });
    thumb.addEventListener('drop', (e) => {
      e.preventDefault();
      const from = dragIndex;
      dragIndex = null;
      if (from == null || from === i) return;
      const order = [...Array(n).keys()];
      order.splice(from, 1);
      const pos = order.indexOf(i);
      order.splice(pos, 0, from);
      void applyReorder(order, pos);
    });
    railEl.appendChild(thumb);
  }
}

function markActiveThumb() {
  railEl?.querySelectorAll('.pdf-thumb').forEach((t) => {
    t.classList.toggle('is-active', Number(t.dataset.page) === currentPage);
  });
  const active = railEl?.querySelector('.pdf-thumb.is-active');
  active?.scrollIntoView({ block: 'nearest' });
}

/* ── Actions ────────────────────────────────────────────────── */

async function openDoc(doc) {
  if (!doc) return;
  try {
    const full = doc.pdf_id != null && doc.page_count != null
      ? doc
      : await fetchPdf(doc.pdf_id || doc.id);
    currentPdf = {
      pdf_id: full.pdf_id || doc.pdf_id || doc.id,
      title: full.title,
      page_count: full.page_count,
      updated_at: full.updated_at,
    };
    currentPage = 0;
    titleInput.value = currentPdf.title;
    editMode = false;
    editToggleBtn?.classList.remove('is-active');
    canvasEl?.classList.remove('is-editing');
    closeRunEditor();
    dropPdfDoc();
    await renderThumbs();
    await renderMain();
    showSaved();
  } catch (e) {
    toast(e.message || 'Could not open PDF', { type: 'error' });
  }
}

async function refreshDocs() {
  try {
    docs = await listPdfs();
  } catch (_) { /* keep last list */ }
}

async function openNewest() {
  const list = await listPdfs();
  docs = list;
  if (list.length) await openDoc(list[0]);
  else {
    currentPdf = null;
    currentPage = 0;
    setTileGlow(tileEl, null);
    if (titleInput) titleInput.value = '';
    if (railEl) railEl.innerHTML = '';
    if (canvasEl) {
      canvasEl.innerHTML = '';
      canvasEl.appendChild(emptyState({
        icon: 'ui/doc',
        title: 'No PDFs yet',
        body: 'Import a .pdf or create a new one to get started.',
      }));
    }
    updateNav();
    showSaved();
  }
}

async function newPdf() {
  try {
    const created = await createPdf();
    await refreshDocs();
    await openDoc(created);
    toast('Created new PDF', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Could not create PDF', { type: 'error' });
  }
}

async function pickPdfFile() {
  const [file] = await pickFiles({ accept: '.pdf,application/pdf' });
  if (file) void importPdf(file);
}

async function importPdf(file) {
  setBusy(true);
  try {
    const imported = await importPdfFile(file);
    await refreshDocs();
    await openDoc(imported);
    toast(`Imported ${file.name}`, { type: 'info' });
  } catch (e) {
    toast(e.message || 'Import failed — is this a valid .pdf file?', { type: 'error' });
    setBusy(false);
  }
}

async function exportCurrent() {
  if (!currentPdf) return;
  try {
    const blob = await apiFetch(
      `/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/export`,
      { responseType: 'blob' },
    );
    await saveOrDownload(blob, { name: `${currentPdf.title || 'document'}.pdf`, dir: 'Documents', app: 'PDF' });
  } catch (e) {
    toast(e.message || 'Save failed', { type: 'error' });
  }
}

async function removeCurrent() {
  if (!currentPdf) return;
  if (!window.confirm(`Delete "${currentPdf.title}"?`)) return;
  try {
    await deletePdf(currentPdf.pdf_id);
    dropPdfDoc();
    currentPdf = null;
    await refreshDocs();
    await openNewest();
  } catch (e) {
    toast(e.message || 'Could not delete PDF', { type: 'error' });
  }
}

async function rotateCurrent(delta) {
  if (!currentPdf) return;
  setBusy(true);
  try {
    await rotatePage([currentPage], delta);
    dropPdfDoc();
    currentPdf = { ...currentPdf };
    await renderThumbs();
    await renderMain();
    showSaved();
  } catch (e) {
    toast(e.message || 'Rotate failed', { type: 'error' });
    setBusy(false);
  }
}

async function applyReorder(order, focusPage) {
  setBusy(true);
  try {
    await reorderPages(order);
    dropPdfDoc();
    currentPdf.page_count = order.length;
    currentPage = clampPage(focusPage);
    await renderThumbs();
    await renderMain();
    showSaved();
  } catch (e) {
    toast(e.message || 'Reorder failed', { type: 'error' });
    setBusy(false);
    await renderThumbs();
  }
}

async function removePage(page) {
  if (!currentPdf) return;
  if (currentPdf.page_count <= 1) {
    toast('Cannot delete the only page', { type: 'error' });
    return;
  }
  if (!window.confirm(`Delete page ${page + 1}?`)) return;
  setBusy(true);
  try {
    await deletePages([page]);
    dropPdfDoc();
    currentPdf.page_count -= 1;
    currentPage = clampPage(page);
    await renderThumbs();
    await renderMain();
    showSaved();
  } catch (e) {
    toast(e.message || 'Could not delete page', { type: 'error' });
    setBusy(false);
  }
}

async function doMerge(otherId) {
  setBusy(true);
  try {
    const res = await mergeOther(otherId);
    dropPdfDoc();
    currentPdf.page_count = res.page_count ?? currentPdf.page_count;
    currentPage = clampPage(currentPage);
    await renderThumbs();
    await renderMain();
    showSaved();
    toast('Merged PDF', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Merge failed', { type: 'error' });
    setBusy(false);
  }
}

/* ── Region selection + annotations ─────────────────────────── */

function dispPos(e) {
  const r = pageEl.getBoundingClientRect();
  return { x: e.clientX - r.left, y: e.clientY - r.top };
}

/** Convert a displayed-pixel selection into PDF points (bottom-left origin). */
function dispToPdfRect(a, b) {
  const pxToPt = 100 / ZOOMS[zoomIndex];
  const x0 = Math.min(a.x, b.x) * pxToPt;
  const y0 = Math.min(a.y, b.y) * pxToPt;
  const w = Math.abs(a.x - b.x) * pxToPt;
  const h = Math.abs(a.y - b.y) * pxToPt;
  return { x: x0, y: pageSizePt.h - (y0 + h), w, h };
}

function layoutSel(a, b) {
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  selOverlay.style.left = `${x}px`;
  selOverlay.style.top = `${y}px`;
  selOverlay.style.width = `${Math.abs(a.x - b.x)}px`;
  selOverlay.style.height = `${Math.abs(a.y - b.y)}px`;
}

function onSelMove(e) {
  if (!selStart) return;
  layoutSel(selStart, dispPos(e));
}

function onSelUp(e) {
  if (!selStart) return;
  const start = selStart;
  const end = dispPos(e);
  selStart = null;
  selOverlay?.classList.add('hidden');
  const dx = Math.abs(end.x - start.x);
  const dy = Math.abs(end.y - start.y);
  if (dx < 6 || dy < 6) return;
  const r = pageEl.getBoundingClientRect();
  selDisplayRect = {
    left: r.left + Math.min(start.x, end.x),
    top: r.top + Math.min(start.y, end.y),
  };
  showAnnotBar(dispToPdfRect(start, end));
  // The bar has taken what it needs from the selection; drop the highlight so
  // the pending annotation region is the only thing visually marked.
  window.getSelection()?.removeAllRanges();
}

/**
 * Region selection for annotations.
 *
 * With a real text layer in play, a plain drag belongs to text selection, so
 * the annotation gesture is Shift+drag — that keeps "select to copy" native
 * while leaving highlights and friends reachable.
 */
function attachSelection(el) {
  selStart = null;
  selOverlay = document.createElement('div');
  selOverlay.className = 'pdf-select-box hidden';
  selOverlay.setAttribute('aria-hidden', 'true');
  el.appendChild(selOverlay);

  el.addEventListener('mousedown', (e) => {
    // Add-text mode claims a plain click; the annotation gesture keeps Shift.
    if (addTextMode && !e.shiftKey && e.button === 0) {
      e.preventDefault();
      placeTextBox(el, e);
      return;
    }
    if (!currentPdf || !e.shiftKey || e.button !== 0) return;
    hideAnnotBar();
    e.preventDefault();
    // Do NOT clear the text selection here: `showAnnotBar` reads it to prefill
    // the note / text-box / find-and-replace prompts, then clears it itself.
    selStart = dispPos(e);
    selOverlay.classList.remove('hidden');
    layoutSel(selStart, selStart);

    const move = (ev) => onSelMove(ev);
    const up = (ev) => {
      window.removeEventListener('mousemove', move, true);
      window.removeEventListener('mouseup', up, true);
      onSelUp(ev);
    };
    window.addEventListener('mousemove', move, true);
    window.addEventListener('mouseup', up, true);
  });
}

const ANNOT_LABELS = {
  highlight: 'Highlight',
  underline: 'Underline',
  strikeout: 'Strikeout',
  squiggly: 'Squiggly',
  note: 'Note',
  free_text: 'Text box',
  link: 'Link',
};

function ensureAnnotBar() {
  if (annotBar) return;
  annotBar = document.createElement('div');
  annotBar.className = 'pdf-annot-bar hidden';
  annotBar.addEventListener('pointerdown', (e) => e.stopPropagation());
  document.body.appendChild(annotBar);
}

function hideAnnotBar() {
  annotBar?.classList.add('hidden');
  annotRect = null;
}

/** The text currently selected in the PDF's text layer, collapsed to one line. */
function selectedPdfText() {
  const text = window.getSelection?.()?.toString() || '';
  return text.replace(/\s+/g, ' ').trim();
}

function showAnnotBar(rect) {
  ensureAnnotBar();
  annotRect = rect;
  // Capture any text selection now: clicking a button clears it, and both the
  // note and text-box prompts are much more useful pre-filled with the words
  // the user actually pointed at.
  const picked = selectedPdfText();
  annotBar.innerHTML = '';
  const mk = (label, fn) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'pdf-annot-btn';
    b.textContent = label;
    b.addEventListener('click', fn);
    annotBar.appendChild(b);
  };
  const mkText = (label, message, kind, prefill) => {
    mk(label, () => {
      const t = window.prompt(message, prefill || '');
      if (t != null && t.trim()) void doAnnotate(kind, t.trim());
    });
  };
  mk('Highlight', () => void doAnnotate('highlight', ''));
  mk('Underline', () => void doAnnotate('underline', ''));
  mk('Strikeout', () => void doAnnotate('strikeout', ''));
  mk('Squiggly', () => void doAnnotate('squiggly', ''));
  mkText('Note', 'Note text:', 'note', picked);
  mkText('Text box', 'Text to place on the page:', 'free_text', picked);
  mk('Link', () => {
    const t = window.prompt('Link URL:');
    if (t != null && t.trim()) void doAnnotate('link', t.trim());
  });
  if (picked) {
    mk('Find & replace…', () => {
      const old = window.prompt('Find text:', picked);
      if (old == null || !old.trim()) return;
      const nw = window.prompt(`Replace "${old.trim()}" with:`, '');
      if (nw == null) return;
      void doReplaceText(old.trim(), nw);
    });
  }
  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'pdf-annot-btn pdf-annot-btn--close';
  close.textContent = '×';
  close.setAttribute('aria-label', 'Cancel');
  close.addEventListener('click', hideAnnotBar);
  annotBar.appendChild(close);
  annotBar.classList.remove('hidden');
  const r = selDisplayRect || { left: 12, top: 12 };
  annotBar.style.left = `${Math.max(8, Math.min(r.left, window.innerWidth - 560))}px`;
  annotBar.style.top = `${Math.max(8, r.top - 48)}px`;
}

async function doAnnotate(kind, text) {
  // Read the rect before hiding the bar: the bar's document-level `pointerdown`
  // handler fires ahead of the button's `click`, so hiding first would leave
  // `annotRect` null here and silently drop every annotation.
  if (!currentPdf || !annotRect) return;
  const rect = [annotRect.x, annotRect.y, annotRect.w, annotRect.h];
  hideAnnotBar();
  setBusy(true);
  try {
    await annotatePdf(kind, rect, text);
    dropPdfDoc();
    await renderThumbs();
    await renderMain();
    showSaved();
    toast(`${ANNOT_LABELS[kind] || kind} added`, { type: 'info' });
  } catch (e) {
    toast(e.message || 'Annotation failed', { type: 'error' });
    setBusy(false);
  }
}

function promptFindReplace() {
  if (!currentPdf) return;
  const old = window.prompt('Find text:', '');
  if (old == null || !old.trim()) return;
  const nw = window.prompt(`Replace "${old.trim()}" with:`, '');
  if (nw == null) return;
  void doReplaceText(old.trim(), nw);
}

async function doReplaceText(oldText, newText) {
  setBusy(true);
  try {
    const res = await replaceTextApi(currentPage, oldText, newText);
    dropPdfDoc();
    await renderThumbs();
    await renderMain();
    showSaved();
    const n = res?.replaced ?? 0;
    toast(`Replaced ${n} ${n === 1 ? 'spot' : 'spots'}`, { type: 'info' });
  } catch (e) {
    toast(e.message || 'Replace failed', { type: 'error' });
    setBusy(false);
  }
}

function promptWatermark() {
  if (!currentPdf) return;
  const t = window.prompt('Watermark text:', 'DRAFT');
  if (t == null || !t.trim()) return;
  void doWatermark(t.trim());
}

async function doWatermark(text) {
  setBusy(true);
  try {
    await watermarkApi(text);
    dropPdfDoc();
    await renderThumbs();
    await renderMain();
    showSaved();
    toast('Watermark applied', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Watermark failed', { type: 'error' });
    setBusy(false);
  }
}

function zoomBy(delta) {
  zoomIndex = Math.max(0, Math.min(ZOOMS.length - 1, zoomIndex + delta));
  updateNav();
  void renderMain();
}

function gotoPage(delta) {
  if (!currentPdf) return;
  currentPage = clampPage(currentPage + delta);
  void renderMain();
  markActiveThumb();
}

/* ── Doc menu ───────────────────────────────────────────────── */

function ensureDocMenu() {
  if (docMenuPopup) return;
  docMenuPopup = document.createElement('div');
  docMenuPopup.className = 'pdf-doc-menu hidden';
  docMenuPopup.setAttribute('role', 'menu');
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
  if (docMenuPopup && !docMenuPopup.contains(e.target) && docMenuBtn && !docMenuBtn.contains(e.target)) {
    closeDocMenu();
  }
}

function onDocMenuKey(e) {
  if (e.key === 'Escape') closeDocMenu();
}

function docMenuItem(title, iconName, opts = {}) {
  const item = document.createElement('button');
  item.type = 'button';
  item.className = 'pdf-doc-menu-item';
  if (opts.danger) item.classList.add('pdf-doc-menu-item--danger');
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
  item.addEventListener('click', () => {
    closeDocMenu();
    opts.onClick?.();
  });
  return item;
}

function renderDocMenuItems() {
  if (!docMenuPopup) return;
  docMenuPopup.innerHTML = '';

  if (menuMode === 'merge') {
    const others = docs.filter((d) => d.pdf_id !== currentPdf?.pdf_id);
    const head = document.createElement('div');
    head.className = 'pdf-doc-menu-empty';
    head.textContent = others.length ? 'Merge into this PDF:' : 'No other PDFs to merge';
    docMenuPopup.appendChild(head);
    others.forEach((d) => {
      docMenuPopup.appendChild(docMenuItem(d.title, 'ui/loop', { time: formatWhen(d.updated_at), onClick: () => void doMerge(d.pdf_id) }));
    });
    const foot = document.createElement('div');
    foot.className = 'pdf-doc-menu-foot';
    foot.appendChild(docMenuItem('Back', 'ui/arrow-left', { onClick: () => { menuMode = 'default'; openDocMenu(); } }));
    docMenuPopup.appendChild(foot);
    return;
  }

  if (!docs.length) {
    const empty = document.createElement('div');
    empty.className = 'pdf-doc-menu-empty';
    empty.textContent = 'No PDFs yet';
    docMenuPopup.appendChild(empty);
  } else {
    docs.forEach((d) => {
      docMenuPopup.appendChild(docMenuItem(d.title, 'ui/doc', {
        time: formatWhen(d.updated_at),
        active: d.pdf_id === currentPdf?.pdf_id,
        onClick: () => { if (d.pdf_id !== currentPdf?.pdf_id) void openDoc(d); },
      }));
    });
  }

  const foot = document.createElement('div');
  foot.className = 'pdf-doc-menu-foot';
  foot.appendChild(docMenuItem('New PDF', 'ui/plus', { onClick: () => void newPdf() }));
  foot.appendChild(docMenuItem('Import .pdf', 'ui/download', { onClick: pickPdfFile }));
  foot.appendChild(docMenuItem('Save to Documents', 'ui/save', { onClick: () => void exportCurrent() }));
  foot.appendChild(docMenuItem('Merge from…', 'ui/loop', { onClick: () => { menuMode = 'merge'; openDocMenu(); } }));
  foot.appendChild(docMenuItem('Find & replace…', 'ui/search', { onClick: promptFindReplace }));
  foot.appendChild(docMenuItem('Watermark…', 'ui/info', { onClick: promptWatermark }));
  foot.appendChild(docMenuItem('Delete PDF', 'ui/trash', { danger: true, onClick: () => void removeCurrent() }));
  docMenuPopup.appendChild(foot);
}

function formatWhen(iso) {
  if (!iso) return '';
  try {
    return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric' });
  } catch (_) {
    return '';
  }
}

function openDocMenu() {
  ensureDocMenu();
  renderDocMenuItems();
  docMenuPopup.classList.remove('hidden');
  const r = docMenuBtn.getBoundingClientRect();
  const left = Math.max(12, Math.min(r.left, window.innerWidth - 300 - 12));
  docMenuPopup.style.left = `${left}px`;
  docMenuPopup.style.top = `${r.bottom + 8}px`;
  docMenuOpen = true;
  docMenuBtn.setAttribute('aria-expanded', 'true');
  document.addEventListener('pointerdown', onDocMenuOutside, true);
  document.addEventListener('keydown', onDocMenuKey, true);
}

function toggleDocMenu() {
  if (docMenuOpen) closeDocMenu();
  else { menuMode = 'default'; openDocMenu(); }
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

function toolbarButton(iconName, label, onClick) {
  const btn = button({ icon: iconName, variant: 'ghost', onClick });
  btn.classList.add('ui-btn--icon', 'pdf-tool');
  btn.title = label;
  btn.setAttribute('aria-label', label);
  return btn;
}

export function mountPdfTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile pdf-tile';
  tileEl.dataset.plugin = PDF_PLUGIN;

  /* Top bar */
  const bar = document.createElement('div');
  bar.className = 'pdf-bar';

  docMenuBtn = document.createElement('button');
  docMenuBtn.type = 'button';
  docMenuBtn.className = 'pdf-doc-btn';
  docMenuBtn.setAttribute('aria-haspopup', 'menu');
  docMenuBtn.setAttribute('aria-expanded', 'false');
  docMenuBtn.title = 'PDFs';
  const docIcon = document.createElement('span');
  docIcon.className = 'pdf-doc-btn-icon';
  docMenuBtn.appendChild(docIcon);
  void setIcon(docIcon, 'ui/doc', { size: 15 });
  const chevron = document.createElement('span');
  chevron.className = 'pdf-doc-btn-chevron';
  docMenuBtn.appendChild(chevron);
  void setIcon(chevron, 'ui/chevron-down', { size: 12 });
  docMenuBtn.addEventListener('click', toggleDocMenu);

  titleInput = document.createElement('input');
  titleInput.className = 'pdf-title';
  titleInput.type = 'text';
  titleInput.placeholder = 'Untitled';
  titleInput.maxLength = 120;
  titleInput.autocomplete = 'off';
  titleInput.addEventListener('change', () => {
    if (currentPdf) void renamePdf(currentPdf.pdf_id, titleInput.value.trim());
  });

  /* Page nav */
  const prevBtn = toolbarButton('ui/chevron-left', 'Previous page', () => gotoPage(-1));
  pageLabelEl = document.createElement('span');
  pageLabelEl.className = 'pdf-page-label';
  const nextBtn = toolbarButton('ui/chevron-right', 'Next page', () => gotoPage(1));

  /* Zoom */
  const zoomOut = toolbarButton('ui/minus', 'Zoom out', () => zoomBy(-1));
  zoomLabelEl = document.createElement('span');
  zoomLabelEl.className = 'pdf-zoom-label';
  const zoomIn = toolbarButton('ui/plus', 'Zoom in', () => zoomBy(1));

  /* Rotate */
  const rotLeft = toolbarButton('ui/rotate-left', 'Rotate page counter-clockwise', () => void rotateCurrent(-90));
  const rotRight = toolbarButton('ui/rotate-right', 'Rotate page clockwise', () => void rotateCurrent(90));

  /* Edit text in place */
  // The two text tools sit together, so their icons have to be obviously
  // different: `+` was already taken by New PDF and made "add text" invisible.
  editToggleBtn = toolbarButton('ui/bold', 'Change text — click a line to retype it',
    () => void toggleEditMode());
  editToggleBtn.classList.add('pdf-tool--edit');
  editToggleBtn.setAttribute('aria-pressed', 'false');

  /* Add text — click the page to place a formatted text box. Unlike the line
     editor above, this can apply size, weight and colour. */
  addTextBtn = toolbarButton('ui/list', 'Add text — pick size, bold and colour',
    () => void toggleAddText());
  addTextBtn.classList.add('pdf-tool--addtext');
  addTextBtn.setAttribute('aria-pressed', 'false');

  /* File actions */
  const newBtn = toolbarButton('ui/plus', 'New PDF', () => void newPdf());
  const importBtn = toolbarButton('ui/download', 'Import .pdf', pickPdfFile);
  const exportBtn = toolbarButton('ui/save', 'Save to Documents', () => void exportCurrent());
  const delBtn = toolbarButton('ui/trash', 'Delete PDF', () => void removeCurrent());
  delBtn.classList.add('pdf-tool--danger');

  saveDot = document.createElement('span');
  saveDot.className = 'pdf-save-dot';
  saveDot.setAttribute('aria-hidden', 'true');

  bar.append(
    docMenuBtn, titleInput,
    prevBtn, pageLabelEl, nextBtn,
    zoomOut, zoomLabelEl, zoomIn,
    rotLeft, rotRight, editToggleBtn, addTextBtn,
    newBtn, importBtn, exportBtn, delBtn, saveDot,
  );
  tileEl.appendChild(bar);

  /* Body: rail + canvas */
  const main = document.createElement('div');
  main.className = 'pdf-main';

  railEl = document.createElement('div');
  railEl.className = 'pdf-rail';
  railEl.setAttribute('aria-label', 'Pages');

  canvasEl = document.createElement('div');
  canvasEl.className = 'pdf-canvas';

  main.append(railEl, canvasEl);
  tileEl.appendChild(main);

  /* Status line */
  const status = document.createElement('div');
  status.className = 'pdf-status';
  statusEl = document.createElement('span');
  status.appendChild(statusEl);
  tileEl.appendChild(status);

  void openNewest();
  return tileEl;
}

export function unmountPdfTile() {
  closeDocMenu();
  tileEl?.remove();
  // Drop ALL references (incl. body-level popups) so a later
  // mountPdfTile() builds a fresh tile instead of returning the detached
  // old one, and no orphaned popup survives the window it belonged to.
  tileEl = null;
  titleInput = null;
  docMenuBtn = null;
  statusEl = null;
  saveDot = null;
  railEl = null;
  canvasEl = null;
  pageLabelEl = null;
  zoomLabelEl = null;
  menuMode = 'default';

  // Release the pdf.js document and worker, and forget the page geometry, so a
  // later mount starts clean rather than reusing a destroyed document.
  dropPdfDoc();
  pageEl = null;
  textLayerEl = null;
  pageSizePt = null;
  selOverlay = null;
  selStart = null;
  hideAnnotBar();
  editMode = false;
  editLayer = null;
  editRuns = [];
  editToggleBtn = null;
  addTextBtn = null;
  addTextMode = false;
  cancelAddText();
  closeRunEditor();
}

export function getPdfTileElement() {
  return tileEl;
}

/* ── AI wiring ──────────────────────────────────────────────── */

function onAgentActions(e) {
  const actions = e.detail || [];
  const pdfActions = actions.filter((a) => /^pdf_/.test(a?.action || ''));
  if (!pdfActions.length) return;

  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: PDF_PLUGIN } }));

  const created = pdfActions.some((a) => a.action === 'pdf_create' && a.result === 'ok');
  const modified = pdfActions.some((a) => ['pdf_rotate', 'pdf_reorder', 'pdf_delete_pages', 'pdf_merge',
    'pdf_replace_text', 'pdf_annotate', 'pdf_watermark'].includes(a.action) && a.result === 'ok');
  const deleted = pdfActions.some((a) => a.action === 'pdf_delete' && a.result === 'ok');

  void refreshDocs().then(() => {
    if (created) {
      void openNewest();
    } else if (modified && currentPdf) {
      void openDoc({ pdf_id: currentPdf.pdf_id });
    } else if (deleted) {
      void openNewest();
    }
  });
}

async function importFromFiles(path, name) {
  try {
    if (!getPdfTileElement()) mountPdfTile();
    const file = await fileFromHome(path, name, 'application/pdf');
    await importPdf(file);
  } catch (e) {
    toast(e.message || 'Could not open file', { type: 'error' });
  }
}

let wired = false;
export function wirePdfEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  onOpenFromFiles(PDF_PLUGIN, (d) => importFromFiles(d.path, d.name));
  document.addEventListener('mousemove', onSelMove);
  document.addEventListener('mouseup', onSelUp);
  document.addEventListener('pointerdown', (e) => {
    if (annotBar && !annotBar.contains(e.target)) hideAnnotBar();
  });
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators + window management. */
export function pdfContextMenu(ctx) {
  const hasPdf = !!currentPdf;
  const pages = currentPdf?.page_count ?? 0;
  return [
    { type: 'item', label: 'New PDF', icon: 'ui/plus', onClick: () => void newPdf() },
    { type: 'item', label: 'Import .pdf', icon: 'ui/download', onClick: pickPdfFile },
    { type: 'item', label: 'Save to Documents', icon: 'ui/save', disabled: !hasPdf, onClick: () => void exportCurrent() },
    { type: 'separator' },
    {
      type: 'submenu',
      label: 'Page',
      icon: 'ui/doc',
      items: [
        { type: 'item', label: 'Previous page', icon: 'ui/chevron-left', disabled: !hasPdf || currentPage <= 0, onClick: () => gotoPage(-1) },
        { type: 'item', label: 'Next page', icon: 'ui/chevron-right', disabled: !hasPdf || currentPage >= pages - 1, onClick: () => gotoPage(1) },
        { type: 'separator' },
        { type: 'item', label: 'Rotate left', icon: 'ui/rotate-left', disabled: !hasPdf, onClick: () => void rotateCurrent(-90) },
        { type: 'item', label: 'Rotate right', icon: 'ui/rotate-right', disabled: !hasPdf, onClick: () => void rotateCurrent(90) },
      ],
    },
    { type: 'separator' },
    { type: 'item', label: 'Delete PDF', icon: 'ui/trash', danger: true, disabled: !hasPdf, onClick: () => void removeCurrent() },
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
