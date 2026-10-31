import {
  menuSurface, menuItem, confirmDialog, menuButton, toolbar, input as uiInput, toolButton,
  button, toast, select, setTileGlow,
  setTileGlowFromUrl, glowGradient, fontStackFor, setIcon, largeTitle,
} from '../../ui/index.js';

/**
 * word.js — the Word plugin's window (simple ODT word processor).
 *
 * Editorial surface: the window bar carries the document menu, the
 * formatting tools, one grey "New" pill and a mono save state; the document
 * title sits large above the editor (an editable display title with a mono
 * "EDITED 14:02 · 812 WORDS" meta) and hands over to the bar as the page
 * scrolls. File actions (import / export / save / delete) live in the
 * document menu's footer.
 * Documents are real .odt files stored server-side (core routes) — the
 * editor works on HTML that the server converts to/from ODT.
 *
 * AI wiring: `doc_*` tool outcomes arrive via the `agent:actions` event —
 * the window refreshes its document list and opens what the AI touched.
 */

import { apiFetch } from '../../js/api.js';
import { t, tp } from '../../js/i18n.js';
import { formatMenuDate, formatMenuTime, appendMenuFooter, showDocumentMenu } from '../../js/officeMenu.js';
import { listFonts } from '../../js/fonts.js';
import { saveOrDownload, onOpenFromFiles, fileFromHome, pickFiles } from '../../js/files.js';

export const WORD_PLUGIN = 'word';

let tileEl = null;
let editorEl = null;
let scrollEl = null;
let hero = null;          // largeTitle(): the editable document title + meta
let titleEl = null;       // hero's title node (contentEditable)
let titleText = '';       // canonical title (titleEl may be mid-decode)
let barTitleEl = null;    // the small title the bar shows once scrolled
let docMenuBtn = null;
let stateEl = null;       // mono "SAVED 14:02" / "UNSAVED" in the bar
let fontSelect = null;
let toolbarBtns = {};

let docs = [];
let currentDoc = null;   // { id, title, updated_at }
let dirty = false;
let saveTimer = null;
let saveSeq = 0;

/* Popup (body-level, the tile clips overflow) */
let docMenuPopup = null;
let docMenuOpen = false;

const EXEC_COMMANDS = [
  { key: 'bold', command: 'bold', icon: 'ui/bold', labelKey: 'plugin.word.toolbar.bold' },
  { key: 'italic', command: 'italic', icon: 'ui/italic', labelKey: 'plugin.word.toolbar.italic' },
  { key: 'underline', command: 'underline', icon: 'ui/underline', labelKey: 'plugin.word.toolbar.underline' },
];

/* ── API ────────────────────────────────────────────────────── */

async function listDocs() {
  const res = await apiFetch('/api/documents');
  return res?.data || [];
}

async function fetchDoc(id) {
  const res = await apiFetch(`/api/documents/${encodeURIComponent(id)}`);
  return res?.data;
}

async function createDoc() {
  const res = await apiFetch('/api/documents', {
    method: 'POST',
    body: JSON.stringify({ title: 'Untitled', html: '<p></p>' }),
  });
  return res?.data;
}

async function saveDoc(title, html) {
  if (!currentDoc) return;
  const seq = ++saveSeq;
  await apiFetch(`/api/documents/${encodeURIComponent(currentDoc.id)}`, {
    method: 'PUT',
    body: JSON.stringify({ title, html }),
  });
  return seq;
}

async function deleteDoc(id) {
  await apiFetch(`/api/documents/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/* ── State ──────────────────────────────────────────────────── */

/** The bar's save state plus the large title's meta line. */
function setStatus(mode) {
  if (!stateEl) return;
  stateEl.classList.toggle('is-pending', mode !== 'saved');
  if (mode === 'saving') {
    stateEl.textContent = t('plugin.word.state.saving', null, 'Saving');
  } else if (mode === 'dirty') {
    stateEl.textContent = t('plugin.word.state.unsaved', null, 'Unsaved');
  } else {
    const time = formatMenuTime(currentDoc?.updated_at);
    stateEl.textContent = t('plugin.word.state.saved', { time }, `Saved ${time}`);
    // Counting words reads layout: once per save, not per keystroke.
    syncHeroMeta();
  }
}

function syncHeroMeta() {
  if (!hero) return;
  if (!currentDoc) { hero.setMeta([]); return; }
  const words = wordCount();
  const edited = formatMenuTime(currentDoc.updated_at);
  hero.setMeta([
    t('plugin.word.hero.edited', { time: edited }, `Edited ${edited}`),
    tp('plugin.word.hero.words', words, null, `${words} words`),
  ]);
}

/** Show a title in the large title and the bar (decoded when a doc opens). */
function showTitle(text, { decode = false } = {}) {
  titleText = text;
  hero?.setTitle(text, { decode });
  if (barTitleEl) barTitleEl.textContent = text || t('plugin.word.toolbar.titlePlaceholder', null, 'Untitled');
}

function wordCount() {
  const text = editorEl?.innerText || '';
  return text.trim().split(/\s+/).filter(Boolean).length;
}

function markDirty() {
  dirty = true;
  setStatus('dirty');
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => void persist(), 1200);
}

/* ── Ambient glow ───────────────────────────────────────────── */

/** Warm 28–54° pair seeded from the document title (the no-image case). */
function titleGlow(title) {
  const s = String(title || 'Untitled');
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  const hue = 28 + (Math.abs(h) % 26);        // warm 28–54°
  return glowGradient(`hsl(${hue} 62% 62%)`, `hsl(${(hue + 210) % 360} 45% 26%)`);
}

/** Glow mirrors the document's first embedded image, else the title. */
function updateGlow() {
  if (!currentDoc) { setTileGlow(tileEl, null); return; }
  const src = editorEl?.querySelector('img')?.getAttribute('src');
  if (src) {
    // An embedded picture: mirror it, lightly blurred.
    void setTileGlowFromUrl(tileEl, src, { size: 320, blur: 7 });
  } else {
    setTileGlow(tileEl, titleGlow(currentDoc.title));
  }
}

async function persist() {
  if (!currentDoc || !dirty) return;
  dirty = false;
  setStatus('saving');
  try {
    await saveDoc(titleText.trim(), editorEl.innerHTML);
    currentDoc.title = titleText.trim();
    currentDoc.updated_at = new Date().toISOString();
    setStatus('saved');
  } catch (e) {
    dirty = true;
    setStatus('dirty');
    toast(e.message || t('plugin.word.toast.saveFailed', null, 'Save failed'), { type: 'error' });
  }
}

async function openDoc(doc) {
  window.clearTimeout(saveTimer);
  dirty = false;
  saveSeq++;
  try {
    const full = doc.html != null ? doc : await fetchDoc(doc.id);
    currentDoc = { id: doc.id, title: full.title, updated_at: full.updated_at };
    editorEl.innerHTML = full.html || '<p></p>';
    showTitle(full.title || '', { decode: true });
    if (scrollEl) scrollEl.scrollTop = 0;
    syncEditorFontFromContent();
    setStatus('saved');
    updateGlow();
  } catch (e) {
    toast(e.message || t('plugin.word.toast.openFailed', null, 'Could not open document'), { type: 'error' });
  }
}

async function openNewest() {
  const list = await listDocs();
  docs = list;
  if (list.length) await openDoc(list[0]);
}

async function newDocument() {
  try {
    const created = await createDoc();
    await refreshDocs();
    await openDoc(created);
  } catch (e) {
    toast(e.message || t('plugin.word.toast.createFailed', null, 'Could not create document'), { type: 'error' });
  }
}

async function removeCurrent() {
  if (!currentDoc) return;
  const target = currentDoc;
  if (!await confirmDialog(t('plugin.word.dialog.deleteDoc', { title: currentDoc.title }, `Delete "${currentDoc.title}"?`))) return;
  if (currentDoc !== target || !tileEl) return;
  try {
    await deleteDoc(currentDoc.id);
    currentDoc = null;
    await refreshDocs();
    await openNewest();
    updateGlow();
  } catch (e) {
    toast(e.message || t('plugin.word.toast.deleteFailed', null, 'Could not delete document'), { type: 'error' });
  }
}

async function refreshDocs() {
  try {
    docs = await listDocs();
  } catch (_) { /* keep last list */ }
}

/* ── Doc menu ───────────────────────────────────────────────── */

function ensureDocMenu() {
  if (docMenuPopup) return;
  docMenuPopup = menuSurface({ className: 'word-doc-menu', onEscape: closeDocMenu });
  document.body.appendChild(docMenuPopup);
}

function closeDocMenu() {
  if (!docMenuOpen) return;
  docMenuOpen = false;
  docMenuPopup?.classList.add('hidden');
  if (docMenuBtn) docMenuBtn.setAttribute('aria-expanded', 'false');
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

function renderDocMenuItems() {
  if (!docMenuPopup) return;
  docMenuPopup.innerHTML = '';
  if (!docs.length) {
    const empty = document.createElement('div');
    empty.className = 'word-doc-menu-empty';
    empty.textContent = t('plugin.word.menu.empty', null, 'No documents yet');
    docMenuPopup.appendChild(empty);
  } else {
    docs.forEach((doc) => {
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

      item.addEventListener('click', () => {
        closeDocMenu();
        if (doc.id !== currentDoc?.id) void openDoc(doc);
      });
      docMenuPopup.appendChild(item);
    });
  }

  // Actions footer: the file actions live here, not in the window bar.
  appendMenuFooter(docMenuPopup, {
    prefix: 'word-doc',
    close: closeDocMenu,
    items: [
      { icon: 'ui/plus', label: t('plugin.word.menu.newDocument', null, 'New document'), onClick: () => void newDocument() },
      { icon: 'ui/folder-open', label: t('plugin.word.menu.importOdt', null, 'Import .odt'), onClick: pickOdtFile },
      { icon: 'ui/save', label: t('plugin.word.menu.saveNow', null, 'Save now'), onClick: () => void persist() },
      { icon: 'ui/upload', label: t('plugin.word.menu.saveToDocuments', null, 'Save to Documents'), onClick: () => void exportCurrent() },
      { icon: 'ui/trash', label: t('plugin.word.menu.deleteDocument', null, 'Delete document'), danger: true, onClick: () => void removeCurrent() },
    ],
  });
}

function openDocMenu() {
  ensureDocMenu();
  renderDocMenuItems();
  docMenuOpen = true;
  showDocumentMenu(docMenuPopup, docMenuBtn, onDocMenuOutside, onDocMenuKey);
}

function toggleDocMenu() {
  if (docMenuOpen) closeDocMenu();
  else openDocMenu();
}

/* ── Editor commands ────────────────────────────────────────── */

function exec(command, value = null) {
  if (!editorEl) return;
  editorEl.focus();
  document.execCommand(command, false, value);
  editorEl.dispatchEvent(new Event('input'));
  syncToolbar();
}

function toggleHeading() {
  exec('formatBlock', '<h2>');
}

function toggleList() {
  exec('insertUnorderedList');
}

function syncToolbar() {
  const state = {
    bold: document.queryCommandState('bold'),
    italic: document.queryCommandState('italic'),
    underline: document.queryCommandState('underline'),
    heading: false,
    list: document.queryCommandState('insertUnorderedList'),
  };
  const block = document.queryCommandValue('formatBlock');
  if (typeof block === 'string' && /^h[1-6]$/i.test(block.trim())) state.heading = true;
  for (const [key, btn] of Object.entries(toolbarBtns)) {
    btn?.classList.toggle('is-active', !!state[key]);
  }
  syncFontSelect();
}

/* ── Fonts ──────────────────────────────────────────────────────
 * The picker lists the host's installed fonts (GET /api/fonts). Choosing one
 * with text selected restyles just that run; with no selection it becomes the
 * document's default face. Both survive the .odt round trip: the server's
 * HTML↔ODT codec carries `font-family` as an automatic text style (TF1, …).
 * ─────────────────────────────────────────────────────────────── */

/** The family name on an element, from `data-font` or its inline stack. */
function detectFont(node) {
  if (!node || node.nodeType !== 1) return '';
  const explicit = node.dataset?.font;
  if (explicit) return explicit;
  const stack = node.style?.fontFamily || '';
  return stack.split(',')[0].replace(/["']/g, '').trim();
}

/** The current selection, only when it is inside the editor. */
function editorSelection() {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) return null;
  const range = sel.getRangeAt(0);
  return editorEl?.contains(range.commonAncestorContainer) ? range : null;
}

/** Wrap the selected run in a span carrying the chosen family. */
function applyFontToSelection(range, family) {
  const span = document.createElement('span');
  span.style.fontFamily = fontStackFor(family);
  span.dataset.font = family;
  try {
    range.surroundContents(span);
  } catch (_) {
    // The range crosses element boundaries — lift it into a single span first.
    span.appendChild(range.extractContents());
    range.insertNode(span);
  }
  editorEl.dispatchEvent(new Event('input'));
  syncToolbar();
}

/** Set the document-wide default face (existing text + new typing). */
function applyDocumentFont(family) {
  const stack = fontStackFor(family) || '';
  editorEl.style.fontFamily = stack;
  editorEl.dataset.font = family;
  editorEl.querySelectorAll('p, h1, h2, h3, h4, h5, h6, li, blockquote, pre').forEach((block) => {
    if (stack) {
      block.style.fontFamily = stack;
      block.dataset.font = family;
    } else {
      block.style.removeProperty('font-family');
      delete block.dataset.font;
    }
  });
  editorEl.dispatchEvent(new Event('input'));
  syncToolbar();
}

/** Restore the document default after loading an .odt/.html document. */
function syncEditorFontFromContent() {
  let family = editorEl.dataset.font || '';
  if (!family) {
    const first = editorEl.firstElementChild;
    if (first) family = detectFont(first);
  }
  editorEl.style.fontFamily = family ? (fontStackFor(family) || '') : '';
  editorEl.dataset.font = family;
  syncFontSelect();
}

/** Reflect the face at the caret in the picker. */
function syncFontSelect() {
  if (!fontSelect || !fontSelect.select) return;
  let family = '';
  const range = editorSelection();
  if (range) {
    let node = range.startContainer;
    while (node && node !== editorEl) {
      if (node.nodeType === 1) {
        family = detectFont(node);
        if (family) break;
      }
      node = node.parentNode;
    }
  }
  if (!family) family = editorEl?.dataset.font || '';
  if (family && ![...fontSelect.select.options].some((o) => o.value === family)) {
    fontSelect.setOptions([
      { value: family, label: family },
      ...[...fontSelect.select.options]
        .filter((o) => o.value)
        .map((o) => ({ value: o.value, label: o.textContent })),
    ]);
  }
  fontSelect.select.value = family || fontSelect.select.options[0]?.value || '';
}

function onFontChange(family) {
  if (!family || !editorEl) return;
  const range = editorSelection();
  if (range && !range.collapsed) applyFontToSelection(range, family);
  else applyDocumentFont(family);
}

/* ── Import / export ────────────────────────────────────────── */

async function pickOdtFile() {
  const [file] = await pickFiles({ accept: '.odt,application/vnd.oasis.opendocument.text' });
  if (file) void importOdt(file);
}

async function importOdt(file) {
  const form = new FormData();
  form.append('file', file);
  try {
    await apiFetch('/api/documents/import', { method: 'POST', body: form });
    toast(t('plugin.word.toast.imported', { name: file.name }, `Imported ${file.name}`), { type: 'info' });
    await refreshDocs();
    await openNewest();
  } catch (e) {
    toast(e.message || t('plugin.word.toast.importFailed', null, 'Import failed — is this a valid .odt file?'), { type: 'error' });
  }
}

async function exportCurrent() {
  if (!currentDoc) return;
  try {
    const blob = await apiFetch(
      `/api/documents/${encodeURIComponent(currentDoc.id)}/export`,
      { responseType: 'blob' },
    );
    await saveOrDownload(blob, { name: `${currentDoc.title || 'document'}.odt`, dir: 'Documents', app: 'Word' });
  } catch (e) {
    toast(e.message || t('plugin.word.toast.saveFailed', null, 'Save failed'), { type: 'error' });
  }
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

function toolbarButton(key, iconName, label, onClick) {
  const btn = toolButton(iconName, label, onClick, { className: 'word-tool' });
  toolbarBtns[key] = btn;
  return btn;
}

/** Create the Word tile element (the plugin's window container). */
export function mountWordTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile word-tile';
  tileEl.dataset.plugin = WORD_PLUGIN;

  /* Window bar: doc menu, the scrolled-in title, formatting, New, save state */
  const bar = toolbar([], { className: 'word-bar' });
  bar.dataset.windowBar = '';

  docMenuBtn = menuButton({ icon: 'ui/doc', label: t('plugin.word.toolbar.documents', null, 'Documents'), onClick: toggleDocMenu, className: 'word-doc-btn' });

  barTitleEl = document.createElement('span');
  barTitleEl.className = 'word-bar-title';
  barTitleEl.setAttribute('aria-hidden', 'true');

  const newBtn = button({ label: t('plugin.word.toolbar.new', null, 'New'), icon: 'ui/plus', iconSize: 14, variant: 'ghost', size: 'sm', onClick: () => void newDocument() });
  newBtn.classList.add('word-new');
  newBtn.title = t('plugin.word.toolbar.newDocument', null, 'New document');

  stateEl = document.createElement('span');
  stateEl.className = 'word-state';
  stateEl.setAttribute('role', 'status');

  // Font picker — populated from the host's installed fonts.
  fontSelect = select({
    options: [{ value: '', label: t('plugin.word.toolbar.font', null, 'Font') }],
    onChange: onFontChange,
  });
  fontSelect.classList.add('word-font');
  fontSelect.select.title = t('plugin.word.toolbar.fontHint', null,
    'Font — applies to the selection, or the whole document when nothing is selected');
  fontSelect.select.setAttribute('aria-label', t('plugin.word.toolbar.font', null, 'Font'));
  void listFonts().then((families) => {
    fontSelect?.setOptions(families.map((f) => ({ value: f, label: f })));
    syncFontSelect();
  });

  const tools = document.createElement('div');
  tools.className = 'word-tools';
  tools.append(
    toolbarButton('bold', 'ui/bold', t('plugin.word.toolbar.bold', null, 'Bold'), () => exec('bold')),
    toolbarButton('italic', 'ui/italic', t('plugin.word.toolbar.italic', null, 'Italic'), () => exec('italic')),
    toolbarButton('underline', 'ui/underline', t('plugin.word.toolbar.underline', null, 'Underline'), () => exec('underline')),
    toolbarButton('heading', 'ui/heading', t('plugin.word.toolbar.heading', null, 'Heading'), toggleHeading),
    toolbarButton('list', 'ui/list', t('plugin.word.toolbar.bulletList', null, 'Bullet list'), toggleList),
    fontSelect,
  );

  bar.append(docMenuBtn, barTitleEl, tools, newBtn, stateEl);
  tileEl.appendChild(bar);

  /* Page: the large editable title, then the editor, in one scroller */
  scrollEl = document.createElement('div');
  scrollEl.className = 'word-scroll';

  hero = largeTitle({ scroller: scrollEl });
  hero.el.classList.add('word-hero');
  titleEl = hero.el.querySelector('.ui-large-title-text');
  titleEl.contentEditable = 'plaintext-only';
  titleEl.spellcheck = false;
  titleEl.setAttribute('role', 'textbox');
  titleEl.setAttribute('aria-label', t('plugin.word.toolbar.titleLabel', null, 'Document title'));
  titleEl.dataset.placeholder = t('plugin.word.toolbar.titlePlaceholder', null, 'Untitled');
  titleEl.addEventListener('input', () => {
    let text = titleEl.textContent.replace(/\s+/g, ' ');
    if (text.length > 120) { text = text.slice(0, 120); titleEl.textContent = text; }
    titleText = text;
    if (barTitleEl) barTitleEl.textContent = text.trim() || t('plugin.word.toolbar.titlePlaceholder', null, 'Untitled');
    if (currentDoc) markDirty();
  });
  titleEl.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); editorEl?.focus(); }
  });
  titleEl.addEventListener('blur', () => {
    if (titleEl.textContent !== titleText) titleEl.textContent = titleText;
    if (currentDoc && dirty) void persist();
  });

  /* Editor */
  editorEl = document.createElement('div');
  editorEl.className = 'word-editor';
  editorEl.contentEditable = 'true';
  editorEl.spellcheck = true;
  editorEl.dataset.placeholder = t('plugin.word.editor.placeholder', null, 'Write something…');
  editorEl.setAttribute('role', 'textbox');
  editorEl.setAttribute('aria-multiline', 'true');
  editorEl.addEventListener('input', markDirty);
  editorEl.addEventListener('selectionchange', () => window.setTimeout(syncToolbar, 0));
  editorEl.addEventListener('keyup', syncToolbar);
  editorEl.addEventListener('mouseup', syncToolbar);

  scrollEl.append(hero.el, editorEl);
  tileEl.appendChild(scrollEl);

  document.addEventListener('selectionchange', () => {
    if (tileEl?.isConnected && document.activeElement === editorEl) syncToolbar();
  });

  void openNewest();
  updateGlow();
  return tileEl;
}

/** Deactivated mid-edit: flush, drop the window. */
export function unmountWordTile() {
  if (currentDoc && dirty) void persist();
  closeDocMenu();
  hero?.dispose();
  tileEl?.remove();
  // Drop ALL references so a later mountWordTile() builds a fresh tile
  // instead of returning the detached old one.
  tileEl = null;
  editorEl = null;
  scrollEl = null;
  hero = null;
  titleEl = null;
  titleText = '';
  barTitleEl = null;
  docMenuBtn = null;
  stateEl = null;
  fontSelect = null;
  toolbarBtns = {};
}

/** The tile element (or null when the word window is not mounted). */
export function getWordTileElement() {
  return tileEl;
}

/* ── AI wiring ──────────────────────────────────────────────── */

function onAgentActions(e) {
  const actions = e.detail || [];
  const docActions = actions.filter((a) => /^doc_/.test(a?.action || ''));
  if (!docActions.length) return;

  // Always surface the Word window when the AI touches documents.
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: WORD_PLUGIN } }));

  const deleted = docActions.some((a) => a.action === 'doc_delete' && a.result === 'ok');
  // The action payload carries the touched doc_id (now always present).
  const touchedId = docActions
    .map((a) => a.data?.doc_id)
    .find((id) => !!id);

  void (async () => {
    await refreshDocs();
    if (deleted) {
      await openNewest();
      return;
    }
    if (touchedId) {
      // Flush the user's unsaved edits before switching away.
      if (dirty && currentDoc) {
        try { await persist(); } catch (_) { /* keep going */ }
      }
      const found = docs.find((d) => d.id === touchedId);
      if (found) await openDoc(found);
    }
  })();
}

async function importFromFiles(path, name) {
  try {
    if (!getWordTileElement()) mountWordTile();
    const file = await fileFromHome(path, name, 'application/vnd.oasis.opendocument.text');
    await importOdt(file);
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
    if (currentDoc && dirty) {
      e.preventDefault();
      e.returnValue = '';
    }
  });
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators + window management. */
export function wordContextMenu() {
  const hasDoc = !!currentDoc;
  return [
    { type: 'item', label: t('plugin.word.ctx.newDocument', null, 'New document'), icon: 'ui/plus', onClick: () => void newDocument() },
    { type: 'item', label: t('plugin.word.ctx.saveNow', null, 'Save now'), icon: 'ui/save', disabled: !hasDoc, onClick: () => void persist() },
    { type: 'item', label: t('plugin.word.ctx.saveToDocuments', null, 'Save to Documents'), icon: 'ui/upload', disabled: !hasDoc, onClick: () => void exportCurrent() },
    { type: 'separator' },
    { type: 'item', label: t('plugin.word.ctx.bold', null, 'Bold'), icon: 'ui/bold', disabled: !hasDoc, onClick: () => exec('bold') },
    { type: 'item', label: t('plugin.word.ctx.italic', null, 'Italic'), icon: 'ui/italic', disabled: !hasDoc, onClick: () => exec('italic') },
    { type: 'item', label: t('plugin.word.ctx.heading', null, 'Heading'), icon: 'ui/heading', disabled: !hasDoc, onClick: () => toggleHeading() },
    { type: 'item', label: t('plugin.word.ctx.list', null, 'List'), icon: 'ui/list', disabled: !hasDoc, onClick: () => toggleList() },
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
