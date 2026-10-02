/**
 * word.js — the Word plugin's window (simple ODT word processor).
 *
 * Flat editorial surface: a hairline toolbar (doc menu + title + formatting),
 * a contentEditable editor with editorial typography, and a status line.
 * Documents are real .odt files stored server-side (core routes) — the
 * editor works on HTML that the server converts to/from ODT.
 *
 * AI wiring: `doc_*` tool outcomes arrive via the `agent:actions` event —
 * the window refreshes its document list and opens what the AI touched.
 */

import {
  icon, button, emptyState, toast, select, setTileGlow, setTileGlowFromUrl, glowGradient,
  fontStackFor,
} from '../../ui/index.js';
import { setIcon } from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { listFonts } from '../../js/fonts.js';
import { saveOrDownload, onOpenFromFiles, fileFromHome, pickFiles } from '../../js/files.js';

export const WORD_PLUGIN = 'word';

let tileEl = null;
let editorEl = null;
let titleInput = null;
let docMenuBtn = null;
let statusEl = null;
let saveDot = null;
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
  { key: 'bold', command: 'bold', icon: 'ui/bold', label: 'Bold' },
  { key: 'italic', command: 'italic', icon: 'ui/italic', label: 'Italic' },
  { key: 'underline', command: 'underline', icon: 'ui/underline', label: 'Underline' },
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

function setStatus(mode) {
  if (!statusEl) return;
  const now = new Date();
  const t = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  if (mode === 'saving') {
    statusEl.textContent = 'Saving…';
    saveDot?.classList.add('is-active');
  } else if (mode === 'dirty') {
    statusEl.textContent = 'Unsaved changes';
    saveDot?.classList.add('is-active');
  } else {
    const words = wordCount();
    statusEl.textContent = `Saved ${t} · ${words} ${words === 1 ? 'word' : 'words'}`;
    saveDot?.classList.remove('is-active');
  }
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
    await saveDoc(titleInput.value.trim(), editorEl.innerHTML);
    currentDoc.updated_at = new Date().toISOString();
    setStatus('saved');
  } catch (e) {
    dirty = true;
    setStatus('dirty');
    toast(e.message || 'Save failed', { type: 'error' });
  }
}

async function openDoc(doc) {
  window.clearTimeout(saveTimer);
  dirty = false;
  saveSeq++;
  try {
    const full = doc.html != null ? doc : await fetchDoc(doc.id);
    currentDoc = { id: doc.id, title: full.title, updated_at: full.updated_at };
    titleInput.value = full.title;
    editorEl.innerHTML = full.html || '<p></p>';
    syncEditorFontFromContent();
    setStatus('saved');
    updateGlow();
  } catch (e) {
    toast(e.message || 'Could not open document', { type: 'error' });
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
    toast(e.message || 'Could not create document', { type: 'error' });
  }
}

async function removeCurrent() {
  if (!currentDoc) return;
  if (!window.confirm(`Delete "${currentDoc.title}"?`)) return;
  try {
    await deleteDoc(currentDoc.id);
    currentDoc = null;
    await refreshDocs();
    await openNewest();
    updateGlow();
  } catch (e) {
    toast(e.message || 'Could not delete document', { type: 'error' });
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
  docMenuPopup = document.createElement('div');
  docMenuPopup.className = 'word-doc-menu hidden';
  docMenuPopup.setAttribute('role', 'menu');
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
    empty.textContent = 'No documents yet';
    docMenuPopup.appendChild(empty);
  } else {
    docs.forEach((doc) => {
      const item = document.createElement('button');
      item.type = 'button';
      item.className = 'word-doc-menu-item';
      if (doc.id === currentDoc?.id) item.classList.add('is-active');
      item.setAttribute('role', 'menuitem');

      const title = document.createElement('span');
      title.className = 'word-doc-menu-title';
      title.textContent = doc.title;
      const time = document.createElement('span');
      time.className = 'word-doc-menu-time';
      time.textContent = formatWhen(doc.updated_at);
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

  // Actions footer: new / import / export / delete. The compact window
  // layout hides these from the toolbar — the menu is their home.
  const foot = document.createElement('div');
  foot.className = 'word-doc-menu-foot';
  const footItem = (iconName, label, danger, onClick) => {
    const item = document.createElement('button');
    item.type = 'button';
    item.className = 'word-doc-menu-item';
    if (danger) item.classList.add('word-doc-menu-item--danger');
    item.setAttribute('role', 'menuitem');
    const ic = document.createElement('span');
    ic.className = 'word-doc-menu-foot-icon';
    item.appendChild(ic);
    void setIcon(ic, iconName, { size: 14 });
    const labelEl = document.createElement('span');
    labelEl.className = 'word-doc-menu-title';
    labelEl.textContent = label;
    item.appendChild(labelEl);
    item.addEventListener('click', () => {
      closeDocMenu();
      onClick();
    });
    foot.appendChild(item);
  };
  footItem('ui/plus', 'New document', false, () => void newDocument());
  footItem('ui/download', 'Import .odt', false, pickOdtFile);
  footItem('ui/save', 'Save to Documents', false, () => void exportCurrent());
  footItem('ui/trash', 'Delete document', true, () => void removeCurrent());
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
  const left = Math.max(12, Math.min(r.left, window.innerWidth - 280 - 12));
  docMenuPopup.style.left = `${left}px`;
  docMenuPopup.style.top = `${r.bottom + 8}px`;
  docMenuOpen = true;
  docMenuBtn.setAttribute('aria-expanded', 'true');
  document.addEventListener('pointerdown', onDocMenuOutside, true);
  document.addEventListener('keydown', onDocMenuKey, true);
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
    toast(`Imported ${file.name}`, { type: 'info' });
    await refreshDocs();
    await openNewest();
  } catch (e) {
    toast(e.message || 'Import failed — is this a valid .odt file?', { type: 'error' });
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
    toast(e.message || 'Save failed', { type: 'error' });
  }
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

function toolbarButton(key, iconName, label, onClick) {
  const btn = button({ icon: iconName, variant: 'ghost', onClick });
  btn.classList.add('ui-btn--icon', 'word-tool');
  btn.title = label;
  btn.setAttribute('aria-label', label);
  toolbarBtns[key] = btn;
  return btn;
}

/** Create the Word tile element (the plugin's window container). */
export function mountWordTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile word-tile';
  tileEl.dataset.plugin = WORD_PLUGIN;

  /* Top bar: doc menu + title + save indicator */
  const bar = document.createElement('div');
  bar.className = 'word-bar';
  bar.dataset.windowBar = '';

  docMenuBtn = document.createElement('button');
  docMenuBtn.type = 'button';
  docMenuBtn.className = 'word-doc-btn';
  docMenuBtn.setAttribute('aria-haspopup', 'menu');
  docMenuBtn.setAttribute('aria-expanded', 'false');
  docMenuBtn.title = 'Documents';
  const docIcon = document.createElement('span');
  docIcon.className = 'word-doc-btn-icon';
  docMenuBtn.appendChild(docIcon);
  void setIcon(docIcon, 'ui/doc', { size: 15 });
  const chevron = document.createElement('span');
  chevron.className = 'word-doc-btn-chevron';
  docMenuBtn.appendChild(chevron);
  void setIcon(chevron, 'ui/chevron-down', { size: 12 });
  docMenuBtn.addEventListener('click', toggleDocMenu);

  titleInput = document.createElement('input');
  titleInput.className = 'word-title';
  titleInput.type = 'text';
  titleInput.placeholder = 'Untitled';
  titleInput.maxLength = 120;
  titleInput.autocomplete = 'off';
  titleInput.addEventListener('change', () => {
    if (currentDoc) void persist();
  });

  // File actions — unified order across word/calc/impress: new, import,
  // export, (save), delete.
  const newBtn = button({ icon: 'ui/plus', variant: 'ghost', onClick: () => void newDocument() });
  newBtn.classList.add('ui-btn--icon', 'word-tool', 'word-tool--secondary');
  newBtn.title = 'New document';
  newBtn.setAttribute('aria-label', 'New document');
  const importBtn = button({ icon: 'ui/download', variant: 'ghost', onClick: pickOdtFile });
  importBtn.classList.add('ui-btn--icon', 'word-tool', 'word-tool--secondary');
  importBtn.title = 'Import .odt';
  importBtn.setAttribute('aria-label', 'Import .odt');
  const exportBtn = button({ icon: 'ui/upload', variant: 'ghost', onClick: () => void exportCurrent() });
  exportBtn.classList.add('ui-btn--icon', 'word-tool', 'word-tool--secondary');
  exportBtn.title = 'Save to Documents';
  exportBtn.setAttribute('aria-label', 'Save to Documents');
  const saveBtn = toolbarButton('save', 'ui/save', 'Save now', () => void persist());
  saveBtn.classList.add('word-tool--secondary');
  const delBtn = button({ icon: 'ui/trash', variant: 'ghost', onClick: () => void removeCurrent() });
  delBtn.classList.add('ui-btn--icon', 'word-tool', 'word-tool--danger', 'word-tool--secondary');
  delBtn.title = 'Delete document';
  delBtn.setAttribute('aria-label', 'Delete document');

  saveDot = document.createElement('span');
  saveDot.className = 'word-save-dot';
  saveDot.setAttribute('aria-hidden', 'true');

  // Font picker — populated from the host's installed fonts.
  fontSelect = select({
    options: [{ value: '', label: 'Font' }],
    onChange: onFontChange,
  });
  fontSelect.classList.add('word-font');
  fontSelect.select.title = 'Font — applies to the selection, or the whole document when nothing is selected';
  fontSelect.select.setAttribute('aria-label', 'Font');
  void listFonts().then((families) => {
    fontSelect?.setOptions(families.map((f) => ({ value: f, label: f })));
    syncFontSelect();
  });

  // Single top bar (Studio-style): doc menu + title + every action button.
  bar.append(
    docMenuBtn,
    titleInput,
    toolbarButton('bold', 'ui/bold', 'Bold', () => exec('bold')),
    toolbarButton('italic', 'ui/italic', 'Italic', () => exec('italic')),
    toolbarButton('underline', 'ui/underline', 'Underline', () => exec('underline')),
    toolbarButton('heading', 'ui/heading', 'Heading', toggleHeading),
    toolbarButton('list', 'ui/list', 'Bullet list', toggleList),
    fontSelect,
    newBtn, importBtn, exportBtn, saveBtn, delBtn, saveDot,
  );
  tileEl.appendChild(bar);

  /* Editor */
  editorEl = document.createElement('div');
  editorEl.className = 'word-editor';
  editorEl.contentEditable = 'true';
  editorEl.spellcheck = true;
  editorEl.setAttribute('role', 'textbox');
  editorEl.setAttribute('aria-multiline', 'true');
  editorEl.addEventListener('input', markDirty);
  editorEl.addEventListener('selectionchange', () => window.setTimeout(syncToolbar, 0));
  editorEl.addEventListener('keyup', syncToolbar);
  editorEl.addEventListener('mouseup', syncToolbar);
  tileEl.appendChild(editorEl);

  /* Status line */
  const status = document.createElement('div');
  status.className = 'word-status';
  statusEl = document.createElement('span');
  status.appendChild(statusEl);
  tileEl.appendChild(status);

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
  tileEl?.remove();
  // Drop ALL references so a later mountWordTile() builds a fresh tile
  // instead of returning the detached old one.
  tileEl = null;
  editorEl = null;
  titleInput = null;
  docMenuBtn = null;
  statusEl = null;
  saveDot = null;
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
    toast(e.message || 'Could not open file', { type: 'error' });
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
    { type: 'item', label: 'New document', icon: 'ui/plus', onClick: () => void newDocument() },
    { type: 'item', label: 'Save now', icon: 'ui/save', disabled: !hasDoc, onClick: () => void persist() },
    { type: 'item', label: 'Save to Documents', icon: 'ui/save', disabled: !hasDoc, onClick: () => void exportCurrent() },
    { type: 'separator' },
    { type: 'item', label: 'Bold', icon: 'ui/bold', disabled: !hasDoc, onClick: () => exec('bold') },
    { type: 'item', label: 'Italic', icon: 'ui/italic', disabled: !hasDoc, onClick: () => exec('italic') },
    { type: 'item', label: 'Heading', icon: 'ui/heading', disabled: !hasDoc, onClick: () => toggleHeading() },
    { type: 'item', label: 'List', icon: 'ui/list', disabled: !hasDoc, onClick: () => toggleList() },
    { type: 'separator' },
    { type: 'item', label: 'Delete document', icon: 'ui/trash', danger: true, disabled: !hasDoc, onClick: () => void removeCurrent() },
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
