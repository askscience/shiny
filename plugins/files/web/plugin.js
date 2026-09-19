/**
 * files.js — the Files plugin's window (GNOME Files / Nautilus style).
 *
 * Layout mirrors modern GNOME Files: a full-height Places sidebar, a header
 * with back/forward, a path-bar pill and a grid/list view switcher, a grid of
 * large tiles or a column list, a Sushi-style Space quick-look, and a floating
 * selection status pill. Lazy thumbnails (server PNG / PDF.js / video frame /
 * text mini) and AI wiring (`file_*` outcomes) work as before.
 */

import {
  icon, setIcon, button, emptyState, toast, notify, modal,
  setTileGlow, setTileGlowFromUrl, glowGradient,
} from '/ui/index.js';
import { openContextMenu } from '/js/contextMenu.js';
import { pluginForFile, openWithPlugin } from '/js/files.js';
import { apiFetch } from '/js/api.js';

export const FILES_PLUGIN = 'files';

/* ── state ──────────────────────────────────────────────────── */

let tileEl = null;
let shellEl = null;
let sidebarEl = null;
let placesEl = null;
let headerEl = null;
let pathbarEl = null;
let contentEl = null;
let statusEl = null;
let searchBtn = null;
let viewBtn = null;
let navBack = null;
let navFwd = null;

let cwd = '';
let homeLabel = 'Home';
let allEntries = [];
let entries = [];
let selection = new Set();
let lastAnchor = null;
let view = localStorage.getItem('files.view') === 'list' ? 'list' : 'grid';
let showHidden = localStorage.getItem('files.hidden') === '1';
let sortKey = localStorage.getItem('files.sort') || 'name';
let sortDir = localStorage.getItem('files.sortDir') === 'desc' ? -1 : 1;
let history = [];
let forward = [];
let searchMode = false;
let query = '';
let mounted = false;

const textCache = new Map();   // rel -> Promise<string>
const renderCache = new Map(); // rel -> Promise<render data | null> (office)
const thumbCache = new Map();  // rel -> Promise<string|null> (dataURL)
let observer = null;

const PLACES = [
  { name: 'Home', path: '', icon: 'ui/home' },
  { name: 'Trash', path: '.Trash', icon: 'ui/trash' },
];
const BOOKMARKS = [
  { name: 'Desktop', path: 'Desktop', icon: 'ui/monitor' },
  { name: 'Documents', path: 'Documents', icon: 'ui/doc' },
  { name: 'Downloads', path: 'Downloads', icon: 'ui/download' },
  { name: 'Music', path: 'Music', icon: 'ui/music' },
  { name: 'Pictures', path: 'Pictures', icon: 'ui/image' },
  { name: 'Public', path: 'Public', icon: 'ui/forward' },
  { name: 'Templates', path: 'Templates', icon: 'ui/file' },
  { name: 'Videos', path: 'Videos', icon: 'ui/video' },
];

/* ── API ────────────────────────────────────────────────────── */

const enc = encodeURIComponent;
const rawUrl = (rel) => `/api/files/raw?path=${enc(rel)}`;
const thumbUrl = (rel, size = 256) => `/api/files/thumb?path=${enc(rel)}&size=${size}`;
const downloadUrl = (rel) => `/api/files/download?path=${enc(rel)}`;

async function apiList(path) {
  const res = await apiFetch(`/api/files/list?path=${enc(path)}`);
  return res?.data;
}
async function apiText(path) {
  if (textCache.has(path)) return textCache.get(path);
  const p = apiFetch(`/api/files/text?path=${enc(path)}`)
    .then((r) => r?.data?.content || '')
    .catch(() => '');
  textCache.set(path, p);
  return p;
}
async function apiRead(path) {
  const res = await apiFetch(`/api/files/read?path=${enc(path)}`);
  return res?.data?.content || '';
}
async function apiRender(path) {
  const res = await apiFetch(`/api/files/render?path=${enc(path)}`);
  return res?.data;
}
function apiRenderCached(path) {
  if (renderCache.has(path)) return renderCache.get(path);
  const p = apiRender(path).catch(() => null);
  renderCache.set(path, p);
  return p;
}
const apiMkdir = (path) => apiFetch('/api/files/mkdir', { method: 'POST', body: JSON.stringify({ path }) });
const apiDelete = (path) => apiFetch('/api/files/delete', { method: 'POST', body: JSON.stringify({ path }) });
const apiRename = (from, to) => apiFetch('/api/files/rename', { method: 'POST', body: JSON.stringify({ from, to }) });
const apiRestore = (name) => apiFetch('/api/files/restore', { method: 'POST', body: JSON.stringify({ name }) });
const apiEmptyTrash = () => apiFetch('/api/files/empty-trash', { method: 'POST' });
async function apiSearch(q, path = '') {
  const res = await apiFetch(`/api/files/search?q=${enc(q)}&path=${enc(path)}`);
  return res?.data?.results || [];
}

/* ── helpers ────────────────────────────────────────────────── */

function isTrashPath(p) {
  return p === '.Trash' || (p || '').startsWith('.Trash/');
}
function parentOf(p) {
  const parts = (p || '').split('/').filter(Boolean);
  parts.pop();
  return parts.join('/');
}
function baseName(p) {
  const parts = (p || '').split('/').filter(Boolean);
  return parts.length ? parts[parts.length - 1] : homeLabel;
}
function joinPath(dir, name) {
  return dir ? `${dir}/${name}` : name;
}
function formatSize(n) {
  if (!n) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v < 10 && i > 0 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}
function formatWhen(secs) {
  if (!secs) return '';
  const d = new Date(secs * 1000);
  const now = new Date();
  const day = 86_400_000;
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const t = d.getTime();
  const time = d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  if (t >= startOfToday) return `Today ${time}`;
  if (t >= startOfToday - day) return `Yesterday ${time}`;
  if (t >= startOfToday - 6 * day) return d.toLocaleDateString([], { weekday: 'long' });
  return d.toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' });
}
function entryIcon(entry) {
  if (entry.kind === 'dir') return 'ui/folder';
  if (entry.kind === 'symlink') return 'ui/forward';
  if (entry.is_image) return 'ui/image';
  if (entry.is_pdf) return 'ui/doc';
  if (entry.is_video) return 'ui/video';
  if (entry.is_audio) return 'ui/music';
  if (entry.is_archive) return 'ui/archive';
  return 'ui/file';
}
function visibleEntries() {
  const list = entries.filter((e) => showHidden || !e.hidden);
  return sortEntries(list);
}
function sortEntries(list) {
  const s = [...list];
  s.sort((a, b) => {
    if (a.kind === 'dir' && b.kind !== 'dir') return -1;
    if (a.kind !== 'dir' && b.kind === 'dir') return 1;
    let r = 0;
    if (sortKey === 'size') r = (a.size || 0) - (b.size || 0);
    else if (sortKey === 'modified') r = (a.modified || 0) - (b.modified || 0);
    else r = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
    return r * sortDir;
  });
  return s;
}
function selectionEntries() {
  return allEntries.filter((e) => selection.has(e.path));
}
function updateStatus() {
  if (!statusEl) return;
  const sel = selectionEntries();
  statusEl.classList.toggle('hidden', sel.length === 0);
  if (!sel.length) return;
  if (sel.length === 1) {
    const e = sel[0];
    const what = e.kind === 'dir' ? (e.count != null ? `${e.count} item${e.count === 1 ? '' : 's'}` : 'Folder') : formatSize(e.size);
    statusEl.textContent = `“${e.name}” selected (${what})`;
  } else {
    const bytes = sel.reduce((n, e) => n + (e.kind === 'file' ? (e.size || 0) : 0), 0);
    statusEl.textContent = `${sel.length} items selected (${formatSize(bytes)})`;
  }
}
function cssEscape(s) {
  return String(s).replace(/["\\]/g, '\\$&');
}
function moveSelection(delta) {
  const list = visibleEntries();
  if (!list.length) return;
  const anchor = selection.size ? [...selection][selection.size - 1] : null;
  let idx = list.findIndex((x) => x.path === anchor);
  if (idx < 0) idx = delta > 0 ? -1 : list.length;
  const next = Math.max(0, Math.min(list.length - 1, idx + delta));
  selectEntry(list[next], {});
  const el = contentEl.querySelector(`.files-entry[data-path="${cssEscape(list[next].path)}"]`);
  el?.scrollIntoView({ block: 'nearest' });
}

/* ── thumbnails ─────────────────────────────────────────────── */

let slots = 0;
const waiters = [];
async function withSlot(fn) {
  if (slots >= 2) await new Promise((r) => waiters.push(r));
  slots++;
  try { return await fn(); } finally { slots--; waiters.shift()?.(); }
}
function makeImg(src) {
  const img = document.createElement('img');
  img.className = 'files-grid-img';
  img.alt = '';
  img.src = src;
  return img;
}
function thumbIcon(host, entry) {
  host.innerHTML = '';
  host.classList.remove('files-thumb--mini');
  host.classList.add('files-thumb--icon');
  const ic = icon(entry.kind === 'dir' ? 'ui/folder' : entryIcon(entry), { size: entry.kind === 'dir' ? 56 : 40 });
  host.appendChild(ic);
}
function imageThumb(host, entry) {
  const img = document.createElement('img');
  img.className = 'files-grid-img';
  img.alt = '';
  img.decoding = 'async';
  img.loading = 'lazy';
  img.src = thumbUrl(entry.path, 320);
  img.addEventListener('error', () => {
    if (img.dataset.fallback) { thumbIcon(host, entry); return; }
    img.dataset.fallback = '1';
    img.src = rawUrl(entry.path);
  });
  host.innerHTML = '';
  host.classList.remove('files-thumb--icon');
  host.appendChild(img);
}
async function cachedThumb(host, entry, producer) {
  if (thumbCache.has(entry.path)) {
    const url = await thumbCache.get(entry.path);
    if (!host.isConnected) return;
    url ? (host.innerHTML = '', host.appendChild(makeImg(url)), host.classList.remove('files-thumb--icon')) : thumbIcon(host, entry);
    return;
  }
  const p = withSlot(producer).catch(() => null);
  thumbCache.set(entry.path, p);
  const url = await p;
  if (!host.isConnected) return;
  if (url) { host.innerHTML = ''; host.classList.remove('files-thumb--icon'); host.appendChild(makeImg(url)); }
  else thumbIcon(host, entry);
}
function grabVideoFrame(rel) {
  return new Promise((resolve) => {
    const video = document.createElement('video');
    video.muted = true;
    video.playsInline = true;
    video.preload = 'metadata';
    video.src = rawUrl(rel);
    const done = (val) => { video.removeAttribute('src'); video.load?.(); resolve(val); };
    const timer = setTimeout(() => done(null), 8000);
    video.addEventListener('loadeddata', () => {
      try { video.currentTime = Math.min(1, (video.duration || 2) * 0.1); } catch (_) { clearTimeout(timer); done(null); }
    });
    video.addEventListener('seeked', () => {
      try {
        const w = 320;
        const h = Math.max(1, Math.round((video.videoHeight / video.videoWidth) * w));
        const canvas = document.createElement('canvas');
        canvas.width = w; canvas.height = h;
        canvas.getContext('2d').drawImage(video, 0, 0, w, h);
        clearTimeout(timer);
        done(canvas.toDataURL('image/jpeg', 0.72));
      } catch (_) { clearTimeout(timer); done(null); }
    });
    video.addEventListener('error', () => { clearTimeout(timer); done(null); });
  });
}
async function textThumb(host, entry) {
  const text = await apiText(entry.path);
  if (!host.isConnected) return;
  host.innerHTML = '';
  host.classList.remove('files-thumb--icon');
  host.classList.add('files-thumb--mini');
  const pre = document.createElement('pre');
  pre.className = 'files-mini';
  pre.textContent = (text || '').split('\n').slice(0, 8).join('\n') || ' ';
  host.appendChild(pre);
}
/** Office files get a real content mini (rendered via /api/files/render), not
 *  an icon: a tiny typeset page, a small table, or the first slide. */
async function officeThumb(host, entry) {
  host.innerHTML = '';
  host.classList.remove('files-thumb--doc');
  host.classList.add('files-thumb--mini', 'files-thumb--office');
  const page = document.createElement('div');
  page.className = 'files-mini-page';
  host.appendChild(page);
  const data = await apiRenderCached(entry.path);
  if (!host.isConnected) return;
  if (!data) { officeBadge(host, entry); return; }
  try {
    renderOfficeMini(page, data);
    if (!page.childNodes.length) officeBadge(host, entry);
  } catch (_) {
    officeBadge(host, entry);
  }
}

/** Fallback when a document can't be rendered: an accent document badge. */
function officeBadge(host, entry) {
  host.innerHTML = '';
  host.classList.remove('files-thumb--mini', 'files-thumb--office');
  host.classList.add('files-thumb--doc');
  const which = entry.ext === 'ods' ? 'ui/calc' : entry.ext === 'odp' ? 'ui/present' : 'ui/doc';
  const badge = document.createElement('span');
  badge.className = 'files-doc-badge';
  badge.textContent = (entry.ext || '').toUpperCase();
  host.append(icon(which, { size: 38 }), badge);
}

function renderOfficeMini(page, data) {
  page.innerHTML = '';
  if (data.format === 'doc') {
    page.classList.add('is-doc');
    page.innerHTML = data.html || '';
    sanitizeDocHtml(page);
    return;
  }
  if (data.format === 'sheet') {
    const grid = (data.grid || []).slice(0, 5).map((r) => (r || []).slice(0, 5));
    const table = document.createElement('table');
    table.className = 'files-mini-sheet';
    grid.forEach((row, r) => {
      const tr = document.createElement('tr');
      for (const v of row) {
        const cell = document.createElement(r === 0 ? 'th' : 'td');
        cell.textContent = v == null ? '' : String(v);
        tr.appendChild(cell);
      }
      table.appendChild(tr);
    });
    page.appendChild(table);
    return;
  }
  if (data.format === 'slides') {
    const s = (data.slides || [])[0];
    if (!s) return;
    page.classList.add('is-slide');
    if (s.title) { const t = document.createElement('div'); t.className = 'files-mini-slide-title'; t.textContent = s.title; page.appendChild(t); }
    if (s.subtitle) { const t = document.createElement('div'); t.className = 'files-mini-slide-sub'; t.textContent = s.subtitle; page.appendChild(t); }
    for (const b of (s.bullets || []).slice(0, 4)) {
      const d = document.createElement('div'); d.className = 'files-mini-slide-bullet'; d.textContent = `• ${b}`; page.appendChild(d);
    }
    if (!s.title && !(s.bullets || []).length && s.body) {
      const d = document.createElement('div'); d.className = 'files-mini-slide-bullet'; d.textContent = s.body.slice(0, 160); page.appendChild(d);
    }
  }
}

function hydrateThumb(host, entry) {
  if (entry.kind === 'dir' || entry.kind === 'symlink') { thumbIcon(host, entry); return; }
  if (entry.is_image) { imageThumb(host, entry); return; }
  if (entry.is_video) { void cachedThumb(host, entry, () => grabVideoFrame(entry.path)); return; }
  if (entry.is_pdf) { void cachedThumb(host, entry, () => renderPdfPage(entry.path, 1, 260).then((c) => (c ? c.toDataURL('image/jpeg', 0.72) : null))); return; }
  if (entry.is_office) { void officeThumb(host, entry); return; }
  if (entry.is_text) { void textThumb(host, entry); return; }
  thumbIcon(host, entry);
}

/* ── PDF.js ─────────────────────────────────────────────────── */

let pdfjsPromise = null;
function loadPdfJs() {
  if (!pdfjsPromise) {
    pdfjsPromise = import('/vendor/pdfjs/pdf.min.mjs').then((mod) => {
      mod.GlobalWorkerOptions.workerSrc = '/vendor/pdfjs/pdf.worker.min.mjs';
      return mod;
    }).catch(() => null);
  }
  return pdfjsPromise;
}
async function openPdfDoc(rel) {
  const pdfjs = await loadPdfJs();
  if (!pdfjs) return null;
  return pdfjs.getDocument({
    url: rawUrl(rel),
    cMapUrl: '/vendor/pdfjs/cmaps/',
    cMapPacked: true,
    standardFontDataUrl: '/vendor/pdfjs/standard_fonts/',
  }).promise;
}
async function renderPdfPage(rel, pageNum, targetWidth) {
  const doc = await openPdfDoc(rel);
  if (!doc) return null;
  const page = await doc.getPage(pageNum);
  const base = page.getViewport({ scale: 1 });
  const scale = targetWidth ? targetWidth / base.width : 1;
  const viewport = page.getViewport({ scale });
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.ceil(viewport.width));
  canvas.height = Math.max(1, Math.ceil(viewport.height));
  await page.render({ canvasContext: canvas.getContext('2d'), viewport }).promise;
  return canvas;
}

/* ── sidebar ────────────────────────────────────────────────── */

function buildSidebar() {
  sidebarEl = document.createElement('aside');
  sidebarEl.className = 'files-sidebar';

  const head = document.createElement('div');
  head.className = 'files-sidebar-head';
  searchBtn = iconBtn('ui/search', 'Search', toggleSearch);
  const title = document.createElement('span');
  title.className = 'files-app-title';
  title.textContent = 'Files';
  const menuBtn = iconBtn('ui/list', 'Menu', (e) => openAppMenu(e.currentTarget));
  head.append(searchBtn, title, menuBtn);
  sidebarEl.appendChild(head);

  placesEl = document.createElement('nav');
  placesEl.className = 'files-places';
  renderPlaces();
  sidebarEl.appendChild(placesEl);
}

function renderPlaces() {
  if (!placesEl) return;
  placesEl.innerHTML = '';
  const addPlace = (place) => {
    const active = place.path === '' ? cwd === '' : cwd === place.path || cwd.startsWith(`${place.path}/`);
    const el = document.createElement('button');
    el.type = 'button';
    el.className = 'files-place' + (active ? ' is-active' : '');
    el.dataset.path = place.path;
    const ic = document.createElement('span');
    ic.className = 'files-place-icon';
    void setIcon(ic, place.icon, { size: 16 });
    const label = document.createElement('span');
    label.className = 'files-place-label';
    label.textContent = place.name;
    el.append(ic, label);
    el.addEventListener('click', () => navigate(place.path));
    placesEl.appendChild(el);
  };
  PLACES.forEach(addPlace);
  const sep = document.createElement('div');
  sep.className = 'files-place-sep';
  placesEl.appendChild(sep);
  BOOKMARKS.forEach(addPlace);
}

/* ── header ─────────────────────────────────────────────────── */

function iconBtn(iconName, label, onClick) {
  const btn = button({ icon: iconName, variant: 'ghost', onClick });
  btn.classList.add('ui-btn--icon', 'files-icon-btn');
  btn.title = label;
  btn.setAttribute('aria-label', label);
  return btn;
}

function buildHeader() {
  headerEl = document.createElement('div');
  headerEl.className = 'files-header';

  navBack = iconBtn('ui/chevron-left', 'Back', goBack);
  navFwd = iconBtn('ui/chevron-right', 'Forward', goForward);

  pathbarEl = document.createElement('div');
  pathbarEl.className = 'files-pathbar';

  const actions = document.createElement('div');
  actions.className = 'files-header-actions';

  viewBtn = iconBtn(view === 'grid' ? 'ui/grid' : 'ui/list', 'Toggle view', toggleView);
  const viewChevron = iconBtn('ui/chevron-down', 'View options', (e) => openViewMenu(e.currentTarget));
  viewChevron.classList.add('files-view-chevron');
  const split = document.createElement('div');
  split.className = 'files-view-split';
  split.append(viewBtn, viewChevron);
  actions.append(split);

  headerEl.append(navBack, navFwd, pathbarEl, actions);
}

function renderHeader() {
  if (!pathbarEl) return;
  navBack.disabled = history.length === 0;
  navFwd.disabled = forward.length === 0;
  navBack.classList.toggle('is-disabled', history.length === 0);
  navFwd.classList.toggle('is-disabled', forward.length === 0);
  void setIcon(viewBtn, view === 'grid' ? 'ui/grid' : 'ui/list', { size: 16 });

  pathbarEl.innerHTML = '';
  if (searchMode) {
    const wrap = document.createElement('div');
    wrap.className = 'files-search-entry';
    wrap.appendChild(icon('ui/search', { size: 15 }));
    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'files-search-input';
    input.placeholder = 'Search…';
    input.autocomplete = 'off';
    input.value = query;
    input.addEventListener('input', () => {
      window.clearTimeout(input.__t);
      input.__t = window.setTimeout(() => void runSearch(input.value), 250);
    });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') { e.preventDefault(); exitSearch(); }
    });
    const clear = iconBtn('ui/close', 'Clear', () => { input.value = ''; input.focus(); void runSearch(''); });
    clear.classList.add('files-search-clear');
    wrap.append(input, clear);
    pathbarEl.appendChild(wrap);
    requestAnimationFrame(() => input.focus());
    return;
  }

  const root = document.createElement('button');
  root.type = 'button';
  root.className = 'files-path-crumb' + (cwd === '' ? ' is-current' : '');
  root.appendChild(icon('ui/home', { size: 14 }));
  const rootLabel = document.createElement('span');
  rootLabel.textContent = homeLabel;
  root.appendChild(rootLabel);
  if (cwd !== '') root.addEventListener('click', () => navigate(''));
  pathbarEl.appendChild(root);

  const segs = cwd.split('/').filter(Boolean);
  let acc = '';
  segs.forEach((seg, i) => {
    acc = acc ? `${acc}/${seg}` : seg;
    const sep = document.createElement('span');
    sep.className = 'files-path-sep';
    sep.textContent = '/';
    pathbarEl.appendChild(sep);
    const last = i === segs.length - 1;
    const el = document.createElement(last ? 'span' : 'button');
    el.className = 'files-path-crumb' + (last ? ' is-current' : '');
    el.textContent = seg;
    if (!last) {
      el.type = 'button';
      const target = acc;
      el.addEventListener('click', () => navigate(target));
    }
    pathbarEl.appendChild(el);
  });

  const menu = iconBtn('ui/list', 'Path actions', (e) => openPathMenu(e.currentTarget));
  menu.classList.add('files-path-menu');
  pathbarEl.appendChild(menu);
}

/* ── content ────────────────────────────────────────────────── */

function render() {
  if (!contentEl || !mounted) return;
  contentEl.innerHTML = '';
  observer?.disconnect();
  observer = null;
  updateStatus();

  const list = visibleEntries();
  if (!list.length) {
    contentEl.appendChild(emptyState({
      icon: query ? 'ui/search' : 'ui/folder-open',
      title: query ? 'No results found' : (isTrashPath(cwd) ? 'Trash is empty' : 'Folder is empty'),
      body: query ? `No files match “${query}”.` : 'Drag files here or use the menu to upload.',
    }));
    return;
  }

  observer = new IntersectionObserver((obs) => {
    for (const o of obs) {
      if (!o.isIntersecting) continue;
      observer.unobserve(o.target);
      const entry = o.target.__entry;
      const host = o.target.querySelector('.files-thumb');
      if (entry && host) hydrateThumb(host, entry);
    }
  }, { root: contentEl, rootMargin: '320px' });

  if (view === 'grid') {
    const grid = document.createElement('div');
    grid.className = 'files-grid';
    for (const entry of list) grid.appendChild(gridEntry(entry));
    contentEl.appendChild(grid);
  } else {
    contentEl.appendChild(columnHeader());
    const rows = document.createElement('div');
    rows.className = 'files-rows';
    for (const entry of list) rows.appendChild(rowEntry(entry));
    contentEl.appendChild(rows);
  }
  syncSelectionDom();
}

function columnHeader() {
  const head = document.createElement('div');
  head.className = 'files-col-head';
  const mk = (key, label, cls) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = `files-col ${cls}`;
    const span = document.createElement('span');
    span.textContent = label;
    b.appendChild(span);
    if (sortKey === key) {
      b.classList.add('is-sorted');
      const caret = document.createElement('span');
      caret.className = 'files-col-caret';
      void setIcon(caret, sortDir > 0 ? 'ui/chevron-up' : 'ui/chevron-down', { size: 11 });
      b.appendChild(caret);
    }
    b.addEventListener('click', () => {
      if (sortKey === key) sortDir = -sortDir;
      else { sortKey = key; sortDir = 1; }
      localStorage.setItem('files.sort', sortKey);
      localStorage.setItem('files.sortDir', sortDir < 0 ? 'desc' : 'asc');
      render();
    });
    return b;
  };
  head.append(mk('name', 'Name', 'files-col-name'), mk('size', 'Size', 'files-col-size'), mk('modified', 'Modified', 'files-col-when'));
  return head;
}

function baseEntry(entry, cls) {
  const el = document.createElement('div');
  el.className = `files-entry ${cls}`;
  el.dataset.path = entry.path;
  el.tabIndex = -1;
  el.__entry = entry;
  el.title = entry.name;

  el.addEventListener('click', (e) => {
    e.stopPropagation();
    selectEntry(entry, e);
  });
  el.addEventListener('dblclick', () => openEntry(entry));
  el.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); openEntry(entry); }
    if (e.key === ' ') { e.preventDefault(); void openPreview(entry); }
  });
  return el;
}

function gridEntry(entry) {
  const el = baseEntry(entry, 'files-entry--grid');
  const thumb = document.createElement('div');
  thumb.className = 'files-thumb' + (entry.kind === 'dir' ? ' files-thumb--folder' : '');
  const name = document.createElement('div');
  name.className = 'files-grid-name';
  name.textContent = entry.name;
  el.append(thumb, name);
  observer.observe(el);
  return el;
}

function rowEntry(entry) {
  const el = baseEntry(entry, 'files-entry--row');
  const ic = document.createElement('span');
  ic.className = 'files-row-icon';
  void setIcon(ic, entryIcon(entry), { size: 16 });
  if (entry.kind === 'dir') ic.classList.add('is-folder');
  const name = document.createElement('div');
  name.className = 'files-row-name';
  name.textContent = entry.name;
  const size = document.createElement('div');
  size.className = 'files-row-size';
  size.textContent = entry.kind === 'dir'
    ? (entry.count != null ? `${entry.count} item${entry.count === 1 ? '' : 's'}` : '—')
    : formatSize(entry.size);
  const when = document.createElement('div');
  when.className = 'files-row-when';
  when.textContent = formatWhen(entry.modified);
  el.append(ic, name, size, when);
  return el;
}

function syncSelectionDom() {
  if (!contentEl) return;
  for (const el of contentEl.querySelectorAll('.files-entry')) {
    el.classList.toggle('is-selected', selection.has(el.dataset.path));
  }
}

/* ── navigation ─────────────────────────────────────────────── */

async function navigate(rel, { push = true, clearForward = true } = {}) {
  if (searchMode) exitSearch(true);
  if (push && rel !== cwd) {
    history.push(cwd);
    if (clearForward) forward = [];
  }
  cwd = rel;
  query = '';
  selection.clear();
  await load();
}

async function load() {
  try {
    const data = await apiList(cwd);
    if (!mounted) return;
    homeLabel = 'Home';
    allEntries = data?.entries || [];
    entries = allEntries;
    updateGlow();
  } catch (e) {
    if (!mounted) return;
    allEntries = []; entries = [];
    toast(e.message || 'Could not open folder', { type: 'error' });
  }
  renderPlaces();
  renderHeader();
  render();
}

async function refresh() {
  textCache.clear();
  renderCache.clear();
  await load();
}
function goUp() {
  if (!cwd) return;
  void navigate(parentOf(cwd));
}
function goBack() {
  if (!history.length) return;
  forward.push(cwd);
  const prev = history.pop();
  void navigate(prev, { push: false, clearForward: false });
}
function goForward() {
  if (!forward.length) return;
  history.push(cwd);
  const next = forward.pop();
  void navigate(next, { push: false, clearForward: false });
}

function selectEntry(entry, e) {
  const multi = e && (e.ctrlKey || e.metaKey);
  const range = e && e.shiftKey && lastAnchor;
  if (range) {
    const list = visibleEntries();
    const a = list.findIndex((x) => x.path === lastAnchor);
    const b = list.findIndex((x) => x.path === entry.path);
    if (a >= 0 && b >= 0) {
      selection.clear();
      for (let i = Math.min(a, b); i <= Math.max(a, b); i++) selection.add(list[i].path);
    }
  } else if (multi) {
    if (selection.has(entry.path)) selection.delete(entry.path);
    else selection.add(entry.path);
    lastAnchor = entry.path;
  } else {
    selection.clear();
    selection.add(entry.path);
    lastAnchor = entry.path;
  }
  syncSelectionDom();
  updateStatus();
}

function openEntry(entry) {
  if (entry.kind === 'dir') { void navigate(entry.path); return; }
  // Desktop behaviour: double-click opens the file in the app that owns it.
  // If that app isn't installed, fall back to the quick-look preview.
  const plugin = pluginForFile(entry.name);
  if (plugin) {
    void openWithPlugin({ plugin, path: entry.path, name: entry.name }).then((ok) => {
      if (!ok) void openPreview(entry);
    });
    return;
  }
  void openPreview(entry);
}

function downloadEntry(entry) {
  const a = document.createElement('a');
  a.href = downloadUrl(entry.path);
  a.download = entry.name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  notify({ app: 'Files', title: 'Downloading', body: entry.name, icon: 'ui/download', urgency: 'low' });
}

/* ── search ─────────────────────────────────────────────────── */

function toggleSearch() {
  if (searchMode) exitSearch();
  else { searchMode = true; renderHeader(); }
}
function exitSearch(quiet = false) {
  searchMode = false;
  query = '';
  if (!quiet) { entries = allEntries; renderHeader(); render(); }
}
async function runSearch(q) {
  query = q.trim();
  if (query.length < 2) {
    if (!query) { entries = allEntries; render(); }
    return;
  }
  try {
    entries = await apiSearch(query, '');
    if (!mounted) return;
    render();
  } catch (e) {
    toast(e.message || 'Search failed', { type: 'error' });
  }
}

/* ── view & menus ───────────────────────────────────────────── */

function setView(next) {
  view = next;
  localStorage.setItem('files.view', view);
  renderHeader();
  render();
}
function toggleView() {
  setView(view === 'grid' ? 'list' : 'grid');
}
function toggleHidden() {
  showHidden = !showHidden;
  localStorage.setItem('files.hidden', showHidden ? '1' : '0');
  render();
}

function openAppMenu(anchor) {
  const r = anchor.getBoundingClientRect();
  const trashed = isTrashPath(cwd);
  openContextMenu([
    { type: 'item', label: 'New Folder', icon: 'ui/folder-plus', onClick: () => void newFolder() },
    { type: 'item', label: 'Upload Files…', icon: 'ui/upload', onClick: pickUpload },
    { type: 'separator' },
    { type: 'item', label: 'Show Hidden Files', icon: 'ui/eye', checked: showHidden, onClick: toggleHidden },
    { type: 'item', label: 'View', icon: view === 'grid' ? 'ui/grid' : 'ui/list', onClick: toggleView },
    { type: 'separator' },
    { type: 'item', label: 'Empty Trash', icon: 'ui/trash', danger: true, disabled: !trashed, onClick: () => void emptyTrash() },
  ], r.left, r.bottom + 6);
}

function openViewMenu(anchor) {
  const r = anchor.getBoundingClientRect();
  openContextMenu([
    { type: 'item', label: 'Grid', icon: 'ui/grid', checked: view === 'grid', onClick: () => setView('grid') },
    { type: 'item', label: 'List', icon: 'ui/list', checked: view === 'list', onClick: () => setView('list') },
    { type: 'separator' },
    { type: 'item', label: 'Show Hidden Files', icon: 'ui/eye', checked: showHidden, onClick: toggleHidden },
  ], r.left, r.bottom + 6);
}

function openPathMenu(anchor) {
  const r = anchor.getBoundingClientRect();
  openContextMenu([
    { type: 'item', label: 'New Folder', icon: 'ui/folder-plus', onClick: () => void newFolder() },
    { type: 'item', label: 'Upload Files…', icon: 'ui/upload', onClick: pickUpload },
    { type: 'separator' },
    { type: 'item', label: 'Copy Path', icon: 'ui/copy', onClick: () => copyPath() },
    { type: 'item', label: 'Show Hidden Files', icon: 'ui/eye', checked: showHidden, onClick: toggleHidden },
  ], r.left, r.bottom + 6);
}

function copyPath() {
  const text = cwd || 'Home';
  const p = navigator.clipboard?.writeText(text);
  if (p?.then) p.then(() => toast('Path copied'), () => toast('Could not copy path', { type: 'error' }));
  else toast('Clipboard unavailable', { type: 'error' });
}

/* ── mutations ──────────────────────────────────────────────── */

async function newFolder() {
  const name = window.prompt('New folder name', 'New folder');
  if (!name) return;
  try {
    await apiMkdir(joinPath(cwd, name));
    await refresh();
    toast(`Created “${name}”`);
  } catch (e) {
    toast(e.message || 'Could not create folder', { type: 'error' });
  }
}
function pickUpload() {
  const input = document.createElement('input');
  input.type = 'file';
  input.multiple = true;
  input.addEventListener('change', () => {
    const files = [...(input.files || [])];
    if (files.length) void uploadFiles(files);
  });
  input.click();
}
async function uploadFiles(files) {
  let ok = 0;
  for (const file of files) {
    try {
      await apiFetch(`/api/files/upload?path=${enc(cwd)}&name=${enc(file.name)}`, { method: 'POST', body: file });
      ok++;
    } catch (_) {
      toast(`Upload failed: ${file.name}`, { type: 'error' });
    }
  }
  if (ok) {
    notify({ app: 'Files', title: 'Upload complete', body: `${ok} file${ok === 1 ? '' : 's'} added`, icon: 'ui/upload', urgency: 'low' });
    await refresh();
  }
}
async function renameEntry(entry) {
  const name = window.prompt('Rename to', entry.name);
  if (!name || name === entry.name) return;
  try {
    await apiRename(entry.path, joinPath(parentOf(entry.path), name));
    await refresh();
    toast(`Renamed to “${name}”`);
  } catch (e) {
    toast(e.message || 'Could not rename', { type: 'error' });
  }
}
async function removeEntries(list, permanent) {
  if (!list.length) return;
  const label = list.length === 1
    ? (permanent ? `Permanently delete “${list[0].name}”?` : `Move “${list[0].name}” to Trash?`)
    : (permanent ? `Permanently delete ${list.length} items?` : `Move ${list.length} items to Trash?`);
  if (!window.confirm(label)) return;
  let ok = 0;
  for (const entry of list) {
    try { await apiDelete(entry.path); ok++; } catch (_) { /* keep going */ }
  }
  selection.clear();
  await refresh();
  if (ok) {
    if (permanent) toast(`Deleted ${ok} item${ok === 1 ? '' : 's'}`);
    else notify({ app: 'Files', title: 'Moved to Trash', body: `${ok} item${ok === 1 ? '' : 's'}`, icon: 'ui/trash', urgency: 'low' });
  }
}
async function restoreEntries(list) {
  let ok = 0;
  for (const entry of list) {
    try { await apiRestore(entry.name); ok++; } catch (e) { toast(e.message || 'Could not restore', { type: 'error' }); }
  }
  selection.clear();
  await refresh();
  if (ok) toast(`Restored ${ok} item${ok === 1 ? '' : 's'}`);
}
async function emptyTrash() {
  if (!window.confirm('Empty the Trash? This cannot be undone.')) return;
  try {
    await apiEmptyTrash();
    await refresh();
    toast('Trash emptied');
  } catch (e) {
    toast(e.message || 'Could not empty trash', { type: 'error' });
  }
}

/* ── quick look ─────────────────────────────────────────────── */

let previewApi = null;
let previewIndex = -1;
let previewOpenedAt = 0;

function closePreview() {
  const api = previewApi;
  previewApi = null;
  previewIndex = -1;
  if (!api) return;
  api.close();
  // Each preview builds a fresh modal; drop the previous one after its exit
  // animation so detached overlays don't pile up in the DOM.
  setTimeout(() => api.el.remove(), 400);
}
async function openPreview(entry, index = -1) {
  const list = visibleEntries().filter((e) => e.kind !== 'dir');
  previewIndex = index >= 0 ? index : list.findIndex((e) => e.path === entry.path);
  const nav = { prev: () => stepPreview(-1, list), next: () => stepPreview(1, list), count: list.length };
  closePreview();
  let api;
  api = modal({
    onClose: () => {
      if (previewApi === api) previewApi = null;
      // Each preview builds a fresh modal; drop this one after its exit
      // animation so detached overlays don't pile up in the DOM.
      setTimeout(() => api.el.remove(), 400);
    },
  });
  api.card.classList.add('files-preview-card');
  api.body.remove();
  previewApi = api;
  api.open();
  previewOpenedAt = Date.now();
  try {
    await fillPreview(api.card, entry, nav);
  } catch (_) { /* keep the window up even if a preview fails */ }
}
function stepPreview(dir, list) {
  if (!list.length) return;
  previewIndex = (previewIndex + dir + list.length) % list.length;
  const entry = list[previewIndex];
  selectEntry(entry, null);
  closePreview();
  void openPreview(entry, previewIndex);
}
async function fillPreview(card, entry, nav) {
  const info = document.createElement('div');
  info.className = 'files-preview-meta';
  info.textContent = `${entry.path || homeLabel} · ${entry.kind === 'dir' ? 'Folder' : formatSize(entry.size)}${entry.modified ? ` · ${formatWhen(entry.modified)}` : ''}`;

  if (entry.is_image) {
    const img = document.createElement('img');
    img.className = 'files-preview-img';
    img.src = rawUrl(entry.path);
    img.addEventListener('click', () => img.classList.toggle('is-zoomed'));
    card.append(img, navBar(nav, 'Click image to zoom'), info);
    return;
  }
  if (entry.is_pdf) {
    const box = document.createElement('div');
    box.className = 'files-preview-pdf';
    card.appendChild(box);
    await renderPdfInto(box, entry.path);
    card.append(navBar(nav, ''), info);
    return;
  }
  if (entry.is_video) {
    const video = document.createElement('video');
    video.className = 'files-preview-media';
    video.controls = true; video.autoplay = true; video.src = rawUrl(entry.path);
    card.append(video, navBar(nav, ''), info);
    return;
  }
  if (entry.is_audio) {
    const audio = document.createElement('audio');
    audio.className = 'files-preview-audio';
    audio.controls = true; audio.autoplay = true; audio.src = rawUrl(entry.path);
    card.append(audio, navBar(nav, ''), info);
    return;
  }
  if (entry.is_office) {
    const box = document.createElement('div');
    box.className = 'files-preview-doc';
    box.textContent = 'Loading…';
    card.append(box, navBar(nav, ''), info);
    try {
      renderOfficePreview(box, await apiRender(entry.path));
    } catch (e) {
      box.textContent = `Cannot preview: ${e.message}`;
    }
    return;
  }
  if (entry.is_text) {
    const isCode = /^(rs|py|js|mjs|cjs|ts|tsx|jsx|json|toml|yaml|yml|sh|zsh|bash|css|html|htm|xml|sql|ini|conf|cfg|log)$/.test(entry.ext || '');
    const pre = document.createElement('pre');
    pre.className = 'files-preview-text' + (isCode ? ' files-preview-text--code' : '');
    pre.textContent = 'Loading…';
    card.append(pre, navBar(nav, ''), info);
    try {
      pre.textContent = (await apiRead(entry.path)) || (await apiText(entry.path));
    } catch (e) {
      pre.textContent = `Cannot preview: ${e.message}`;
    }
    return;
  }
  const box = document.createElement('div');
  box.className = 'files-preview-unknown';
  box.appendChild(icon(entryIcon(entry), { size: 48 }));
  const p = document.createElement('p');
  p.textContent = 'No preview for this file type.';
  const dl = button({ label: 'Download', icon: 'ui/download', onClick: () => downloadEntry(entry) });
  box.append(p, dl);
  card.append(box, navBar(nav, ''), info);
}
function navBar(nav, hint) {
  const bar = document.createElement('div');
  bar.className = 'files-preview-nav';
  const prev = button({ icon: 'ui/chevron-left', variant: 'ghost', onClick: nav.prev });
  prev.classList.add('ui-btn--icon');
  const next = button({ icon: 'ui/chevron-right', variant: 'ghost', onClick: nav.next });
  next.classList.add('ui-btn--icon');
  const label = document.createElement('span');
  label.className = 'files-preview-hint';
  label.textContent = hint || `${nav.count} items · ← → to move`;
  bar.append(prev, label, next);
  return bar;
}
let pdfPage = 1;
function renderOfficePreview(box, data) {
  box.innerHTML = '';
  if (!data) { box.textContent = 'No preview'; return; }
  if (data.format === 'doc') {
    const doc = document.createElement('div');
    doc.className = 'files-doc';
    doc.innerHTML = data.html || '';
    sanitizeDocHtml(doc);
    box.appendChild(doc);
    return;
  }
  if (data.format === 'sheet') {
    const grid = Array.isArray(data.grid) ? data.grid : [];
    const cols = grid.reduce((m, r) => Math.max(m, (r || []).length), 0);
    const wrap = document.createElement('div');
    wrap.className = 'files-sheet-wrap';
    const table = document.createElement('table');
    table.className = 'files-sheet';
    grid.forEach((row, r) => {
      const tr = document.createElement('tr');
      for (let c = 0; c < cols; c++) {
        const cell = document.createElement(r === 0 ? 'th' : 'td');
        cell.textContent = (row && row[c] != null) ? row[c] : '';
        tr.appendChild(cell);
      }
      table.appendChild(tr);
    });
    wrap.appendChild(table);
    box.appendChild(wrap);
    return;
  }
  if (data.format === 'slides') {
    const slides = Array.isArray(data.slides) ? data.slides : [];
    const wrap = document.createElement('div');
    wrap.className = 'files-slides';
    slides.forEach((s, i) => {
      const slide = document.createElement('div');
      slide.className = 'files-slide';
      const num = document.createElement('div');
      num.className = 'files-slide-num';
      num.textContent = String(i + 1);
      const body = document.createElement('div');
      body.className = 'files-slide-body';
      if (s.title) { const h = document.createElement('div'); h.className = 'files-slide-title'; h.textContent = s.title; body.appendChild(h); }
      if (s.subtitle) { const p = document.createElement('div'); p.className = 'files-slide-sub'; p.textContent = s.subtitle; body.appendChild(p); }
      if (Array.isArray(s.bullets)) {
        for (const b of s.bullets) { const li = document.createElement('div'); li.className = 'files-slide-bullet'; li.textContent = `• ${b}`; body.appendChild(li); }
      }
      if (s.body) { const p = document.createElement('div'); p.className = 'files-slide-text'; p.textContent = s.body; body.appendChild(p); }
      slide.append(num, body);
      wrap.appendChild(slide);
    });
    box.appendChild(wrap);
    return;
  }
  box.textContent = 'No preview';
}

/** The ODT HTML comes from our own converter (escaped text, limited tags), but
 *  strip anything executable and unsafe link schemes before injecting it. */
function sanitizeDocHtml(root) {
  for (const el of root.querySelectorAll('script,img,iframe,object,embed,style,link')) el.remove();
  for (const a of root.querySelectorAll('a')) {
    const href = a.getAttribute('href') || '';
    if (/^(https?:|mailto:|#|\/)/i.test(href)) {
      a.target = '_blank';
      a.rel = 'noopener noreferrer';
    } else {
      a.removeAttribute('href');
    }
  }
  return root;
}

async function renderPdfInto(host, rel) {
  pdfPage = 1;
  host.innerHTML = '';
  const canvas = await renderPdfPage(rel, pdfPage, 900);
  if (!canvas) { host.textContent = 'Could not render PDF'; return; }
  const pageBox = document.createElement('div');
  pageBox.className = 'files-preview-pdf-page';
  pageBox.appendChild(canvas);
  const controls = document.createElement('div');
  controls.className = 'files-preview-pdf-nav';
  const prev = button({ icon: 'ui/chevron-left', variant: 'ghost', onClick: () => { pdfPage = Math.max(1, pdfPage - 1); void redraw(); } });
  prev.classList.add('ui-btn--icon');
  const label = document.createElement('span'); label.className = 'files-preview-hint'; label.textContent = `Page ${pdfPage}`;
  const next = button({ icon: 'ui/chevron-right', variant: 'ghost', onClick: () => { pdfPage += 1; void redraw(); } });
  next.classList.add('ui-btn--icon');
  controls.append(prev, label, next);
  host.append(pageBox, controls);
  async function redraw() {
    const c = await renderPdfPage(rel, pdfPage, 900);
    if (!c) return;
    pageBox.innerHTML = '';
    pageBox.appendChild(c);
    label.textContent = `Page ${pdfPage}`;
  }
}

/* ── ambient glow ───────────────────────────────────────────── */

function updateGlow() {
  if (!tileEl) return;
  const hue = Math.abs(hash(cwd || 'Home')) % 360;
  setTileGlow(tileEl, glowGradient(`hsl(${hue} 55% 55%)`, `hsl(${(hue + 200) % 360} 45% 24%)`));
  const image = allEntries.find((e) => e.is_image);
  if (image) void setTileGlowFromUrl(tileEl, rawUrl(image.path), { size: 320, blur: 7 }).catch(() => {});
}
function hash(s) {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return h;
}

/* ── lifecycle ──────────────────────────────────────────────── */

export function mountFilesTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile files-tile';
  tileEl.dataset.plugin = FILES_PLUGIN;

  shellEl = document.createElement('div');
  shellEl.className = 'files-shell';

  buildSidebar();

  const main = document.createElement('div');
  main.className = 'files-main';
  buildHeader();
  main.appendChild(headerEl);

  contentEl = document.createElement('div');
  contentEl.className = 'files-content';
  main.appendChild(contentEl);

  statusEl = document.createElement('div');
  statusEl.className = 'files-status-pill hidden';
  main.appendChild(statusEl);

  shellEl.append(sidebarEl, main);
  tileEl.appendChild(shellEl);

  contentEl.addEventListener('click', () => {
    selection.clear();
    syncSelectionDom();
    updateStatus();
  });

  // Drag & drop upload.
  tileEl.addEventListener('dragover', (e) => { e.preventDefault(); tileEl.classList.add('files-tile--drop'); });
  tileEl.addEventListener('dragleave', () => tileEl.classList.remove('files-tile--drop'));
  tileEl.addEventListener('drop', (e) => {
    e.preventDefault();
    tileEl.classList.remove('files-tile--drop');
    const files = [...(e.dataTransfer?.files || [])];
    if (files.length && !isTrashPath(cwd)) void uploadFiles(files);
  });
  tileEl.tabIndex = 0;

  mounted = true;
  void load();
  return tileEl;
}

export function unmountFilesTile() {
  mounted = false;
  closePreview();
  observer?.disconnect();
  observer = null;
  tileEl?.remove();
  tileEl = null; shellEl = null; sidebarEl = null; placesEl = null;
  headerEl = null; pathbarEl = null; contentEl = null; statusEl = null;
  searchBtn = null; viewBtn = null; navBack = null; navFwd = null;
  allEntries = []; entries = []; selection.clear(); lastAnchor = null;
  history = []; forward = []; searchMode = false; query = '';
  textCache.clear(); thumbCache.clear(); renderCache.clear();
}

export function getFilesTileElement() {
  return tileEl;
}

/* ── keyboard ───────────────────────────────────────────────── */

function inTextInput() {
  const tag = document.activeElement?.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA';
}
function onKeyDown(e) {
  if (!tileEl?.isConnected) return;
  if (e.repeat) return;           // ignore held-key repeats (Space must not toggle)
  const active = document.activeElement;
  const inside = tileEl.contains(active) || previewApi;
  if (!inside && !tileEl.matches(':hover')) return;
  const mod = e.ctrlKey || e.metaKey;

  if (inTextInput()) {
    if (e.key === 'Escape' && searchMode) { e.preventDefault(); exitSearch(); }
    return;
  }
  if (previewApi) {
    const list = visibleEntries().filter((x) => x.kind !== 'dir');
    if (e.key === 'ArrowLeft') { e.preventDefault(); void stepPreview(-1, list); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); void stepPreview(1, list); }
    else if (e.key === ' ') { e.preventDefault(); if (Date.now() - previewOpenedAt > 200) closePreview(); }
    else if (e.key === 'Escape') closePreview();
    return;
  }

  if (mod) {
    switch (e.key) {
      case '1': e.preventDefault(); setView('list'); return;
      case '2': e.preventDefault(); setView('grid'); return;
      case 'f': e.preventDefault(); toggleSearch(); return;
      case 'h': e.preventDefault(); toggleHidden(); return;
      case 'l': e.preventDefault(); toggleSearch(); return;
      case 'n': if (e.shiftKey) { e.preventDefault(); void newFolder(); } return;
      case 'a': {
        e.preventDefault();
        selection = new Set(visibleEntries().map((x) => x.path));
        syncSelectionDom(); updateStatus();
        return;
      }
      default: break;
    }
  }

  const trashed = isTrashPath(cwd);

  switch (e.key) {
    case 'Enter': {
      if (!selection.size) return;
      e.preventDefault();
      const first = selectionEntries()[0];
      if (first) openEntry(first);
      return;
    }
    case ' ': {
      if (!selection.size) return;
      e.preventDefault();
      const first = selectionEntries()[0];
      if (first) void openPreview(first);
      return;
    }
    case 'ArrowDown': e.preventDefault(); moveSelection(1); return;
    case 'ArrowUp': e.preventDefault(); moveSelection(-1); return;
    case 'ArrowRight': { e.preventDefault(); const first = selectionEntries()[0]; if (first?.kind === 'dir') void navigate(first.path); return; }
    case 'ArrowLeft': e.preventDefault(); goUp(); return;
    case 'F2': { if (trashed) return; const first = selectionEntries()[0]; if (first) { e.preventDefault(); void renameEntry(first); } return; }
    case 'Delete':
    case 'Backspace': { if (selection.size) { e.preventDefault(); void removeEntries(selectionEntries(), trashed); } return; }
    default: break;
  }
}

/* ── AI wiring ──────────────────────────────────────────────── */

function onAgentActions(e) {
  const actions = e.detail || [];
  const fileActions = actions.filter((a) => /^file_/.test(a?.action || ''));
  if (!fileActions.length) return;

  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: FILES_PLUGIN } }));

  const deleted = fileActions.some((a) => a.action === 'file_delete' && a.result === 'ok');
  const touched = fileActions
    .map((a) => a.data?.path || a.data?.to || a.data?.trashed || a.data?.restored)
    .find((p) => typeof p === 'string' && p);
  const isDirAction = fileActions.some((a) => a.action === 'file_mkdir' && a.result === 'ok');

  void (async () => {
    if (deleted) { selection.clear(); await refresh(); return; }
    if (touched) {
      const dir = isDirAction ? touched : parentOf(touched);
      await navigate(dir, { push: true });
      const found = allEntries.find((en) => en.path === touched);
      if (found && !isDirAction) selectEntry(found, {});
      return;
    }
    await refresh();
  })();
}

let wired = false;
export function wireFilesEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  window.addEventListener('keydown', onKeyDown);
  window.addEventListener('app:notification-action', (e) => {
    if (e.detail?.action === 'restore') void navigate('.Trash');
  });
  // Other apps "reveal" saved files here (web/js/files.js).
  window.addEventListener('plugin:focus', (e) => {
    if (e.detail?.name !== FILES_PLUGIN) return;
    if (typeof e.detail.path !== 'string' || e.detail.path === '') return;
    void navigate(e.detail.path, { push: true });
  });
  // A peer plugin saved a file — refresh if it landed in the open folder.
  window.addEventListener('files:changed', (e) => {
    if (!mounted) return;
    const dir = e.detail?.dir ?? '';
    if (cwd === dir || cwd === '') void refresh();
  });
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19). */
export function filesContextMenu(ctx) {
  const target = ctx?.target?.closest?.('.files-entry');
  const rel = target?.dataset?.path || (selection.size === 1 ? [...selection][0] : null);
  const entry = allEntries.find((en) => en.path === rel) || null;
  const trashed = isTrashPath(rel || cwd);
  const selected = selectionEntries();
  // Right-clicking an unselected entry acts on that entry (GNOME behaviour).
  const targets = entry && !selection.has(entry.path) ? [entry] : selected;
  const items = [];

  if (entry) {
    items.push({ type: 'item', label: 'Open', icon: 'ui/expand', onClick: () => openEntry(entry) });
    if (entry.kind === 'file') items.push({ type: 'item', label: 'Download', icon: 'ui/download', onClick: () => downloadEntry(entry) });
  }
  items.push({ type: 'item', label: 'New Folder', icon: 'ui/folder-plus', onClick: () => void newFolder() });
  items.push({ type: 'item', label: 'Upload Files…', icon: 'ui/upload', onClick: pickUpload });
  items.push({ type: 'separator' });
  if (trashed) {
    items.push({ type: 'item', label: 'Restore', icon: 'ui/refresh', disabled: !targets.length, onClick: () => void restoreEntries(targets) });
    items.push({ type: 'item', label: 'Delete Permanently', icon: 'ui/trash', danger: true, disabled: !targets.length, onClick: () => void removeEntries(targets, true) });
  } else {
    items.push({ type: 'item', label: 'Rename…', icon: 'ui/pencil', disabled: targets.length !== 1, onClick: () => targets.length === 1 && void renameEntry(targets[0]) });
    items.push({ type: 'item', label: 'Move to Trash', icon: 'ui/trash', danger: true, disabled: !targets.length, onClick: () => void removeEntries(targets, false) });
  }
  items.push({ type: 'separator' });
  items.push({ type: 'item', label: 'Show Hidden Files', icon: 'ui/eye', checked: showHidden, onClick: toggleHidden });
  if (isTrashPath(cwd)) {
    items.push({ type: 'separator' });
    items.push({ type: 'item', label: 'Empty Trash', icon: 'ui/warning', danger: true, onClick: () => void emptyTrash() });
  }
  return items;
}

export default {
  name: 'files',
  icon: 'ui/folder',
  mount: mountFilesTile,
  unmount: unmountFilesTile,
  getElement: getFilesTileElement,
  wireEvents: wireFilesEvents,
  contextMenu: filesContextMenu,
};
