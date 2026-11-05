import {
  menuSurface, menuItem, confirmDialog, promptDialog, menuButton, toolbar, input as uiInput, toolButton,
  button, toast, select, setIcon, modal, field, toggleRow, segmented, controlButton, placePopover,
  setTileGlow, glowGradient, compactToolbar } from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { t, tp, getLocale } from '../../js/i18n.js';
import { formatMenuDate, formatMenuTime, appendMenuFooter, showDocumentMenu } from '../../js/officeMenu.js';
import { listFonts } from '../../js/fonts.js';
import { saveOrDownload, onOpenFromFiles, fileFromHome, pickFiles, SHINY_FILE_MIME } from '../../js/files.js';
import { askInChat } from '../../js/textInput.js';
import {
  MM, PT, PAPERS, TEMPLATE_IDS, ACCENTS, DOCUMENT_FAMILIES,
  geometry, themeVars, applyVars, isThemed, contentsLabel, installDocumentFonts,
} from './page.js';

/**
 * word.js — the Word plugin's window: a page-faithful word processor.
 *
 * Layout (liteaura's editorial office pattern, like the PDF window):
 *   window bar  — document menu, editable title, undo/redo, Page | Preview,
 *                 AI, Design, Export, New
 *   format row  — paragraph style, font, size, emphasis, colours, alignment,
 *                 lists, link, picture, table, page break, outline
 *   desk        — white paper at its real size on the bare window, set in
 *                 the document's theme (page.js mirrors the engine), with a
 *                 ruler whose handles move the margins, the generated cover /
 *                 contents pages, a header preview and page guides
 *   preview     — the exported pages themselves, rendered by the server
 *                 (WordCraft layout + renderer): cover, contents, headers,
 *                 footers, page numbers exactly as the .docx/PDF
 *   status bar  — page, words, language · save state · zoom (bottom, mono)
 *
 * Content is stored as .odt (server, WordCraft codecs); the editor works on
 * HTML. The design (theme, paper, margins, cover, contents, header, page
 * numbers) is stored beside it and applied on export.
 *
 * AI: `doc_*` tool outcomes arrive via `agent:actions` (the window reloads
 * what the AI touched, saves exports); the AI menu asks the assistant in the
 * chat (`askInChat`) about the open document, which the server marks as the
 * agent's default target (`POST /api/documents/:id/opened`).
 */

export const WORD_PLUGIN = 'word';

/* ── State ──────────────────────────────────────────────────── */

let tileEl = null;
let barEl = null;
let titleInput = null;
let docMenuBtn = null;
let viewSwitch = null;
let undoBtn = null;
let redoBtn = null;
let formatRow = null;
let blockSelect = null;
let fontSelect = null;
let sizeSelect = null;
let tableTools = null;
let mainEl = null;
let navEl = null;
let deskEl = null;
let stageEl = null;
let sheetsEl = null;
let coverSheet = null;
let tocSheet = null;
let bodySheet = null;
let titleBlock = null;
let headerEl = null;
let footerEl = null;
let editorEl = null;
let guidesEl = null;
let probeEl = null;
let rulerEl = null;
let vrulerEl = null;
let previewEl = null;
let statusInfoEl = null;
let stateEl = null;
let zoomLabel = null;
let toolBtns = {};
let resizeObs = null;

let docs = [];
let currentDoc = null;   // { id, title, updated_at, design }
let dirty = false;
let saving = false;
let saveTimer = null;
let saveSeq = 0;
let view = 'page';       // 'page' | 'preview'
let zoom = 1;            // set from the saved preference below
let pageMarks = [];      // body offsets (px, unzoomed) where a new page starts
let navOpen = readPref('word.nav', '0') === '1';
let previewToken = 0;
let previewUrls = [];

let docMenuPopup = null;
let docMenuOpen = false;
let popup = null;        // the one open body-level popup (export, AI, colours)
let popupAnchor = null;

const MIN_ZOOM = 0.3;
/** Window width (px) at or under which the page reflows to it (keep in step
 *  with the `plugin-window` 560px container query in ui/responsive.css). */
const REFLOW_MAX = 560;
const ZOOM_STEPS = [0.3, 0.4, 0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];
/** Fit the page to the window until the user picks a zoom. */
let zoomFit = readPref('word.zoom', 'fit') === 'fit';
zoom = readZoom();

function readPref(key, fallback) {
  try { return localStorage.getItem(key) ?? fallback; } catch (_) { return fallback; }
}
function writePref(key, value) {
  try { localStorage.setItem(key, value); } catch (_) { /* private mode */ }
}
function readZoom() {
  const z = Number(readPref('word.zoom', 'fit'));
  return Number.isFinite(z) && z >= MIN_ZOOM && z <= 2 ? z : 1;
}

/* ── API ────────────────────────────────────────────────────── */

const enc = encodeURIComponent;

async function listDocs() {
  const res = await apiFetch('/api/documents');
  return res?.data || [];
}

async function fetchDoc(id) {
  const res = await apiFetch(`/api/documents/${enc(id)}`);
  return res?.data;
}

async function createDoc(title = 'Untitled', html = '<p></p>') {
  const res = await apiFetch('/api/documents', { method: 'POST', body: JSON.stringify({ title, html }) });
  return res?.data;
}

async function saveDoc(id, title, html) {
  await apiFetch(`/api/documents/${enc(id)}`, { method: 'PUT', body: JSON.stringify({ title, html }) });
}

async function saveDesign(id, design) {
  const res = await apiFetch(`/api/documents/${enc(id)}/design`, { method: 'PUT', body: JSON.stringify(design) });
  return res?.data?.design || design;
}

async function deleteDoc(id) {
  await apiFetch(`/api/documents/${enc(id)}`, { method: 'DELETE' });
}

function markOpened(id) {
  void apiFetch(`/api/documents/${enc(id)}/opened`, { method: 'POST' }).catch(() => {});
}

/* ── Status bar (bottom, mono) ──────────────────────────────── */

function setState(mode) {
  if (!stateEl) return;
  stateEl.parentElement?.classList.toggle('is-busy', mode === 'saving');
  stateEl.classList.toggle('is-pending', mode !== 'saved');
  if (mode === 'saving') stateEl.textContent = t('plugin.word.state.saving', null, 'Saving');
  else if (mode === 'dirty') stateEl.textContent = t('plugin.word.state.unsaved', null, 'Unsaved');
  else if (currentDoc) {
    const time = formatMenuTime(currentDoc.updated_at);
    stateEl.textContent = t('plugin.word.state.saved', { time }, `Saved ${time}`);
  } else stateEl.textContent = '';
}

function wordCount() {
  const text = editorEl?.innerText || '';
  return text.trim().split(/\s+/).filter(Boolean).length;
}

function languageName(code) {
  if (!code) return '';
  try {
    return new Intl.DisplayNames([getLocale() || 'en'], { type: 'language' }).of(code) || code;
  } catch (_) { return code; }
}

/** Pages before the body: cover, contents. */
function frontPages() {
  const d = currentDoc?.design || {};
  return (d.cover ? 1 : 0) + (d.toc && headings().length ? 1 : 0);
}

function totalPages() {
  if (view === 'preview' && previewEl?.dataset.count) return Number(previewEl.dataset.count);
  return frontPages() + pageMarks.length + 1;
}

/** The page at the top third of the view (or under the caret while typing). */
function currentPage() {
  if (!editorEl) return 1;
  if (view === 'preview') {
    const pages = [...(previewEl?.querySelectorAll('.word-preview-page') || [])];
    const mid = (previewEl?.scrollTop || 0) + (previewEl?.clientHeight || 0) / 3;
    const i = pages.findIndex((el) => el.offsetTop + el.offsetHeight > mid);
    return i < 0 ? pages.length || 1 : i + 1;
  }
  const deskTop = deskEl.getBoundingClientRect().top;
  let y = deskTop + deskEl.clientHeight / 3;
  if (document.activeElement === editorEl) {
    const r = editorRange();
    const rect = r?.getClientRects?.()[0] || r?.startContainer?.parentElement?.getBoundingClientRect?.();
    if (rect && rect.top >= deskTop && rect.top <= deskTop + deskEl.clientHeight) y = rect.top;
  }
  let page = 0;
  for (const sheetEl of [coverSheet, tocSheet]) {
    if (sheetEl.hidden) continue;
    page++;
    if (y < sheetEl.getBoundingClientRect().bottom) return page;
  }
  const er = editorEl.getBoundingClientRect();
  const scale = editorEl.offsetHeight ? er.height / editorEl.offsetHeight : zoom;
  const offset = (y - er.top) / (scale || 1);
  return page + 1 + pageMarks.filter((m) => m <= offset).length;
}

function syncStatus() {
  if (!statusInfoEl) return;
  if (!currentDoc) { statusInfoEl.textContent = ''; return; }
  const n = totalPages();
  const page = Math.min(currentPage(), n);
  const words = wordCount();
  const g = geometry(currentDoc.design);
  const parts = [
    t('plugin.word.status.page', { page, count: n }, `Page ${page} of ${n}`),
    tp('plugin.word.hero.words', words, null, `${words} words`),
    g.paper,
  ];
  const lang = languageName(currentDoc.design?.lang || getLocale());
  if (lang) parts.push(lang);
  statusInfoEl.textContent = parts.join('  ·  ');
}

/* ── Saving ─────────────────────────────────────────────────── */

function markDirty() {
  if (!currentDoc) return;
  dirty = true;
  setState('dirty');
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => void persist(), 1200);
  scheduleLayout();
}

async function persist() {
  if (!currentDoc || !dirty || saving) return;
  const target = currentDoc;
  dirty = false;
  saving = true;
  setState('saving');
  try {
    const title = (titleInput?.value || '').trim() || t('plugin.word.toolbar.titlePlaceholder', null, 'Untitled');
    await saveDoc(target.id, title, editorEl.innerHTML);
    target.title = title;
    target.updated_at = new Date().toISOString().replace('T', ' ').slice(0, 19);
    if (currentDoc === target) setState(dirty ? 'dirty' : 'saved');
  } catch (e) {
    dirty = true;
    setState('dirty');
    toast(e.message || t('plugin.word.toast.saveFailed', null, 'Save failed'), { type: 'error' });
  } finally {
    saving = false;
  }
  if (dirty) { window.clearTimeout(saveTimer); saveTimer = window.setTimeout(() => void persist(), 1200); }
}

/* ── Glow ───────────────────────────────────────────────────── */

function updateGlow() {
  if (!tileEl) return;
  if (!currentDoc) { setTileGlow(tileEl, null); return; }
  const s = String(currentDoc.title || 'Untitled');
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  const hue = 28 + (Math.abs(h) % 26);
  setTileGlow(tileEl, glowGradient(`hsl(${hue} 62% 62%)`, `hsl(${(hue + 210) % 360} 45% 26%)`));
}

/* ── Documents ──────────────────────────────────────────────── */

async function refreshDocs() {
  try { docs = await listDocs(); } catch (_) { /* keep the last list */ }
}

async function openDoc(doc) {
  window.clearTimeout(saveTimer);
  if (dirty && currentDoc) { try { await persist(); } catch (_) { /* keep going */ } }
  dirty = false;
  saveSeq++;
  try {
    const full = doc.html != null && doc.design ? doc : await fetchDoc(doc.id);
    currentDoc = { id: doc.id, title: full.title, updated_at: full.updated_at, design: full.design || {} };
    editorEl.innerHTML = full.html || '<p><br></p>';
    if (titleInput) titleInput.value = full.title || '';
    tileEl?.classList.remove('is-empty');
    applyDesign();
    setState('saved');
    updateGlow();
    if (deskEl) deskEl.scrollTop = 0;
    markOpened(doc.id);
    if (view === 'preview') void loadPreview();
  } catch (e) {
    toast(e.message || t('plugin.word.toast.openFailed', null, 'Could not open document'), { type: 'error' });
  }
}

async function openNewest() {
  await refreshDocs();
  if (docs.length) await openDoc(docs[0]);
  else showEmpty();
}

function showEmpty() {
  currentDoc = null;
  if (editorEl) editorEl.innerHTML = '';
  if (titleInput) titleInput.value = '';
  tileEl?.classList.add('is-empty');
  applyDesign();
  setState('saved');
  updateGlow();
}

async function newDocument() {
  try {
    if (dirty) await persist();
    const created = await createDoc(t('plugin.word.toolbar.titlePlaceholder', null, 'Untitled'));
    await refreshDocs();
    await openDoc(created);
    titleInput?.select();
    titleInput?.focus();
  } catch (e) {
    toast(e.message || t('plugin.word.toast.createFailed', null, 'Could not create document'), { type: 'error' });
  }
}

async function removeCurrent() {
  if (!currentDoc) return;
  const target = currentDoc;
  if (!await confirmDialog(t('plugin.word.dialog.deleteDoc', { title: target.title }, `Delete "${target.title}"?`))) return;
  if (currentDoc !== target || !tileEl) return;
  try {
    await deleteDoc(target.id);
    dirty = false;
    currentDoc = null;
    await openNewest();
  } catch (e) {
    toast(e.message || t('plugin.word.toast.deleteFailed', null, 'Could not delete document'), { type: 'error' });
  }
}

/* ── Page: design → paper ───────────────────────────────────── */

/** Apply the current design to the paper: size, margins, theme, generated parts. */
function applyDesign() {
  if (!sheetsEl) return;
  const d = currentDoc?.design || {};
  applyVars(sheetsEl, themeVars(d));
  sheetsEl.dataset.template = (d.template || 'plain').toLowerCase();
  sheetsEl.classList.toggle('is-themed', isThemed(d));
  renderGeneratedParts();
  renderRulers();
  scheduleLayout();
}

function escapeText(s) {
  const span = document.createElement('span');
  span.textContent = s;
  return span.innerHTML;
}

/** Cover page, title block and running header: generated from the design. */
function renderGeneratedParts() {
  const d = currentDoc?.design || {};
  const title = (titleInput?.value || currentDoc?.title || '').trim();
  const meta = [d.author, d.date].map((s) => (s || '').trim()).filter(Boolean);

  coverSheet.hidden = !d.cover || !currentDoc;
  if (d.cover) {
    coverSheet.querySelector('.word-cover-title').textContent = title;
    coverSheet.querySelector('.word-cover-subtitle').textContent = (d.subtitle || '').trim();
    coverSheet.querySelector('.word-cover-meta').innerHTML = meta.map(escapeText).join('<br>');
  }

  // Themed documents without a cover start with a title block.
  const block = !d.cover && isThemed(d) && !!currentDoc;
  titleBlock.hidden = !block;
  if (block) {
    titleBlock.querySelector('.word-tb-title').textContent = title;
    const sub = titleBlock.querySelector('.word-tb-subtitle');
    sub.textContent = (d.subtitle || '').trim();
    sub.hidden = !sub.textContent;
    const m = titleBlock.querySelector('.word-tb-meta');
    m.textContent = meta.join('  ·  ');
    m.hidden = !m.textContent;
  }

  headerEl.hidden = !d.header || !currentDoc;
  if (d.header) {
    headerEl.querySelector('.word-hf-left').textContent = title;
    headerEl.querySelector('.word-hf-right').textContent = (d.subtitle || d.author || '').trim();
  }
  footerEl.hidden = !d.page_numbers || !currentDoc;
}

/** Headings 1–3 of the body, in order. */
function headings() {
  if (!editorEl) return [];
  return [...editorEl.querySelectorAll('h1, h2, h3')].filter((h) => h.textContent.trim());
}

/** The contents page: one entry per heading, with its page. */
function renderToc() {
  const d = currentDoc?.design || {};
  const hs = headings();
  const show = !!d.toc && hs.length > 0 && !!currentDoc;
  tocSheet.hidden = !show;
  if (!show) return;
  const list = tocSheet.querySelector('.word-toc-list');
  tocSheet.querySelector('.word-toc-head').textContent = contentsLabel(d.lang || getLocale());
  list.textContent = '';
  const first = frontPages() + 1;
  const top0 = editorEl.offsetTop;
  for (const h of hs) {
    const y = h.offsetTop - top0;
    const page = first + pageMarks.filter((m) => m <= y).length;
    const row = document.createElement('div');
    row.className = `word-toc-entry word-toc-entry--${h.tagName.toLowerCase()}`;
    const text = document.createElement('span');
    text.className = 'word-toc-text';
    text.textContent = h.textContent.trim();
    const lead = document.createElement('span');
    lead.className = 'word-toc-leader';
    const num = document.createElement('span');
    num.className = 'word-toc-page';
    num.textContent = String(page);
    row.append(text, lead, num);
    row.addEventListener('click', () => scrollToBlock(h));
    list.appendChild(row);
  }
}

/* ── Page guides ────────────────────────────────────────────── */

let layoutQueued = false;
function scheduleLayout() {
  if (layoutQueued) return;
  layoutQueued = true;
  requestAnimationFrame(() => {
    layoutQueued = false;
    layoutPages();
  });
}

/**
 * Where pages end in the body: every (page height − margins) of content, or
 * earlier at a forced page break. Measured against a 100 mm probe, so the
 * numbers hold at any zoom.
 */
function layoutPages() {
  if (!editorEl || !bodySheet) return;
  const g = geometry(currentDoc?.design);
  const per100 = probeEl.offsetHeight || 100 * MM;
  const pxPerMm = per100 / 100;
  // The title block shares page one with the content.
  const lead = titleBlock.hidden ? 0 : titleBlock.offsetHeight;
  const pageContent = (g.h - g.top - g.bottom) * pxPerMm;
  const top0 = editorEl.offsetTop;
  const total = editorEl.offsetHeight;
  const breaks = [...editorEl.children]
    .filter((el) => el.style?.breakBefore === 'page' || el.style?.pageBreakBefore === 'always')
    .map((el) => el.offsetTop - top0)
    .filter((y) => y > 0);
  const marks = [];
  let start = -lead;
  let bi = 0;
  for (let guard = 0; guard < 1000; guard++) {
    const natural = start + pageContent;
    const forced = breaks[bi];
    if (forced != null && forced <= natural) {
      marks.push(forced); start = forced; bi++;
    } else if (natural < total) {
      marks.push(natural); start = natural;
    } else break;
  }
  pageMarks = marks;

  guidesEl.textContent = '';
  const first = frontPages() + 1;
  marks.forEach((y, i) => {
    const gEl = document.createElement('div');
    gEl.className = 'word-guide';
    gEl.style.top = `${top0 + y}px`;
    const label = document.createElement('span');
    label.textContent = t('plugin.word.page.n', { page: first + i + 1 }, `Page ${first + i + 1}`);
    gEl.appendChild(label);
    guidesEl.appendChild(gEl);
  });
  const pages = marks.length + 1;
  bodySheet.style.minHeight = `${g.h * pages}mm`;
  const n = frontPages() + pages;
  footerEl.querySelector('.word-hf-center').textContent = `${n} / ${n}`;
  renderToc();
  renderNav();
  syncStatus();
}

/* ── Rulers: drag the margins ───────────────────────────────── */

function renderRulers() {
  if (!rulerEl) return;
  const g = geometry(currentDoc?.design);
  rulerEl.style.width = `${g.w}mm`;
  vrulerEl.style.height = `${g.h}mm`;
  for (const [el, len, start, end, axis] of [
    [rulerEl, g.w, g.left, g.w - g.right, 'x'],
    [vrulerEl, g.h, g.top, g.h - g.bottom, 'y'],
  ]) {
    const ticks = el.querySelector('.word-ruler-ticks');
    ticks.textContent = '';
    for (let mm = 5; mm < len; mm += 5) {
      const tick = document.createElement('span');
      tick.className = mm % 10 === 0 ? 'word-tick word-tick--cm' : 'word-tick';
      tick.style[axis === 'x' ? 'left' : 'top'] = `${mm}mm`;
      if (mm % 10 === 0) tick.dataset.cm = String(mm / 10);
      ticks.appendChild(tick);
    }
    const [a, b] = el.querySelectorAll('.word-ruler-margin');
    const [ha, hb] = el.querySelectorAll('.word-ruler-handle');
    const prop = axis === 'x' ? ['left', 'width'] : ['top', 'height'];
    a.style[prop[0]] = '0'; a.style[prop[1]] = `${start}mm`;
    b.style[prop[0]] = `${end}mm`; b.style[prop[1]] = `${len - end}mm`;
    ha.style[prop[0]] = `${start}mm`;
    hb.style[prop[0]] = `${end}mm`;
  }
}

const MARGIN_KEYS = { left: 'margin_left', right: 'margin_right', top: 'margin_top', bottom: 'margin_bottom' };

function wireRulerHandle(handle, side) {
  handle.addEventListener('pointerdown', (e) => {
    if (!currentDoc) return;
    e.preventDefault();
    handle.setPointerCapture(e.pointerId);
    const vertical = side === 'top' || side === 'bottom';
    const origin = vertical ? e.clientY : e.clientX;
    const g0 = geometry(currentDoc.design);
    const start = g0[side];
    const pxPerMm = (probeEl.getBoundingClientRect().height || 100 * MM * zoom) / 100;
    const sign = side === 'left' || side === 'top' ? 1 : -1;
    const limit = vertical ? g0.h : g0.w;
    const other = { left: g0.right, right: g0.left, top: g0.bottom, bottom: g0.top }[side];
    handle.classList.add('is-dragging');
    const move = (ev) => {
      const delta = ((vertical ? ev.clientY : ev.clientX) - origin) / pxPerMm;
      const v = Math.round(Math.min(80, Math.max(5, start + sign * delta), limit - other - 40) * 2) / 2;
      currentDoc.design = { ...currentDoc.design, [MARGIN_KEYS[side]]: v };
      handle.dataset.value = `${(v / 10).toFixed(1)} cm`;
      applyDesign();
    };
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
      handle.removeEventListener('pointercancel', up);
      handle.classList.remove('is-dragging');
      delete handle.dataset.value;
      void commitDesign(currentDoc.design);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
    handle.addEventListener('pointercancel', up);
  });
  handle.addEventListener('dblclick', () => {
    if (!currentDoc) return;
    currentDoc.design = { ...currentDoc.design, [MARGIN_KEYS[side]]: 0 };
    applyDesign();
    void commitDesign(currentDoc.design);
  });
}

function ruler(axis) {
  const el = document.createElement('div');
  el.className = `word-ruler word-ruler--${axis}`;
  el.setAttribute('aria-hidden', 'true');
  const ticks = document.createElement('div');
  ticks.className = 'word-ruler-ticks';
  el.append(ticks);
  for (let i = 0; i < 2; i++) {
    const m = document.createElement('div');
    m.className = 'word-ruler-margin';
    el.appendChild(m);
  }
  const sides = axis === 'x' ? ['left', 'right'] : ['top', 'bottom'];
  for (const side of sides) {
    const h = controlButton();
    h.className = `word-ruler-handle word-ruler-handle--${side}`;
    h.tabIndex = -1;
    h.title = t(`plugin.word.ruler.${side}`, null, `${side[0].toUpperCase()}${side.slice(1)} margin — drag; double-click resets`);
    wireRulerHandle(h, side);
    el.appendChild(h);
  }
  return el;
}

async function commitDesign(design) {
  const target = currentDoc;
  if (!target) return;
  try {
    target.design = await saveDesign(target.id, { ...design, lang: design.lang || getLocale() || '' });
    if (currentDoc === target) {
      applyDesign();
      if (view === 'preview') void loadPreview();
    }
  } catch (e) {
    toast(e.message || t('plugin.word.toast.saveFailed', null, 'Save failed'), { type: 'error' });
  }
}

/* ── Outline (navigation pane) ──────────────────────────────── */

function renderNav() {
  if (!navEl || navEl.hidden) return;
  const list = navEl.querySelector('.word-nav-list');
  list.textContent = '';
  const hs = headings();
  if (!hs.length) {
    const empty = document.createElement('p');
    empty.className = 'word-nav-empty';
    empty.textContent = t('plugin.word.nav.empty', null, 'Headings you add appear here');
    list.appendChild(empty);
    return;
  }
  for (const h of hs) {
    const item = controlButton();
    item.className = `word-nav-item word-nav-item--${h.tagName.toLowerCase()}`;
    item.textContent = h.textContent.trim();
    item.addEventListener('click', () => scrollToBlock(h));
    list.appendChild(item);
  }
}

function toggleNav(force) {
  navOpen = typeof force === 'boolean' ? force : !navOpen;
  writePref('word.nav', navOpen ? '1' : '0');
  if (navEl) navEl.hidden = !navOpen;
  toolBtns.nav?.classList.toggle('is-active', navOpen);
  toolBtns.nav?.setAttribute('aria-pressed', String(navOpen));
  renderNav();
}

function scrollToBlock(el) {
  if (view !== 'page') setView('page');
  el.scrollIntoView({ block: 'start', behavior: 'smooth' });
  const range = document.createRange();
  range.selectNodeContents(el);
  range.collapse(true);
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
  editorEl.focus({ preventScroll: true });
}

/* ── Zoom ───────────────────────────────────────────────────── */

function setZoom(z, { fit = false } = {}) {
  zoom = Math.min(2, Math.max(MIN_ZOOM, Math.round(z * 100) / 100));
  zoomFit = fit;
  writePref('word.zoom', fit ? 'fit' : String(zoom));
  if (stageEl) stageEl.style.zoom = String(zoom);
  if (zoomLabel) zoomLabel.textContent = `${Math.round(zoom * 100)}%`;
  if (view === 'preview') sizePreviewPages();
}

function zoomBy(dir) {
  const i = ZOOM_STEPS.findIndex((s) => s >= zoom - 0.001);
  const base = i < 0 ? ZOOM_STEPS.length - 1 : i;
  const next = dir > 0 ? ZOOM_STEPS[Math.min(ZOOM_STEPS.length - 1, base + (ZOOM_STEPS[base] > zoom + 0.001 ? 0 : 1))]
    : ZOOM_STEPS[Math.max(0, base - 1)];
  setZoom(next);
}

/** Fit the page (plus the rulers' gutter) to the view; stays on until a manual zoom. */
function fitWidth() {
  const g = geometry(currentDoc?.design);
  const host = view === 'preview' ? previewEl : deskEl;
  // A phone-narrow window reflows the text to its width instead of shrinking
  // a whole page into it (responsive.css, `plugin-window` ≤ 560px): there
  // "fit" is simply the text at its own size.
  if (view !== 'preview' && host && host.clientWidth && host.clientWidth <= REFLOW_MAX) {
    setZoom(1, { fit: true });
    return;
  }
  const avail = (host?.clientWidth || 800) - (window.matchMedia?.('(pointer: coarse)').matches ? 24 : 72);
  setZoom(Math.min(1, avail / (g.w * MM)), { fit: true });
}

/* ── Preview (server-rendered pages) ────────────────────────── */

function setView(v) {
  view = v === 'preview' ? 'preview' : 'page';
  viewSwitch?.setValue(view, { silent: true });
  tileEl?.classList.toggle('is-preview', view === 'preview');
  if (deskEl) deskEl.hidden = view === 'preview';
  if (previewEl) previewEl.hidden = view !== 'preview';
  formatRow?.classList.toggle('is-disabled', view === 'preview');
  if (view === 'preview') void loadPreview();
  syncStatus();
}

function clearPreview() {
  previewUrls.forEach((u) => URL.revokeObjectURL(u));
  previewUrls = [];
  if (previewEl) { previewEl.textContent = ''; delete previewEl.dataset.count; }
}

function sizePreviewPages() {
  previewEl?.querySelectorAll('.word-preview-page').forEach((el) => {
    el.style.width = `${Number(el.dataset.w) * PT * zoom}px`;
    el.style.height = `${Number(el.dataset.h) * PT * zoom}px`;
  });
}

async function loadPreview() {
  if (!currentDoc || !previewEl) return;
  const token = ++previewToken;
  if (dirty) await persist();
  const target = currentDoc;
  clearPreview();
  previewEl.classList.add('is-loading');
  try {
    const res = await apiFetch(`/api/documents/${enc(target.id)}/pages`);
    if (token !== previewToken || currentDoc !== target) return;
    const pages = res?.data?.pages || [];
    previewEl.dataset.count = String(pages.length);
    const scale = Math.min(4, PT * Math.max(1, zoom) * (window.devicePixelRatio || 1));
    const io = new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        io.unobserve(e.target);
        const el = e.target;
        void (async () => {
          try {
            const blob = await apiFetch(`/api/documents/${enc(target.id)}/page/${el.dataset.n}?scale=${scale.toFixed(2)}`, { responseType: 'blob' });
            if (token !== previewToken) return;
            const url = URL.createObjectURL(blob);
            previewUrls.push(url);
            const img = document.createElement('img');
            img.alt = t('plugin.word.page.n', { page: el.dataset.n }, `Page ${el.dataset.n}`);
            img.src = url;
            el.appendChild(img);
            el.classList.add('is-ready');
          } catch (_) { el.classList.add('is-failed'); }
        })();
      }
    }, { root: previewEl, rootMargin: '600px 0px' });
    pages.forEach((p, i) => {
      const el = document.createElement('div');
      el.className = 'word-preview-page';
      el.dataset.n = String(i + 1);
      el.dataset.w = String(p.w);
      el.dataset.h = String(p.h);
      const num = document.createElement('span');
      num.className = 'word-preview-num';
      num.textContent = String(i + 1);
      el.appendChild(num);
      previewEl.appendChild(el);
      io.observe(el);
    });
    sizePreviewPages();
    syncStatus();
  } catch (e) {
    toast(e.message || t('plugin.word.toast.previewFailed', null, 'Could not render the pages'), { type: 'error' });
  } finally {
    previewEl.classList.remove('is-loading');
  }
}

/* ── Editing ────────────────────────────────────────────────── */

function exec(command, value = null) {
  if (!editorEl || view !== 'page') return;
  editorEl.focus({ preventScroll: true });
  document.execCommand(command, false, value);
  editorEl.dispatchEvent(new Event('input'));
  syncToolbar();
}

/** The current selection, only inside the editor. */
function editorRange() {
  const sel = window.getSelection();
  if (!sel || !sel.rangeCount) return null;
  const r = sel.getRangeAt(0);
  return editorEl?.contains(r.commonAncestorContainer) ? r : null;
}

let savedRange = null;
function rememberSelection() { const r = editorRange(); if (r) savedRange = r.cloneRange(); }
function restoreSelection() {
  if (!savedRange) return;
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(savedRange);
}

/** The editor's direct child holding the caret. */
function currentBlock() {
  const r = editorRange();
  let n = r?.startContainer;
  while (n && n.parentNode !== editorEl) n = n.parentNode;
  return n && n.nodeType === 1 ? n : null;
}

const BLOCKS = [
  { value: 'p', key: 'normal', fallback: 'Normal' },
  { value: 'h1', key: 'h1', fallback: 'Heading 1' },
  { value: 'h2', key: 'h2', fallback: 'Heading 2' },
  { value: 'h3', key: 'h3', fallback: 'Heading 3' },
  { value: 'blockquote', key: 'quote', fallback: 'Quote' },
];

/** Wrap the selection in a span with `css` (font family, size). */
function styleSelection(css) {
  restoreSelection();
  const r = editorRange();
  if (!r) return;
  if (r.collapsed) {
    // No selection: the paragraph at the caret.
    const block = currentBlock();
    if (!block) return;
    r.selectNodeContents(block);
  }
  const span = document.createElement('span');
  Object.assign(span.style, css);
  try { r.surroundContents(span); } catch (_) {
    span.appendChild(r.extractContents());
    r.insertNode(span);
  }
  // Inner spans carrying the same property would win: clear them.
  for (const prop of Object.keys(css)) span.querySelectorAll('span').forEach((s) => s.style.removeProperty(prop.replace(/[A-Z]/g, (m) => `-${m.toLowerCase()}`)));
  editorEl.dispatchEvent(new Event('input'));
  syncToolbar();
}

function insertTable() {
  const head = t('plugin.word.table.column', null, 'Column');
  const cells = (r) => [1, 2, 3].map((c) => `<td>${r === 0 ? `${head} ${c}` : '<br>'}</td>`).join('');
  exec('insertHTML', `<table><tbody>${[0, 1, 2].map((r) => `<tr>${cells(r)}</tr>`).join('')}</tbody></table><p><br></p>`);
}

function caretCell() {
  const r = editorRange();
  let n = r?.startContainer;
  while (n && n !== editorEl) {
    if (n.nodeType === 1 && (n.tagName === 'TD' || n.tagName === 'TH')) return n;
    n = n.parentNode;
  }
  return null;
}

function tableEdit(op) {
  const cell = caretCell();
  if (!cell) return;
  const row = cell.parentElement;
  const table = cell.closest('table');
  const col = [...row.children].indexOf(cell);
  if (op === 'row') {
    const nr = row.cloneNode(true);
    nr.querySelectorAll('td,th').forEach((c) => { c.innerHTML = '<br>'; });
    row.after(nr);
  } else if (op === 'col') {
    table.querySelectorAll('tr').forEach((tr) => {
      const c = document.createElement(tr.children[col]?.tagName === 'TH' ? 'th' : 'td');
      c.innerHTML = '<br>';
      tr.children[col]?.after(c);
    });
  } else if (op === 'delrow') {
    if (table.querySelectorAll('tr').length > 1) row.remove(); else table.remove();
  } else if (op === 'delcol') {
    const rows = [...table.querySelectorAll('tr')];
    if (rows[0].children.length > 1) rows.forEach((tr) => tr.children[col]?.remove()); else table.remove();
  }
  editorEl.dispatchEvent(new Event('input'));
  syncToolbar();
}

/** Start a new page at the caret: the next paragraph breaks before. */
function insertPageBreak() {
  if (view !== 'page') return;
  editorEl.focus({ preventScroll: true });
  document.execCommand('insertParagraph');
  const block = currentBlock();
  if (block) {
    block.style.breakBefore = 'page';
    block.style.pageBreakBefore = 'always';
  }
  editorEl.dispatchEvent(new Event('input'));
}

async function insertLink() {
  rememberSelection();
  const url = await promptDialog(t('plugin.word.dialog.link', null, 'Link address'), 'https://');
  if (!url || !/^(https?:|mailto:)/i.test(url.trim())) return;
  restoreSelection();
  const r = editorRange();
  if (r && r.collapsed) exec('insertHTML', `<a href="${url.trim().replace(/"/g, '&quot;')}">${escapeText(url.trim())}</a>`);
  else exec('createLink', url.trim());
}

function clearFormatting() {
  exec('removeFormat');
  const block = currentBlock();
  if (block && block.tagName !== 'P' && !['UL', 'OL', 'TABLE'].includes(block.tagName)) exec('formatBlock', '<p>');
}

/* Colours: a small swatch popover. */
const TEXT_COLORS = ['', '#1f2328', '#5b636e', '#1d5fbf', '#1f3864', '#0f766e', '#2f7d32', '#b45309', '#b91c1c', '#8b1e3f', '#6d28d9'];
const MARK_COLORS = ['', '#fff3a3', '#d9f99d', '#bae6fd', '#fbcfe8', '#fed7aa', '#e5e7eb'];

function openSwatches(anchor, kind) {
  rememberSelection();
  closePopup();
  const pop = menuSurface({ className: 'word-popup word-swatches', onEscape: closePopup });
  const colors = kind === 'mark' ? MARK_COLORS : TEXT_COLORS;
  for (const c of colors) {
    const b = controlButton();
    b.className = 'word-swatch';
    b.setAttribute('role', 'menuitem');
    if (c) b.style.setProperty('--swatch', c);
    else b.classList.add('word-swatch--auto');
    b.title = c || t('plugin.word.color.auto', null, 'Automatic');
    b.addEventListener('click', () => {
      closePopup();
      restoreSelection();
      if (kind === 'mark') exec('hiliteColor', c || 'transparent');
      else exec('foreColor', c || themeVars(currentDoc?.design)['--doc-ink']);
    });
    pop.appendChild(b);
  }
  openPopup(pop, anchor);
}

function openPopup(pop, anchor) {
  popup = pop;
  popupAnchor = anchor;
  document.body.appendChild(pop);
  pop.classList.remove('hidden');
  placePopover(pop, anchor);
  pop.focusFirst?.();
  anchor.setAttribute('aria-expanded', 'true');
  document.addEventListener('pointerdown', onPopupOutside, true);
}

function closePopup() {
  if (!popup) return;
  popup.remove();
  popupAnchor?.setAttribute('aria-expanded', 'false');
  popup = null;
  popupAnchor = null;
  document.removeEventListener('pointerdown', onPopupOutside, true);
}

function onPopupOutside(e) {
  if (popup && !popup.contains(e.target) && !popupAnchor?.contains(e.target)) closePopup();
}

function popupMenu(anchor, items) {
  closePopup();
  const pop = menuSurface({ className: 'word-popup word-menu', onEscape: closePopup });
  for (const it of items) {
    if (it.separator) {
      const sep = document.createElement('div');
      sep.className = 'word-menu-sep';
      pop.appendChild(sep);
      continue;
    }
    const item = menuItem({ label: it.label, className: 'word-menu-item', disabled: it.disabled, onClick: () => { closePopup(); it.onClick(); } });
    item.setAttribute('role', 'menuitem');
    if (it.icon) {
      const ic = document.createElement('span');
      ic.className = 'word-menu-icon';
      item.prepend(ic);
      void setIcon(ic, it.icon, { size: 14 });
    }
    if (it.hint) {
      const h = document.createElement('span');
      h.className = 'word-menu-hint';
      h.textContent = it.hint;
      item.appendChild(h);
    }
    pop.appendChild(item);
  }
  openPopup(pop, anchor);
}

/* ── Toolbar state ──────────────────────────────────────────── */

function syncToolbar() {
  if (!editorEl || view !== 'page') return;
  const inEditor = !!editorRange();
  const q = (c) => { try { return inEditor && document.queryCommandState(c); } catch (_) { return false; } };
  const state = {
    bold: q('bold'), italic: q('italic'), underline: q('underline'), strike: q('strikeThrough'),
    ul: q('insertUnorderedList'), ol: q('insertOrderedList'),
    left: q('justifyLeft'), center: q('justifyCenter'), right: q('justifyRight'), justify: q('justifyFull'),
    nav: navOpen,
  };
  for (const [key, btn] of Object.entries(toolBtns)) {
    if (key in state) {
      btn.classList.toggle('is-active', !!state[key]);
      btn.setAttribute('aria-pressed', String(!!state[key]));
    }
  }
  const block = currentBlock();
  const tag = (block?.tagName || 'P').toLowerCase();
  if (blockSelect) blockSelect.select.value = BLOCKS.some((b) => b.value === tag) ? tag : 'p';
  // Font and size at the caret.
  const r = editorRange();
  let n = r?.startContainer;
  if (n?.nodeType === 3) n = n.parentElement;
  if (n && fontSelect) {
    const cs = getComputedStyle(n);
    const family = (n.closest?.('[style*="font-family"]')?.style.fontFamily || '').split(',')[0].replace(/["']/g, '').trim();
    ensureOption(fontSelect, family);
    fontSelect.select.value = family;
    const pt = Math.round(parseFloat(cs.fontSize) / PT * 2) / 2;
    const own = n.closest?.('[style*="font-size"]') ? String(pt) : '';
    ensureOption(sizeSelect, own);
    sizeSelect.select.value = own;
  }
  if (tableTools) tableTools.hidden = !caretCell();
  syncStatus();
}

function ensureOption(sel, value) {
  if (!sel || !value) return;
  if ([...sel.select.options].some((o) => o.value === value)) return;
  const opt = document.createElement('option');
  opt.value = value;
  opt.textContent = value;
  sel.select.appendChild(opt);
}

/* ── Pictures & drops ───────────────────────────────────────── */

const IMAGE_MAX_W = 600;          // CSS px ≈ the A4 text width (engine PICTURE_MAX_W)
const IMAGE_MAX_SIDE = 1600;
const IMAGE_EXTS = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'];
const DOC_EXTS = ['odt', 'docx', 'rtf', 'md', 'markdown', 'html', 'htm', 'txt'];
const IMPORT_ACCEPT = [
  '.odt', '.docx', '.rtf', '.md', '.markdown', '.html', '.htm', '.txt',
  'application/vnd.oasis.opendocument.text',
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
].join(',');
const MIME_BY_EXT = {
  odt: 'application/vnd.oasis.opendocument.text',
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  rtf: 'application/rtf', md: 'text/markdown', html: 'text/html', htm: 'text/html', txt: 'text/plain',
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', bmp: 'image/bmp',
};
const extOf = (name) => String(name || '').toLowerCase().split('.').pop();
const isImageFile = (f) => (f?.type || '').startsWith('image/') || IMAGE_EXTS.includes(extOf(f?.name));

function blobToDataUrl(blob) {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result));
    r.onerror = () => reject(r.error || new Error('read failed'));
    r.readAsDataURL(blob);
  });
}

/** `{ src, width, height }`: small PNG/JPEG/GIF kept, the rest scaled and re-encoded. */
async function preparePicture(blob) {
  const bmp = await createImageBitmap(blob);
  const { width: w0, height: h0 } = bmp;
  const scale = Math.min(1, IMAGE_MAX_SIDE / Math.max(w0, h0));
  const type = (blob.type || '').toLowerCase();
  const keep = ['image/png', 'image/jpeg', 'image/gif'].includes(type) && scale === 1 && blob.size <= 1.5 * 1024 * 1024;
  let src;
  if (keep) src = await blobToDataUrl(blob);
  else {
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(w0 * scale));
    canvas.height = Math.max(1, Math.round(h0 * scale));
    canvas.getContext('2d').drawImage(bmp, 0, 0, canvas.width, canvas.height);
    src = type === 'image/png' ? canvas.toDataURL('image/png') : canvas.toDataURL('image/jpeg', 0.86);
  }
  bmp.close?.();
  const width = Math.min(IMAGE_MAX_W, w0);
  return { src, width, height: Math.round(h0 * (width / w0)) };
}

function caretAtPoint(x, y) {
  let range = null;
  if (document.caretRangeFromPoint) range = document.caretRangeFromPoint(x, y);
  else if (document.caretPositionFromPoint) {
    const pos = document.caretPositionFromPoint(x, y);
    if (pos) { range = document.createRange(); range.setStart(pos.offsetNode, pos.offset); }
  }
  if (!range || !editorEl.contains(range.startContainer)) return;
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
}

async function insertPictures(blobs) {
  if (!currentDoc || !blobs.length) return;
  if (view !== 'page') setView('page');
  for (const blob of blobs) {
    try {
      const { src, width, height } = await preparePicture(blob);
      const alt = String(blob.name || '').replace(/\.[^.]+$/, '').replace(/"/g, '');
      exec('insertHTML', `<img src="${src}" alt="${alt}" width="${width}" height="${height}">`);
    } catch (_) {
      toast(t('plugin.word.toast.imageFailed', null, 'Could not insert the picture'), { type: 'error' });
    }
  }
}

async function pickPictures() {
  if (!currentDoc) return;
  rememberSelection();
  const files = await pickFiles({ accept: IMAGE_EXTS.map((e) => `.${e}`).concat('image/*').join(','), multiple: true });
  restoreSelection();
  if (files.length) await insertPictures(files);
}

function wireDrops() {
  const accepts = (e) => {
    const types = [...(e.dataTransfer?.types || [])];
    return types.includes(SHINY_FILE_MIME) || types.includes('Files');
  };
  mainEl.addEventListener('dragover', (e) => {
    if (!accepts(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    tileEl?.classList.add('is-dropping');
  });
  mainEl.addEventListener('dragleave', (e) => {
    if (!mainEl.contains(e.relatedTarget)) tileEl?.classList.remove('is-dropping');
  });
  mainEl.addEventListener('drop', (e) => {
    if (!accepts(e)) return;
    e.preventDefault();
    tileEl?.classList.remove('is-dropping');
    if (editorEl.contains(e.target)) caretAtPoint(e.clientX, e.clientY);
    const shiny = e.dataTransfer.getData(SHINY_FILE_MIME);
    void (async () => {
      let files = [...(e.dataTransfer.files || [])];
      if (shiny) {
        try {
          const { path, name } = JSON.parse(shiny);
          const ext = extOf(name);
          files = IMAGE_EXTS.includes(ext) || DOC_EXTS.includes(ext)
            ? [await fileFromHome(path, name, MIME_BY_EXT[ext] || 'application/octet-stream')] : [];
        } catch (_) { files = []; }
      }
      const pictures = files.filter(isImageFile);
      const documents = files.filter((f) => !isImageFile(f) && DOC_EXTS.includes(extOf(f.name)));
      if (pictures.length) await insertPictures(pictures);
      for (const f of documents) await importFile(f);
      if (!pictures.length && !documents.length) {
        toast(t('plugin.word.toast.dropUnsupported', null, 'Word takes pictures and documents'), { type: 'info' });
      }
    })();
  });
  editorEl.addEventListener('paste', (e) => {
    const pics = [...(e.clipboardData?.files || [])].filter(isImageFile);
    if (!pics.length) return;
    e.preventDefault();
    void insertPictures(pics);
  });
}

/* ── Import / export ────────────────────────────────────────── */

async function pickImportFile() {
  const [file] = await pickFiles({ accept: IMPORT_ACCEPT });
  if (file) void importFile(file);
}

async function importFile(file) {
  const form = new FormData();
  form.append('file', file, file.name);
  try {
    const res = await apiFetch('/api/documents/import', { method: 'POST', body: form });
    toast(t('plugin.word.toast.imported', { name: file.name }, `Imported ${file.name}`), { type: 'info' });
    await refreshDocs();
    const found = docs.find((d) => d.id === res?.data?.id);
    if (found) await openDoc(found); else await openNewest();
  } catch (e) {
    toast(e.message || t('plugin.word.toast.importFailed', null, 'Import failed — is this a document Word can read?'), { type: 'error' });
  }
}

async function saveExport(url, name) {
  const blob = await apiFetch(url, { responseType: 'blob' });
  await saveOrDownload(blob, { name, dir: 'Documents', app: 'Word' });
}

async function exportCurrent(format = 'odt') {
  if (!currentDoc) return;
  if (dirty) { try { await persist(); } catch (_) { /* export what is saved */ } }
  setState('saving');
  try {
    await saveExport(`/api/documents/${enc(currentDoc.id)}/export?format=${enc(format)}`, `${currentDoc.title || 'document'}.${format}`);
  } catch (e) {
    toast(e.message || t('plugin.word.toast.exportFailed', null, 'Export failed'), { type: 'error' });
  } finally {
    setState(dirty ? 'dirty' : 'saved');
  }
}

function openExportMenu(anchor) {
  popupMenu(anchor, [
    { icon: 'ui/doc', label: t('plugin.word.menu.exportDocx', null, 'Export as Word (.docx)'), onClick: () => void exportCurrent('docx') },
    { icon: 'ui/file', label: t('plugin.word.menu.exportPdf', null, 'Export as PDF'), onClick: () => void exportCurrent('pdf') },
    { icon: 'ui/upload', label: t('plugin.word.menu.saveToDocuments', null, 'Save to Documents'), hint: '.odt', onClick: () => void exportCurrent('odt') },
    { icon: 'ui/type', label: t('plugin.word.menu.exportMd', null, 'Export as Markdown'), hint: '.md', onClick: () => void exportCurrent('md') },
  ]);
}

/* ── AI ─────────────────────────────────────────────────────── */

function selectedText() {
  const r = editorRange();
  return r && !r.collapsed ? r.toString().trim() : '';
}

/** Ask the assistant about the open document (it is the agent's default target). */
async function askAi(instruction, { needsSelection = false } = {}) {
  if (!currentDoc) return;
  const sel = selectedText();
  if (needsSelection && !sel) {
    toast(t('plugin.word.ai.selectFirst', null, 'Select some text first'), { type: 'info' });
    return;
  }
  if (dirty) await persist();
  const title = currentDoc.title || '';
  let msg = t('plugin.word.ai.prefix', { title }, `In the Word document "${title}":`) + ' ' + instruction;
  if (sel) msg += `\n\n${t('plugin.word.ai.selection', null, 'Selected text')}:\n"${sel.slice(0, 4000)}"`;
  const sent = await askInChat(msg);
  if (!sent) toast(t('plugin.word.ai.busy', null, 'The assistant is busy — try again in a moment'), { type: 'info' });
}

function openAiMenu(anchor) {
  const hasSel = !!selectedText();
  popupMenu(anchor, [
    { icon: 'ui/pencil', label: t('plugin.word.ai.improve', null, 'Improve the writing'), disabled: !hasSel,
      onClick: () => void askAi(t('plugin.word.ai.improvePrompt', null, 'improve the selected text (clarity, flow, tone), keep its meaning, and replace it with doc_edit.'), { needsSelection: true }) },
    { icon: 'ui/check', label: t('plugin.word.ai.fix', null, 'Fix spelling and grammar'),
      onClick: () => void askAi(hasSel
        ? t('plugin.word.ai.fixSelPrompt', null, 'correct spelling and grammar in the selected text with doc_edit, changing nothing else.')
        : t('plugin.word.ai.fixDocPrompt', null, 'read it and correct spelling and grammar with doc_edit, one fix at a time, changing nothing else.')) },
    { icon: 'ui/minus', label: t('plugin.word.ai.shorten', null, 'Make it shorter'), disabled: !hasSel,
      onClick: () => void askAi(t('plugin.word.ai.shortenPrompt', null, 'rewrite the selected text about a third shorter and replace it with doc_edit.'), { needsSelection: true }) },
    { icon: 'ui/forward', label: t('plugin.word.ai.continue', null, 'Continue writing'),
      onClick: () => void askAi(t('plugin.word.ai.continuePrompt', null, 'read it and continue it with doc_append: two or three paragraphs in the same voice, following the outline.')) },
    { icon: 'ui/scientific', label: t('plugin.word.ai.translate', null, 'Translate…'),
      onClick: async () => {
        const lang = await promptDialog(t('plugin.word.ai.translateTo', null, 'Translate into which language?'), '');
        if (!lang?.trim()) return;
        const l = lang.trim();
        void askAi(hasSel
          ? t('plugin.word.ai.translateSelPrompt', { lang: l }, `translate the selected text into ${l} and replace it with doc_edit.`)
          : t('plugin.word.ai.translateDocPrompt', { lang: l }, `translate the whole document into ${l}, write it back with doc_write and set its language with doc_design.`));
      } },
    { separator: true },
    { icon: 'ui/palette', label: t('plugin.word.ai.design', null, 'Make it look professional'),
      onClick: () => void askAi(t('plugin.word.ai.designPrompt', null, 'read it, choose a fitting template with doc_design (cover and contents if it is long, subtitle, author, date, language) and fix the heading structure with doc_edit where it is inconsistent.')) },
    { icon: 'ui/image', label: t('plugin.word.ai.illustrate', null, 'Illustrate with a picture'),
      onClick: () => void askAi(hasSel
        ? t('plugin.word.ai.illustrateSelPrompt', null, 'generate an illustration for the selected passage with generate_image, then place it after that passage with doc_insert_image and a short caption.')
        : t('plugin.word.ai.illustrateDocPrompt', null, 'read it, generate one fitting illustration with generate_image, and place it after the introduction with doc_insert_image and a short caption.')) },
    { icon: 'ui/list', label: t('plugin.word.ai.summary', null, 'Add a summary at the top'),
      onClick: () => void askAi(t('plugin.word.ai.summaryPrompt', null, 'read it and add a short "Summary" section (3–5 sentences) right after the title, with doc_edit.')) },
  ]);
}

/* ── Design dialog ──────────────────────────────────────────── */

async function openDesignDialog() {
  if (!currentDoc) return;
  const d = {
    template: '', paper: 'a4', cover: false, toc: false, header: false, page_numbers: false,
    subtitle: '', author: '', date: '', lang: '', accent: '',
    margin_top: 0, margin_right: 0, margin_bottom: 0, margin_left: 0,
    ...(currentDoc.design || {}),
  };
  const g = geometry(d);

  const tpl = select({
    options: TEMPLATE_IDS.map((v) => ({ value: v, label: t(`plugin.word.design.template.${v}`, null, v[0].toUpperCase() + v.slice(1)) })),
    value: d.template || 'plain',
    onChange: (v) => {
      if (v !== 'plain' && (d.template || 'plain') === 'plain') {
        headerRow.toggle.setChecked(true, { silent: true });
        numbersRow.toggle.setChecked(true, { silent: true });
        d.header = true;
        d.page_numbers = true;
      }
      d.template = v === 'plain' ? '' : v;
    },
  });
  const paper = select({
    options: Object.entries(PAPERS).map(([value, p]) => ({ value, label: p.label })),
    value: (d.paper || 'a4').toLowerCase(),
    onChange: (v) => { d.paper = v; },
  });
  const accent = select({
    options: ACCENTS.map((c) => ({ value: c, label: c ? c.toUpperCase() : t('plugin.word.design.accentTheme', null, 'Theme colour') })),
    value: d.accent || '',
    onChange: (v) => { d.accent = v; },
  });
  const text = (key, labelKey, fallback, placeholder = '') => {
    const el = uiInput({ value: d[key] || '', maxlength: 200, placeholder, onInput: (v) => { d[key] = v; } });
    return field({ label: t(labelKey, null, fallback), control: el });
  };
  const margin = (key, side, value) => {
    const el = uiInput({ type: 'number', value: String(value), min: 5, max: 80, step: 0.5, onInput: (v) => { d[key] = Number(v) || 0; } });
    return field({ label: t(`plugin.word.design.margin.${side}`, null, side), control: el });
  };
  const row = (key, labelKey, fallback, hintKey, hintFallback) => toggleRow({
    label: t(labelKey, null, fallback),
    hint: hintKey ? t(hintKey, null, hintFallback) : undefined,
    checked: !!d[key],
    onChange: (v) => { d[key] = v; },
  });
  const coverRow = row('cover', 'plugin.word.design.cover', 'Cover page', 'plugin.word.design.coverHint', 'Title, subtitle, author and date on their own page');
  const tocRow = row('toc', 'plugin.word.design.toc', 'Table of contents', 'plugin.word.design.tocHint', 'Headings with their page numbers');
  const headerRow = row('header', 'plugin.word.design.header', 'Header', null);
  const numbersRow = row('page_numbers', 'plugin.word.design.pageNumbers', 'Page numbers', null);

  const grid = (...nodes) => { const el = document.createElement('div'); el.className = 'word-design-grid'; el.append(...nodes); return el; };
  const section = (label) => { const el = document.createElement('p'); el.className = 'word-design-eyebrow'; el.textContent = label; return el; };
  const toggles = document.createElement('div');
  toggles.className = 'word-design-toggles';
  toggles.append(coverRow, tocRow, headerRow, numbersRow);

  const dialog = modal({
    title: t('plugin.word.design.title', null, 'Design'),
    wide: true,
    removeOnClose: true,
    body: [
      grid(
        field({ label: t('plugin.word.design.template', null, 'Template'), control: tpl }),
        field({ label: t('plugin.word.design.accent', null, 'Accent'), control: accent }),
        field({ label: t('plugin.word.design.paper', null, 'Paper'), control: paper }),
      ),
      toggles,
      section(t('plugin.word.design.cover', null, 'Cover page')),
      grid(
        text('subtitle', 'plugin.word.design.subtitle', 'Subtitle'),
        text('author', 'plugin.word.design.author', 'Author'),
        text('date', 'plugin.word.design.date', 'Date', new Date().toLocaleDateString(getLocale() || undefined, { year: 'numeric', month: 'long', day: 'numeric' })),
      ),
      section(t('plugin.word.design.margins', null, 'Margins (mm)')),
      grid(
        margin('margin_top', 'top', g.top), margin('margin_bottom', 'bottom', g.bottom),
        margin('margin_left', 'left', g.left), margin('margin_right', 'right', g.right),
      ),
    ],
  });
  const save = async (thenPreview) => {
    for (const k of ['margin_top', 'margin_right', 'margin_bottom', 'margin_left']) {
      const v = Number(d[k]) || 0;
      d[k] = v === 25 ? 0 : Math.min(80, Math.max(5, v));
    }
    dialog.close();
    await commitDesign(d);
    if (thenPreview) setView('preview');
  };
  const actions = document.createElement('div');
  actions.className = 'ui-row word-design-actions';
  actions.append(
    button({ label: t('plugin.word.design.preview', null, 'Save and preview'), variant: 'quiet', onClick: () => void save(true) }),
    button({ label: t('plugin.word.design.save', null, 'Save'), onClick: () => void save(false) }),
  );
  dialog.card.appendChild(actions);
  dialog.open();
}

/* ── Document menu ──────────────────────────────────────────── */

function ensureDocMenu() {
  if (docMenuPopup) return;
  docMenuPopup = menuSurface({ className: 'word-doc-menu', onEscape: closeDocMenu });
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

function renderDocMenuItems() {
  if (!docMenuPopup) return;
  docMenuPopup.innerHTML = '';
  if (!docs.length) {
    const empty = document.createElement('div');
    empty.className = 'word-doc-menu-empty';
    empty.textContent = t('plugin.word.menu.empty', null, 'No documents yet');
    docMenuPopup.appendChild(empty);
  }
  for (const doc of docs) {
    const item = menuItem({ className: 'word-doc-menu-item' });
    if (doc.id === currentDoc?.id) item.classList.add('is-active');
    item.setAttribute('role', 'menuitem');
    const title = document.createElement('span');
    title.className = 'word-doc-menu-title';
    title.textContent = doc.title;
    const time = document.createElement('span');
    time.className = 'word-doc-menu-time';
    time.textContent = formatMenuDate(doc.updated_at);
    const check = document.createElement('span');
    check.className = 'word-doc-menu-check';
    item.append(title, time, check);
    void setIcon(check, 'ui/check', { size: 13 });
    item.addEventListener('click', () => { closeDocMenu(); if (doc.id !== currentDoc?.id) void openDoc(doc); });
    docMenuPopup.appendChild(item);
  }
  appendMenuFooter(docMenuPopup, {
    prefix: 'word-doc',
    close: closeDocMenu,
    items: [
      { icon: 'ui/plus', label: t('plugin.word.menu.newDocument', null, 'New document'), onClick: () => void newDocument() },
      { icon: 'ui/folder-open', label: t('plugin.word.menu.import', null, 'Import…'), onClick: pickImportFile },
      { icon: 'ui/save', label: t('plugin.word.menu.saveNow', null, 'Save now'), onClick: () => void persist() },
      { icon: 'ui/trash', label: t('plugin.word.menu.deleteDocument', null, 'Delete document'), danger: true, onClick: () => void removeCurrent() },
    ],
  });
}

async function toggleDocMenu() {
  if (docMenuOpen) { closeDocMenu(); return; }
  ensureDocMenu();
  await refreshDocs();
  renderDocMenuItems();
  docMenuOpen = true;
  showDocumentMenu(docMenuPopup, docMenuBtn, onDocMenuOutside, onDocMenuKey);
}

/* ── Building the window ────────────────────────────────────── */

function tool(key, iconName, label, onClick) {
  const btn = toolButton(iconName, label, onClick, { className: 'word-tool' });
  // Keep the editor's selection when a tool is pressed.
  btn.addEventListener('mousedown', (e) => e.preventDefault());
  toolBtns[key] = btn;
  return btn;
}

function cluster(className, ...nodes) {
  const el = document.createElement('div');
  el.className = `word-cluster ${className}`;
  el.append(...nodes);
  return el;
}

function buildBar() {
  barEl = toolbar([], { className: 'word-bar', label: t('plugin.word.toolbar.documents', null, 'Documents') });
  barEl.dataset.windowBar = '';

  docMenuBtn = menuButton({ icon: 'ui/doc', label: t('plugin.word.toolbar.documents', null, 'Documents'), onClick: () => void toggleDocMenu(), className: 'word-doc-btn' });

  titleInput = uiInput({ placeholder: t('plugin.word.toolbar.titlePlaceholder', null, 'Untitled'), maxlength: 120, label: t('plugin.word.toolbar.titleLabel', null, 'Document title') });
  titleInput.classList.add('word-title');
  titleInput.addEventListener('input', () => { renderGeneratedParts(); markDirty(); });
  titleInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); editorEl?.focus(); } });
  titleInput.addEventListener('blur', () => { if (dirty) void persist(); });

  undoBtn = tool('undo', 'ui/undo', t('plugin.word.toolbar.undo', null, 'Undo'), () => exec('undo'));
  redoBtn = tool('redo', 'ui/redo', t('plugin.word.toolbar.redo', null, 'Redo'), () => exec('redo'));

  viewSwitch = segmented({
    label: t('plugin.word.view.label', null, 'View'),
    options: [
      { value: 'page', label: t('plugin.word.view.page', null, 'Page') },
      { value: 'preview', label: t('plugin.word.view.preview', null, 'Preview') },
    ],
    value: view,
    onChange: (v) => setView(v),
  });
  viewSwitch.classList.add('word-view-switch');
  // A narrow window folds the switch whole into the More menu (ui/compact.js).
  viewSwitch.dataset.foldable = '';

  const aiBtn = button({ label: t('plugin.word.ai.button', null, 'AI'), icon: 'ui/magic-wand', iconSize: 14, variant: 'ghost', size: 'sm', onClick: () => openAiMenu(aiBtn) });
  aiBtn.classList.add('word-ai');
  aiBtn.setAttribute('aria-haspopup', 'menu');
  aiBtn.addEventListener('mousedown', (e) => e.preventDefault());
  const designBtn = tool('design', 'ui/palette', t('plugin.word.toolbar.design', null, 'Design'), () => void openDesignDialog());
  const exportBtn = tool('export', 'ui/download', t('plugin.word.toolbar.export', null, 'Export'), () => openExportMenu(exportBtn));
  exportBtn.setAttribute('aria-haspopup', 'menu');

  const newBtn = button({ label: t('plugin.word.toolbar.new', null, 'New'), icon: 'ui/plus', iconSize: 14, variant: 'ghost', size: 'sm', onClick: () => void newDocument() });
  newBtn.classList.add('word-new');
  newBtn.title = t('plugin.word.toolbar.newDocument', null, 'New document');

  barEl.append(
    docMenuBtn, titleInput,
    cluster('word-cluster--history', undoBtn, redoBtn),
    viewSwitch,
    cluster('word-cluster--doc', aiBtn, designBtn, exportBtn),
    newBtn,
  );
  return barEl;
}

function buildFormatRow() {
  formatRow = document.createElement('div');
  formatRow.className = 'word-format';
  formatRow.setAttribute('role', 'toolbar');
  formatRow.setAttribute('aria-label', t('plugin.word.toolbar.format', null, 'Formatting'));

  blockSelect = select({
    options: BLOCKS.map((b) => ({ value: b.value, label: t(`plugin.word.block.${b.key}`, null, b.fallback) })),
    value: 'p',
    onChange: (v) => { restoreSelection(); exec('formatBlock', `<${v}>`); },
  });
  blockSelect.classList.add('word-select', 'word-select--block');
  blockSelect.select.setAttribute('aria-label', t('plugin.word.toolbar.style', null, 'Paragraph style'));
  blockSelect.addEventListener('pointerdown', rememberSelection, true);

  fontSelect = select({
    options: [{ value: '', label: t('plugin.word.toolbar.fontTheme', null, 'Theme font') }, ...DOCUMENT_FAMILIES.map((f) => ({ value: f, label: f }))],
    value: '',
    onChange: (v) => styleSelection({ fontFamily: v ? `'${v}'` : '' }),
  });
  fontSelect.classList.add('word-select', 'word-select--font');
  // On a phone the font and size pickers fold whole into More, after the tools.
  fontSelect.dataset.foldable = '';
  fontSelect.select.setAttribute('aria-label', t('plugin.word.toolbar.font', null, 'Font'));
  fontSelect.select.title = t('plugin.word.toolbar.fontHint', null, 'Font — applies to the selection, or the paragraph at the caret');
  fontSelect.addEventListener('pointerdown', rememberSelection, true);
  void listFonts().then((families) => {
    const extra = families.filter((f) => !DOCUMENT_FAMILIES.includes(f));
    fontSelect?.setOptions([
      { value: '', label: t('plugin.word.toolbar.fontTheme', null, 'Theme font') },
      ...DOCUMENT_FAMILIES.map((f) => ({ value: f, label: f })),
      ...extra.map((f) => ({ value: f, label: f })),
    ]);
  }).catch(() => {});

  sizeSelect = select({
    options: [{ value: '', label: t('plugin.word.toolbar.sizeAuto', null, 'Size') }, ...[8, 9, 10, 10.5, 11, 12, 14, 16, 18, 20, 24, 28, 32, 40, 48].map((s) => ({ value: String(s), label: `${s}` }))],
    value: '',
    onChange: (v) => styleSelection({ fontSize: v ? `${v}pt` : '' }),
  });
  sizeSelect.classList.add('word-select', 'word-select--size');
  sizeSelect.dataset.foldable = '';
  sizeSelect.select.setAttribute('aria-label', t('plugin.word.toolbar.size', null, 'Font size'));
  sizeSelect.addEventListener('pointerdown', rememberSelection, true);

  const colorBtn = tool('color', 'ui/text-color', t('plugin.word.toolbar.color', null, 'Text colour'), () => openSwatches(colorBtn, 'text'));
  const markBtn = tool('mark', 'ui/highlight', t('plugin.word.toolbar.highlight', null, 'Highlight'), () => openSwatches(markBtn, 'mark'));

  tableTools = cluster('word-cluster--table',
    tool('trow', 'ui/plus', t('plugin.word.table.addRow', null, 'Add row'), () => tableEdit('row')),
    tool('tcol', 'ui/expand', t('plugin.word.table.addColumn', null, 'Add column'), () => tableEdit('col')),
    tool('tdelrow', 'ui/minus', t('plugin.word.table.deleteRow', null, 'Delete row'), () => tableEdit('delrow')),
    tool('tdelcol', 'ui/close', t('plugin.word.table.deleteColumn', null, 'Delete column'), () => tableEdit('delcol')),
  );
  tableTools.hidden = true;

  const navBtn = tool('nav', 'ui/navigator', t('plugin.word.toolbar.outline', null, 'Outline'), () => toggleNav());

  const spacer = document.createElement('span');
  spacer.className = 'word-spacer';

  formatRow.append(
    blockSelect, fontSelect, sizeSelect,
    cluster('word-cluster--emphasis',
      tool('bold', 'ui/bold', t('plugin.word.toolbar.bold', null, 'Bold'), () => exec('bold')),
      tool('italic', 'ui/italic', t('plugin.word.toolbar.italic', null, 'Italic'), () => exec('italic')),
      tool('underline', 'ui/underline', t('plugin.word.toolbar.underline', null, 'Underline'), () => exec('underline')),
      tool('strike', 'ui/strikethrough', t('plugin.word.toolbar.strike', null, 'Strikethrough'), () => exec('strikeThrough')),
      colorBtn, markBtn),
    cluster('word-cluster--align',
      tool('left', 'ui/align-left', t('plugin.word.toolbar.alignLeft', null, 'Align left'), () => exec('justifyLeft')),
      tool('center', 'ui/align-center', t('plugin.word.toolbar.alignCenter', null, 'Centre'), () => exec('justifyCenter')),
      tool('right', 'ui/align-right', t('plugin.word.toolbar.alignRight', null, 'Align right'), () => exec('justifyRight')),
      tool('justify', 'ui/align-justify', t('plugin.word.toolbar.justify', null, 'Justify'), () => exec('justifyFull'))),
    cluster('word-cluster--lists',
      tool('ul', 'ui/list', t('plugin.word.toolbar.bulletList', null, 'Bullet list'), () => exec('insertUnorderedList')),
      tool('ol', 'ui/list-ordered', t('plugin.word.toolbar.numberedList', null, 'Numbered list'), () => exec('insertOrderedList')),
      tool('outdent', 'ui/outdent', t('plugin.word.toolbar.outdent', null, 'Decrease list level'), () => { if (currentBlock()?.matches('ul,ol')) exec('outdent'); }),
      tool('indent', 'ui/indent', t('plugin.word.toolbar.indent', null, 'Increase list level'), () => { if (currentBlock()?.matches('ul,ol')) exec('indent'); })),
    cluster('word-cluster--insert',
      tool('link', 'ui/link', t('plugin.word.toolbar.link', null, 'Link'), () => void insertLink()),
      tool('image', 'ui/image', t('plugin.word.toolbar.image', null, 'Insert picture'), () => void pickPictures()),
      tool('table', 'ui/grid', t('plugin.word.toolbar.table', null, 'Insert table'), insertTable),
      tool('break', 'ui/page-break', t('plugin.word.toolbar.pageBreak', null, 'Page break (Ctrl+Enter)'), insertPageBreak),
      tool('clear', 'ui/clear-format', t('plugin.word.toolbar.clear', null, 'Clear formatting'), clearFormatting)),
    tableTools,
    spacer,
    navBtn,
  );
  // Narrow windows: the trailing tools fold into the shared "More" menu
  // instead of the row scrolling sideways (ui/compact.js).
  compactToolbar(formatRow);
  return formatRow;
}

function sheet(className) {
  const el = document.createElement('div');
  el.className = `word-sheet ${className}`;
  return el;
}

function buildDesk() {
  deskEl = document.createElement('div');
  deskEl.className = 'word-desk';
  deskEl.addEventListener('wheel', (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    e.preventDefault();
    zoomBy(e.deltaY < 0 ? 1 : -1);
  }, { passive: false });
  deskEl.addEventListener('scroll', () => syncStatus(), { passive: true });

  stageEl = document.createElement('div');
  stageEl.className = 'word-stage';
  stageEl.style.zoom = String(zoom);

  sheetsEl = document.createElement('div');
  sheetsEl.className = 'word-sheets';

  rulerEl = ruler('x');

  // Generated pages: click opens Design.
  coverSheet = sheet('word-sheet--cover word-sheet--generated');
  coverSheet.innerHTML = '<div class="word-cover"><div class="word-cover-title"></div><div class="word-cover-subtitle"></div><div class="word-cover-meta"></div></div>';
  coverSheet.title = t('plugin.word.page.generated', null, 'Generated from the design — click to change it');
  coverSheet.addEventListener('click', () => void openDesignDialog());
  coverSheet.hidden = true;

  tocSheet = sheet('word-sheet--toc word-sheet--generated');
  tocSheet.innerHTML = '<div class="word-toc"><h2 class="word-toc-head"></h2><div class="word-toc-list"></div></div>';
  tocSheet.hidden = true;

  bodySheet = sheet('word-sheet--body');
  vrulerEl = ruler('y');
  headerEl = document.createElement('div');
  headerEl.className = 'word-hf word-hf--header';
  headerEl.innerHTML = '<span class="word-hf-left"></span><span class="word-hf-right"></span>';
  headerEl.hidden = true;
  footerEl = document.createElement('div');
  footerEl.className = 'word-hf word-hf--footer';
  footerEl.innerHTML = '<span class="word-hf-center"></span>';
  footerEl.hidden = true;

  titleBlock = document.createElement('div');
  titleBlock.className = 'word-titleblock';
  titleBlock.innerHTML = '<div class="word-tb-title"></div><div class="word-tb-subtitle"></div><div class="word-tb-meta"></div>';
  titleBlock.title = t('plugin.word.page.generated', null, 'Generated from the design — click to change it');
  titleBlock.addEventListener('click', () => void openDesignDialog());
  titleBlock.hidden = true;

  editorEl = document.createElement('div');
  editorEl.className = 'word-content';
  editorEl.contentEditable = 'true';
  editorEl.spellcheck = true;
  editorEl.dataset.placeholder = t('plugin.word.editor.placeholder', null, 'Write something…');
  editorEl.setAttribute('role', 'textbox');
  editorEl.setAttribute('aria-multiline', 'true');
  editorEl.setAttribute('aria-label', t('plugin.word.editor.label', null, 'Document'));
  editorEl.addEventListener('input', markDirty);
  editorEl.addEventListener('keyup', syncToolbar);
  editorEl.addEventListener('mouseup', syncToolbar);
  editorEl.addEventListener('keydown', onEditorKey);

  guidesEl = document.createElement('div');
  guidesEl.className = 'word-guides';
  guidesEl.setAttribute('aria-hidden', 'true');

  probeEl = document.createElement('div');
  probeEl.className = 'word-probe';
  probeEl.setAttribute('aria-hidden', 'true');

  bodySheet.append(rulerEl, vrulerEl, headerEl, titleBlock, editorEl, footerEl, guidesEl, probeEl);
  sheetsEl.append(coverSheet, tocSheet, bodySheet);
  stageEl.appendChild(sheetsEl);
  deskEl.appendChild(stageEl);

  // Click on the bare paper below the text puts the caret at the end.
  bodySheet.addEventListener('mousedown', (e) => {
    if (e.target !== bodySheet || !currentDoc) return;
    e.preventDefault();
    editorEl.focus();
    const r = document.createRange();
    r.selectNodeContents(editorEl);
    r.collapse(false);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(r);
  });
  return deskEl;
}

function onEditorKey(e) {
  const mod = e.ctrlKey || e.metaKey;
  if (mod && e.key === 'Enter') { e.preventDefault(); insertPageBreak(); }
  else if (mod && (e.key === 's' || e.key === 'S')) { e.preventDefault(); void persist(); }
}

function buildStatus() {
  const status = document.createElement('div');
  status.className = 'word-status';
  statusInfoEl = document.createElement('span');
  statusInfoEl.className = 'word-status-info';
  stateEl = document.createElement('span');
  stateEl.className = 'word-state';
  stateEl.setAttribute('role', 'status');
  const zoomOut = toolButton('ui/zoom-out', t('plugin.word.toolbar.zoomOut', null, 'Zoom out'), () => zoomBy(-1), { className: 'word-status-btn' });
  zoomLabel = controlButton();
  zoomLabel.type = 'button';
  zoomLabel.className = 'word-zoom-label';
  zoomLabel.title = t('plugin.word.toolbar.fitWidth', null, 'Fit to width');
  zoomLabel.textContent = `${Math.round(zoom * 100)}%`;
  zoomLabel.addEventListener('click', fitWidth);
  const zoomIn = toolButton('ui/zoom-in', t('plugin.word.toolbar.zoomIn', null, 'Zoom in'), () => zoomBy(1), { className: 'word-status-btn' });
  const right = cluster('word-cluster--status', stateEl, zoomOut, zoomLabel, zoomIn);
  status.append(statusInfoEl, right);
  return status;
}

/** Create the Word tile element (the plugin's window container). */
export function mountWordTile() {
  if (tileEl) return tileEl;
  installDocumentFonts();
  try { document.execCommand('styleWithCSS', false, true); } catch (_) { /* older engines */ }

  tileEl = document.createElement('section');
  tileEl.className = 'tile word-tile';
  tileEl.dataset.plugin = WORD_PLUGIN;

  tileEl.appendChild(buildBar());
  tileEl.appendChild(buildFormatRow());

  mainEl = document.createElement('div');
  mainEl.className = 'word-main';
  navEl = document.createElement('aside');
  navEl.className = 'word-nav';
  navEl.setAttribute('aria-label', t('plugin.word.toolbar.outline', null, 'Outline'));
  const navHead = document.createElement('p');
  navHead.className = 'word-nav-head';
  navHead.textContent = t('plugin.word.toolbar.outline', null, 'Outline');
  const navList = document.createElement('div');
  navList.className = 'word-nav-list';
  navEl.append(navHead, navList);
  navEl.hidden = !navOpen;

  previewEl = document.createElement('div');
  previewEl.className = 'word-preview';
  previewEl.hidden = true;
  previewEl.addEventListener('scroll', () => syncStatus(), { passive: true });
  previewEl.addEventListener('wheel', (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    e.preventDefault();
    zoomBy(e.deltaY < 0 ? 1 : -1);
  }, { passive: false });

  mainEl.append(navEl, buildDesk(), previewEl);
  tileEl.appendChild(mainEl);
  tileEl.appendChild(buildStatus());
  toolBtns.nav?.classList.toggle('is-active', navOpen);

  wireDrops();
  resizeObs = new ResizeObserver((entries) => {
    if (entries.some((e) => e.target === mainEl) && zoomFit) fitWidth();
    scheduleLayout();
  });
  resizeObs.observe(editorEl);
  resizeObs.observe(mainEl);
  document.addEventListener('selectionchange', onSelectionChange);

  if (zoomFit) requestAnimationFrame(() => fitWidth()); else setZoom(zoom);
  void openNewest();
  return tileEl;
}

function onSelectionChange() {
  if (tileEl?.isConnected && document.activeElement === editorEl) syncToolbar();
}

/** Deactivated mid-edit: flush, drop the window. */
export function unmountWordTile() {
  if (currentDoc && dirty) void persist();
  closeDocMenu();
  closePopup();
  clearPreview();
  resizeObs?.disconnect();
  resizeObs = null;
  document.removeEventListener('selectionchange', onSelectionChange);
  tileEl?.remove();
  tileEl = null; barEl = null; titleInput = null; docMenuBtn = null; viewSwitch = null;
  undoBtn = null; redoBtn = null; formatRow = null; blockSelect = null; fontSelect = null; sizeSelect = null;
  tableTools = null; mainEl = null; navEl = null; deskEl = null; stageEl = null; sheetsEl = null;
  coverSheet = null; tocSheet = null; bodySheet = null; titleBlock = null; headerEl = null; footerEl = null;
  editorEl = null; guidesEl = null; probeEl = null; rulerEl = null; vrulerEl = null; previewEl = null;
  statusInfoEl = null; stateEl = null; zoomLabel = null; toolBtns = {};
  view = 'page';
}

export function getWordTileElement() {
  return tileEl;
}

/* ── AI wiring ──────────────────────────────────────────────── */

function onAgentActions(e) {
  const actions = e.detail || [];
  const docActions = actions.filter((a) => /^doc_/.test(a?.action || ''));
  if (!docActions.length) return;
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: WORD_PLUGIN } }));
  const deleted = docActions.some((a) => a.action === 'doc_delete' && a.result === 'ok');
  const exports = docActions.filter((a) => a.action === 'doc_export' && a.result === 'ok' && a.data?.url);
  const touchedId = docActions.map((a) => a.data?.doc_id).find(Boolean);

  void (async () => {
    for (const ex of exports) {
      try {
        await saveExport(ex.data.url, ex.data.filename || `document.${ex.data.format || 'docx'}`);
      } catch (err) {
        toast(err.message || t('plugin.word.toast.exportFailed', null, 'Export failed'), { type: 'error' });
      }
    }
    if (!tileEl) return;
    await refreshDocs();
    if (deleted) { await openNewest(); return; }
    if (touchedId) {
      if (dirty && currentDoc) { try { await persist(); } catch (_) { /* keep going */ } }
      const found = docs.find((d) => d.id === touchedId);
      if (found) await openDoc(found);
    }
  })();
}

async function importFromFiles(path, name) {
  try {
    if (!getWordTileElement()) mountWordTile();
    const file = await fileFromHome(path, name, MIME_BY_EXT[extOf(name)] || 'application/octet-stream');
    await importFile(file);
  } catch (e) {
    toast(e.message || t('plugin.word.toast.openFileFailed', null, 'Could not open file'), { type: 'error' });
  }
}

let wired = false;
export function wireWordEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  onOpenFromFiles(WORD_PLUGIN, (d) => importFromFiles(d.path, d.name));
  window.addEventListener('beforeunload', (e) => {
    if (currentDoc && dirty) { e.preventDefault(); e.returnValue = ''; }
  });
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19). */
export function wordContextMenu() {
  const hasDoc = !!currentDoc;
  const hasSel = !!selectedText();
  return [
    { type: 'item', label: t('plugin.word.ctx.newDocument', null, 'New document'), icon: 'ui/plus', onClick: () => void newDocument() },
    { type: 'item', label: t('plugin.word.ctx.saveNow', null, 'Save now'), icon: 'ui/save', disabled: !hasDoc, onClick: () => void persist() },
    { type: 'separator' },
    { type: 'item', label: t('plugin.word.ai.improve', null, 'Improve the writing'), icon: 'ui/magic-wand', disabled: !hasSel,
      onClick: () => void askAi(t('plugin.word.ai.improvePrompt', null, 'improve the selected text (clarity, flow, tone), keep its meaning, and replace it with doc_edit.'), { needsSelection: true }) },
    { type: 'item', label: t('plugin.word.ctx.image', null, 'Insert picture'), icon: 'ui/image', disabled: !hasDoc, onClick: () => void pickPictures() },
    { type: 'item', label: t('plugin.word.toolbar.table', null, 'Insert table'), icon: 'ui/grid', disabled: !hasDoc, onClick: insertTable },
    { type: 'separator' },
    { type: 'item', label: t('plugin.word.ctx.design', null, 'Design…'), icon: 'ui/palette', disabled: !hasDoc, onClick: () => void openDesignDialog() },
    { type: 'item', label: t('plugin.word.ctx.exportDocx', null, 'Export as Word (.docx)'), icon: 'ui/download', disabled: !hasDoc, onClick: () => void exportCurrent('docx') },
    { type: 'item', label: t('plugin.word.ctx.exportPdf', null, 'Export as PDF'), icon: 'ui/download', disabled: !hasDoc, onClick: () => void exportCurrent('pdf') },
    { type: 'separator' },
    { type: 'item', label: t('plugin.word.ctx.deleteDocument', null, 'Delete document'), icon: 'ui/trash', danger: true, disabled: !hasDoc, onClick: () => void removeCurrent() },
  ];
}

export default {
  name: 'word',
  icon: 'ui/doc',
  mount: mountWordTile,
  unmount: unmountWordTile,
  getElement: getWordTileElement,
  wireEvents: wireWordEvents,
  contextMenu: wordContextMenu,
};
