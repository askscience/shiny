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
  icon, button, emptyState, toast,
  setTileGlow, setTileGlowFromUrl,
} from '/ui/index.js';
import { setIcon } from '/ui/index.js';
import { apiFetch } from '/js/api.js';

export const PDF_PLUGIN = 'pdf';

const THUMB_DPI = 48;
const ZOOMS = [75, 100, 120, 150, 200, 300];
const DEFAULT_ZOOM = 2; // 120 dpi

let tileEl = null;
let titleInput = null;
let docMenuBtn = null;
let statusEl = null;
let saveDot = null;
let railEl = null;
let canvasEl = null;
let pageLabelEl = null;
let zoomLabelEl = null;

let docs = [];
let currentPdf = null; // { pdf_id, title, page_count, updated_at }
let currentPage = 0;
let zoomIndex = DEFAULT_ZOOM;
let busy = false;

/* Popup (body-level, the tile clips overflow) */
let docMenuPopup = null;
let docMenuOpen = false;
let menuMode = 'default'; // 'default' | 'merge'

/* Rendered-page cache: key `${id}:${page}:${dpi}` -> object URL */
const renderCache = new Map();
let dragIndex = null;

/* Region selection + annotation state */
let pageImgEl = null;
let selOverlay = null;
let selStart = null;         // {x,y} in displayed px relative to the page image
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

async function annotatePdf(kind, rect, text) {
  const res = await apiFetch(`/api/pdfs/${encodeURIComponent(currentPdf.pdf_id)}/annotate`, {
    method: 'POST',
    body: JSON.stringify({ page: currentPage, kind, rect, text }),
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

/* ── Page rendering ─────────────────────────────────────────── */

function pageKey(id, page, dpi) {
  return `${id}:${page}:${dpi}`;
}

function clearDocCache(id) {
  for (const key of [...renderCache.keys()]) {
    if (key.startsWith(`${id}:`)) {
      URL.revokeObjectURL(renderCache.get(key));
      renderCache.delete(key);
    }
  }
}

async function pageUrl(id, page, dpi) {
  const key = pageKey(id, page, dpi);
  if (renderCache.has(key)) return renderCache.get(key);
  const blob = await apiFetch(
    `/api/pdfs/${encodeURIComponent(id)}/pages/${page}?dpi=${dpi}`,
    { responseType: 'blob' },
  );
  const url = URL.createObjectURL(blob);
  renderCache.set(key, url);
  return url;
}

/* ── Status ─────────────────────────────────────────────────── */

function setStatus(text) {
  if (statusEl) statusEl.textContent = text;
}

function setBusy(on) {
  busy = on;
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

async function renderMain() {
  if (!currentPdf || !canvasEl) {
    if (!currentPdf) setTileGlow(tileEl, null);
    return;
  }
  canvasEl.innerHTML = '';
  if (currentPdf.page_count === 0) {
    setTileGlow(tileEl, null);
    canvasEl.appendChild(emptyState({ icon: 'ui/doc', title: 'Empty PDF', body: 'This document has no pages.' }));
    return;
  }
  setBusy(true);
  try {
    const url = await pageUrl(currentPdf.pdf_id, currentPage, ZOOMS[zoomIndex]);
    const thumb = await pageUrl(currentPdf.pdf_id, currentPage, THUMB_DPI).catch(() => null);
    // The window background mirrors the page, lightly blurred (pre-blurred at
    // thumbnail size, so it stays cheap to repaint while resizing).
    void setTileGlowFromUrl(tileEl, thumb || url, { size: 320, blur: 7 });
    const page = document.createElement('div');
    page.className = 'pdf-page';
    const img = document.createElement('img');
    img.className = 'pdf-page-img';
    img.src = url;
    img.alt = `Page ${currentPage + 1}`;
    page.appendChild(img);
    attachSelection(page, img);
    canvasEl.appendChild(page);
    canvasEl.scrollTop = 0;
  } catch (e) {
    canvasEl.appendChild(emptyState({ icon: 'ui/warning', title: 'Could not render page', body: e.message || '' }));
  }
  setBusy(false);
  updateNav();
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
    // Lazy-load the thumbnail image.
    pageUrl(currentPdf.pdf_id, i, THUMB_DPI)
      .then((u) => { img.src = u; })
      .catch(() => { img.remove(); });

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
    clearDocCache(currentPdf.pdf_id);
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

function pickPdfFile() {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = '.pdf,application/pdf';
  input.addEventListener('change', () => {
    const file = input.files?.[0];
    if (file) void importPdf(file);
  });
  input.click();
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
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${currentPdf.title || 'document'}.pdf`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  } catch (e) {
    toast(e.message || 'Export failed', { type: 'error' });
  }
}

async function removeCurrent() {
  if (!currentPdf) return;
  if (!window.confirm(`Delete "${currentPdf.title}"?`)) return;
  try {
    await deletePdf(currentPdf.pdf_id);
    clearDocCache(currentPdf.pdf_id);
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
    clearDocCache(currentPdf.pdf_id);
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
    clearDocCache(currentPdf.pdf_id);
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
    clearDocCache(currentPdf.pdf_id);
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
    clearDocCache(currentPdf.pdf_id);
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
  const r = pageImgEl.getBoundingClientRect();
  return { x: e.clientX - r.left, y: e.clientY - r.top };
}

/** Convert a displayed-pixel selection into PDF points (bottom-left origin). */
function dispToPdfRect(a, b) {
  const r = pageImgEl.getBoundingClientRect();
  const sx = pageImgEl.naturalWidth / Math.max(1, r.width);
  const sy = pageImgEl.naturalHeight / Math.max(1, r.height);
  const ptsPerPx = 72 / ZOOMS[zoomIndex];
  const x0 = Math.min(a.x, b.x) * sx * ptsPerPx;
  const y0 = Math.min(a.y, b.y) * sy * ptsPerPx;
  const w = Math.abs(a.x - b.x) * sx * ptsPerPx;
  const h = Math.abs(a.y - b.y) * sy * ptsPerPx;
  const pageH = pageImgEl.naturalHeight * ptsPerPx;
  return { x: x0, y: pageH - (y0 + h), w, h };
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
  const r = pageImgEl.getBoundingClientRect();
  selDisplayRect = {
    left: r.left + Math.min(start.x, end.x),
    top: r.top + Math.min(start.y, end.y),
  };
  showAnnotBar(dispToPdfRect(start, end));
}

function attachSelection(page, img) {
  pageImgEl = img;
  selStart = null;
  selOverlay = document.createElement('div');
  selOverlay.className = 'pdf-select-box hidden';
  page.appendChild(selOverlay);

  img.addEventListener('mousedown', (e) => {
    if (!currentPdf) return;
    hideAnnotBar();
    e.preventDefault();
    selStart = dispPos(e);
    selOverlay.classList.remove('hidden');
    layoutSel(selStart, selStart);
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

function showAnnotBar(rect) {
  ensureAnnotBar();
  annotRect = rect;
  annotBar.innerHTML = '';
  const mk = (label, fn) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'pdf-annot-btn';
    b.textContent = label;
    b.addEventListener('click', fn);
    annotBar.appendChild(b);
  };
  mk('Highlight', () => void doAnnotate('highlight', ''));
  mk('Underline', () => void doAnnotate('underline', ''));
  mk('Strikeout', () => void doAnnotate('strikeout', ''));
  mk('Squiggly', () => void doAnnotate('squiggly', ''));
  mk('Note', () => {
    const t = window.prompt('Note text:');
    if (t != null && t.trim()) void doAnnotate('note', t.trim());
  });
  mk('Text box', () => {
    const t = window.prompt('Text to place on the page:');
    if (t != null && t.trim()) void doAnnotate('free_text', t.trim());
  });
  mk('Link', () => {
    const t = window.prompt('Link URL:');
    if (t != null && t.trim()) void doAnnotate('link', t.trim());
  });
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
  hideAnnotBar();
  if (!currentPdf || !annotRect) return;
  const rect = [annotRect.x, annotRect.y, annotRect.w, annotRect.h];
  setBusy(true);
  try {
    await annotatePdf(kind, rect, text);
    clearDocCache(currentPdf.pdf_id);
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
    clearDocCache(currentPdf.pdf_id);
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
    clearDocCache(currentPdf.pdf_id);
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
  foot.appendChild(docMenuItem('Export .pdf', 'ui/upload', { onClick: () => void exportCurrent() }));
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

  /* File actions */
  const newBtn = toolbarButton('ui/plus', 'New PDF', () => void newPdf());
  const importBtn = toolbarButton('ui/download', 'Import .pdf', pickPdfFile);
  const exportBtn = toolbarButton('ui/upload', 'Export .pdf', () => void exportCurrent());
  const delBtn = toolbarButton('ui/trash', 'Delete PDF', () => void removeCurrent());
  delBtn.classList.add('pdf-tool--danger');

  saveDot = document.createElement('span');
  saveDot.className = 'pdf-save-dot';
  saveDot.setAttribute('aria-hidden', 'true');

  bar.append(
    docMenuBtn, titleInput,
    prevBtn, pageLabelEl, nextBtn,
    zoomOut, zoomLabelEl, zoomIn,
    rotLeft, rotRight,
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
  const modified = pdfActions.some((a) => ['pdf_rotate', 'pdf_reorder', 'pdf_delete_pages', 'pdf_merge'].includes(a.action) && a.result === 'ok');
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

let wired = false;
export function wirePdfEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  document.addEventListener('mousemove', onSelMove);
  document.addEventListener('mouseup', onSelUp);
  document.addEventListener('pointerdown', (e) => {
    if (annotBar && !annotBar.contains(e.target)) hideAnnotBar();
  });
}

export default {
  name: 'pdf',
  icon: 'ui/doc',
  mount: mountPdfTile,
  unmount: unmountPdfTile,
  getElement: getPdfTileElement,
  wireEvents: wirePdfEvents,
};
