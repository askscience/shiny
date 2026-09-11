/**
 * impress.js — the Impress plugin's window (slide deck builder).
 *
 * Decks are stored server-side as a JSON array of slides (the SDK `Slide`
 * model) and exported as real OpenDocument Presentation (.odp) files. This
 * window is the human editor: a slide strip (thumbnails + reorder/add), a
 * 16:9 stage whose text is edited directly (contentEditable), a slim
 * inspector (layout selector, speaker notes, slide ops), and a Present
 * overlay.
 *
 * AI wiring: `slide_*` tool outcomes arrive via the `agent:actions` event —
 * the window refreshes its deck list and opens what the AI touched.
 */

import {
  icon, button, emptyState, toast,
  setTileGlow, glowGradient,
} from '/ui/index.js';
import { setIcon } from '/ui/index.js';
import { apiFetch } from '/js/api.js';

export const IMPRESS_PLUGIN = 'impress';

const THEMES = ['aurora', 'slate', 'ocean', 'mono', 'ember'];
// Glow colours per theme — must stay in sync with web/css/tiles.css.
const THEME_GLOW = {
  aurora: { accent: '#6366f1', darkB: '#2b2b52' },
  slate: { accent: '#475569', darkB: '#1e293b' },
  ocean: { accent: '#0ea5e9', darkB: '#0c4a6e' },
  mono: { accent: '#18181b', darkB: '#1c1c1c' },
  ember: { accent: '#f97316', darkB: '#2d1f14' },
};
const LAYOUTS = ['title', 'section', 'content', 'two-column', 'quote', 'blank'];
// Per-slide animation. TRANSITIONS must stay in sync with the SDK
// `odp::TRANSITIONS` (the ODF export maps each one to a standard effect);
// REVEALS matches `odp::REVEALS`.
const TRANSITIONS = ['none', 'fade', 'slide', 'push', 'zoom'];
const REVEALS = ['all', 'bullets'];
const TRANSITION_LABELS = {
  none: 'None', fade: 'Fade', slide: 'Slide', push: 'Push', zoom: 'Zoom',
};
const REVEAL_LABELS = { all: 'All at once', bullets: 'One by one' };

let tileEl = null;
let deckMenuBtn = null;
let presentBtn = null;
let titleInput = null;
let themeSelect = null;
let saveDot = null;
let statusEl = null;
let stripEl = null;
let stageEl = null;
let presentEl = null;
let stageWrapEl = null;
let inspectorEl = null;
let stageSlideNode = null;
let resizeObserver = null;

// Inspector controls (created once; repopulated per selection). Slide text is
// edited directly on the stage via contentEditable; the inspector keeps only
// the layout selector, speaker notes and slide operations.
let layoutSelect = null;
let transitionSelect = null;
let revealSelect = null;
let fieldNotes = null;
let thumbRefreshTimer = null;

let decks = [];
let current = null;   // { id, title, theme, aspect, slides: [], updated_at }
let selIndex = 0;
let presentIndex = -1; // -1 = not presenting
let presentStep = 0;   // bullets revealed on the current slide (reveal: 'bullets')
let presentFrags = []; // the current slide's bullet nodes ([] unless building)
let presentFullBtn = null;
let dirty = false;
let saveTimer = null;
let saveSeq = 0;

/* Popup (body-level, the tile clips overflow) */
let deckMenuPopup = null;
let deckMenuOpen = false;

/* ── API ────────────────────────────────────────────────────── */

async function listDecks() {
  const res = await apiFetch('/api/presentations');
  return res?.data || [];
}

async function fetchDeck(id) {
  const res = await apiFetch(`/api/presentations/${encodeURIComponent(id)}`);
  return res?.data;
}

async function createDeck(title = 'Untitled') {
  const res = await apiFetch('/api/presentations', {
    method: 'POST',
    body: JSON.stringify({ title, theme: 'aurora' }),
  });
  return res?.data;
}

async function saveDeck(id, { title, theme, slides }) {
  const seq = ++saveSeq;
  await apiFetch(`/api/presentations/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify({ title, theme, slides }),
  });
  return seq;
}

async function deleteDeck(id) {
  await apiFetch(`/api/presentations/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/* ── Slide model helpers ────────────────────────────────────── */

function newSlide(layout = 'content') {
  return {
    layout,
    title: '',
    subtitle: '',
    bullets: [],
    columns: [[], []],
    body: '',
    attribution: '',
    notes: '',
    transition: 'none',
    reveal: 'all',
  };
}

function normalizeSlide(s) {
  const slide = { ...newSlide(), ...(s || {}) };
  if (!LAYOUTS.includes(slide.layout)) slide.layout = 'content';
  if (!TRANSITIONS.includes(slide.transition)) slide.transition = 'none';
  if (!REVEALS.includes(slide.reveal)) slide.reveal = 'all';
  slide.bullets = Array.isArray(slide.bullets) ? slide.bullets : [];
  slide.columns = Array.isArray(slide.columns) ? slide.columns : [[], []];
  slide.title = String(slide.title || '');
  slide.subtitle = String(slide.subtitle || '');
  slide.body = String(slide.body || '');
  slide.attribution = String(slide.attribution || '');
  slide.notes = String(slide.notes || '');
  return slide;
}

function slideCount() {
  return current?.slides?.length || 0;
}

/* ── Rendering ──────────────────────────────────────────────── */

function h(tag, className, text) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text != null) el.textContent = text;
  return el;
}

/** Render one slide as a DOM node (used by the stage and thumbnails).
 *  `editable` makes the stage slide's text contentEditable so it can be
 *  edited in place; thumbnails and the Present overlay render read-only. */
function slideEl(slide, theme, editable = false) {
  const el = h('div', `impress-slide impress-slide--${slide.layout}`);
  el.dataset.theme = theme;

  const addText = (field, className, content, multiline = false) => {
    if (!editable && String(content || '').trim() === '') return;
    const node = h('div', className, content);
    if (editable) {
      node.contentEditable = 'true';
      node.spellcheck = false;
      node.dataset.field = field;
      if (multiline) node.dataset.multiline = '1';
      node.setAttribute('role', 'textbox');
      const placeholder = field === 'body'
        ? (slide.layout === 'blank' ? 'Add text' : '')
        : ({ title: 'Add a title', subtitle: 'Add a subtitle', attribution: 'Attribution' }[field] || '');
      if (placeholder) node.dataset.placeholder = placeholder;
      wireTextField(node);
    }
    el.appendChild(node);
  };

  if (slide.layout === 'title') {
    addText('title', 'impress-slide-title', slide.title);
    addText('subtitle', 'impress-slide-subtitle', slide.subtitle);
    return el;
  }

  if (slide.layout === 'section') {
    addText('title', 'impress-slide-title', slide.title);
    return el;
  }

  if (slide.layout === 'quote') {
    addText('body', 'impress-slide-quote', slide.body, true);
    addText('attribution', 'impress-slide-attribution', slide.attribution);
    return el;
  }

  if (slide.layout === 'blank') {
    addText('body', 'impress-slide-body', slide.body, true);
    return el;
  }

  // content + two-column share a title block.
  addText('title', 'impress-slide-title', slide.title);

  if (slide.layout === 'two-column') {
    const cols = h('div', 'impress-slide-columns');
    const [left = [], right = []] = slide.columns || [[], []];
    cols.appendChild(bulletList(left, 'columns', 0, editable));
    cols.appendChild(bulletList(right, 'columns', 1, editable));
    el.appendChild(cols);
  } else {
    el.appendChild(bulletList(slide.bullets, 'bullets', 0, editable));
  }
  return el;
}

function bulletList(items, field, colIdx, editable) {
  const ul = h('div', 'impress-slide-bullets');
  ul.dataset.field = field;
  if (field === 'columns') ul.dataset.col = String(colIdx);
  let list = (items || []).filter((b) => editable || String(b).trim() !== '');
  if (editable && list.length === 0) list = [''];
  for (const b of list) ul.appendChild(makeBullet(String(b), ul, editable));
  return ul;
}

function makeBullet(text, containerEl, editable) {
  const d = document.createElement('div');
  d.className = 'impress-slide-bullet';
  d.textContent = text;
  if (!editable) return d;

  d.contentEditable = 'true';
  d.spellcheck = true;
  d.addEventListener('input', () => syncBullets(containerEl));
  d.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      const next = makeBullet('', containerEl, true);
      d.after(next);
      next.focus();
      syncBullets(containerEl);
    } else if (e.key === 'Backspace' && d.textContent === '') {
      const siblings = [...containerEl.querySelectorAll('.impress-slide-bullet')];
      if (siblings.length > 1) {
        e.preventDefault();
        const idx = siblings.indexOf(d);
        d.remove();
        (siblings[idx - 1] || siblings[idx + 1])?.focus();
        syncBullets(containerEl);
      }
    }
  });
  return d;
}

function syncBullets(containerEl) {
  const slide = currentSlide();
  if (!slide) return;
  const items = [...containerEl.querySelectorAll('.impress-slide-bullet')]
    .map((b) => b.textContent);
  if (containerEl.dataset.field === 'columns') {
    const col = Number(containerEl.dataset.col || 0);
    while (slide.columns.length <= col) slide.columns.push([]);
    slide.columns[col] = items;
  } else {
    slide.bullets = items;
  }
  markDirty();
  scheduleThumbRefresh();
}

function wireTextField(node) {
  node.addEventListener('input', () => {
    const slide = currentSlide();
    if (!slide) return;
    slide[node.dataset.field] = node.innerText;
    markDirty();
    scheduleThumbRefresh();
  });
  if (!node.dataset.multiline) {
    node.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') { e.preventDefault(); node.blur(); }
    });
  }
}

function renderStage() {
  if (!stageEl || !current) return;
  resizeStage();
  stageEl.innerHTML = '';
  const slide = current.slides[selIndex];
  if (slide) {
    stageSlideNode = slideEl(slide, current.theme, true);
    stageEl.appendChild(stageSlideNode);
    fitSlide(stageEl, stageSlideNode);
  } else {
    stageSlideNode = null;
  }
}

function renderStrip() {
  if (!stripEl || !current) return;
  stripEl.innerHTML = '';
  const items = [];
  current.slides.forEach((slide, i) => {
    const item = h('button', 'impress-thumb');
    item.type = 'button';
    item.dataset.index = String(i);
    if (i === selIndex) item.classList.add('is-active');
    const num = h('span', 'impress-thumb-num', String(i + 1));
    const mini = slideEl(slide, current.theme);
    mini.classList.add('impress-thumb-slide');
    item.append(num, mini);
    item.title = `${i + 1}. ${slide.title || slide.layout}`;
    item.addEventListener('click', () => selectSlide(i));
    items.push(item);
  });
  items.forEach((item) => stripEl.appendChild(item));
  // Scale the 640px-wide logical slide to each thumbnail's real width.
  items.forEach((item) => {
    const mini = item.querySelector('.impress-slide');
    if (mini) fitSlide(item, mini);
  });
}

/** Scale a 640×360 logical slide to fill a container's width. */
function fitSlide(container, node) {
  node.style.transformOrigin = 'top left';
  node.style.transform = `scale(${container.clientWidth / 640})`;
}

/** Size the stage container to the largest 16:9 box that fits above the
 *  inspector (width- and height-constrained). The slide inside is then scaled
 *  by fitSlide to fill it exactly. */
function resizeStage() {
  if (!stageEl || !stageWrapEl) return;
  const gutter = 24;           // 12px horizontal inset on each side
  const vgap = 12;             // vertical breathing room around the stage
  const inspectorH = inspectorEl ? inspectorEl.offsetHeight : 0;
  const availW = stageWrapEl.clientWidth - gutter;
  const availH = stageWrapEl.clientHeight - inspectorH - vgap * 2;
  if (availW <= 0 || availH <= 0) return;
  const scale = Math.min(availW / 640, availH / 360);
  stageEl.style.width = `${Math.floor(640 * scale)}px`;
  stageEl.style.height = `${Math.floor(360 * scale)}px`;
}

function observeStageSize() {
  if (typeof ResizeObserver === 'undefined' || resizeObserver || !stageWrapEl) return;
  resizeObserver = new ResizeObserver(() => {
    resizeStage();
    if (stageSlideNode && stageEl) fitSlide(stageEl, stageSlideNode);
  });
  resizeObserver.observe(stageWrapEl);
  if (inspectorEl) resizeObserver.observe(inspectorEl);
}

function currentSlide() {
  return current?.slides?.[selIndex] || null;
}

function renderInspector() {
  if (!current) return;
  const slide = current.slides[selIndex] || newSlide();
  layoutSelect.value = slide.layout;
  if (transitionSelect) transitionSelect.value = slide.transition;
  if (revealSelect) revealSelect.value = slide.reveal;
  fieldNotes.value = slide.notes;
}

/** Copy the selected slide's transition + bullet build onto every slide. */
function applyAnimationToAll() {
  if (!current) return;
  const source = current.slides[selIndex] || newSlide();
  for (const slide of current.slides) {
    slide.transition = source.transition;
    slide.reveal = source.reveal;
  }
  markDirty();
}

/** Debounce thumbnail redraws while the user types on the stage — the stage
 *  itself must NOT be re-rendered mid-edit or the caret/focus is lost. */
function scheduleThumbRefresh() {
  window.clearTimeout(thumbRefreshTimer);
  thumbRefreshTimer = window.setTimeout(() => {
    thumbRefreshTimer = null;
    if (current) renderStrip();
  }, 300);
}

/* ── State / persistence ────────────────────────────────────── */

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
    statusEl.textContent = `Saved ${t} · ${slideCount()} ${slideCount() === 1 ? 'slide' : 'slides'}`;
    saveDot?.classList.remove('is-active');
  }
}

function markDirty() {
  dirty = true;
  setStatus('dirty');
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => void persist(), 1200);
}

async function persist() {
  if (!current || !dirty) return;
  dirty = false;
  setStatus('saving');
  try {
    await saveDeck(current.id, {
      title: titleInput.value.trim(),
      theme: current.theme,
      slides: current.slides,
    });
    current.title = titleInput.value.trim();
    current.updated_at = new Date().toISOString();
    setStatus('saved');
  } catch (e) {
    dirty = true;
    setStatus('dirty');
    toast(e.message || 'Save failed', { type: 'error' });
  }
}

/* ── Open / create / delete ─────────────────────────────────── */

/** Repaint the deck's ambient glow from its slide theme (null = no deck). */
function updateGlow() {
  const theme = THEME_GLOW[current?.theme] || THEME_GLOW.aurora;
  setTileGlow(tileEl, current
    ? glowGradient(theme.accent, theme.darkB, { x: 22, y: 0, x2: 82, y2: 100 })
    : null);
}

function selectSlide(i) {
  selIndex = Math.max(0, Math.min(i, slideCount() - 1));
  renderStrip();
  renderStage();
  renderInspector();
  updateGlow();
}

async function openDeck(deck) {
  window.clearTimeout(saveTimer);
  dirty = false;
  saveSeq++;
  try {
    const full = deck.slides != null ? deck : await fetchDeck(deck.id);
    current = {
      id: full.id,
      title: full.title,
      theme: full.theme || 'aurora',
      aspect: full.aspect || '16x9',
      slides: (full.slides || []).map(normalizeSlide),
      updated_at: full.updated_at,
    };
    if (current.slides.length === 0) current.slides.push(newSlide('title'));
    titleInput.value = current.title;
    themeSelect.value = current.theme;
    selIndex = 0;
    renderStrip();
    renderStage();
    renderInspector();
    setStatus('saved');
    updateGlow();
  } catch (e) {
    toast(e.message || 'Could not open presentation', { type: 'error' });
  }
}

async function openNewest() {
  const list = await listDecks();
  decks = list;
  if (list.length) await openDeck(list[0]);
}

async function newDeck() {
  try {
    const created = await createDeck();
    await refreshDecks();
    await openDeck(created);
  } catch (e) {
    toast(e.message || 'Could not create presentation', { type: 'error' });
  }
}

async function removeCurrent() {
  if (!current) return;
  if (!window.confirm(`Delete "${current.title}"?`)) return;
  try {
    await deleteDeck(current.id);
    current = null;
    updateGlow();
    await refreshDecks();
    await openNewest();
  } catch (e) {
    toast(e.message || 'Could not delete presentation', { type: 'error' });
  }
}

async function refreshDecks() {
  try {
    decks = await listDecks();
  } catch (_) { /* keep last list */ }
}

/* ── Deck menu ──────────────────────────────────────────────── */

function ensureDeckMenu() {
  if (deckMenuPopup) return;
  deckMenuPopup = document.createElement('div');
  deckMenuPopup.className = 'impress-deck-menu hidden';
  deckMenuPopup.setAttribute('role', 'menu');
  document.body.appendChild(deckMenuPopup);
}

function closeDeckMenu() {
  if (!deckMenuOpen) return;
  deckMenuOpen = false;
  deckMenuPopup?.classList.add('hidden');
  if (deckMenuBtn) deckMenuBtn.setAttribute('aria-expanded', 'false');
  document.removeEventListener('pointerdown', onDeckMenuOutside, true);
  document.removeEventListener('keydown', onDeckMenuKey, true);
}

function onDeckMenuOutside(e) {
  if (deckMenuPopup && !deckMenuPopup.contains(e.target)
    && deckMenuBtn && !deckMenuBtn.contains(e.target)) {
    closeDeckMenu();
  }
}

function onDeckMenuKey(e) {
  if (e.key === 'Escape') closeDeckMenu();
}

function formatWhen(iso) {
  if (!iso) return '';
  try {
    return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric' });
  } catch (_) {
    return '';
  }
}

function renderDeckMenuItems() {
  if (!deckMenuPopup) return;
  deckMenuPopup.innerHTML = '';
  if (!decks.length) {
    const empty = h('div', 'impress-deck-menu-empty', 'No presentations yet');
    deckMenuPopup.appendChild(empty);
  } else {
    decks.forEach((deck) => {
      const item = h('button', 'impress-deck-menu-item');
      item.type = 'button';
      item.setAttribute('role', 'menuitem');
      if (deck.id === current?.id) item.classList.add('is-active');

      const title = h('span', 'impress-deck-menu-title', deck.title);
      const meta = h('span', 'impress-deck-menu-time', `${deck.slide_count || 0} · ${formatWhen(deck.updated_at)}`);
      const check = h('span', 'impress-deck-menu-check');
      item.append(title, meta, check);
      void setIcon(check, 'ui/check', { size: 13 });

      item.addEventListener('click', () => {
        closeDeckMenu();
        if (deck.id !== current?.id) void openDeck(deck);
      });
      deckMenuPopup.appendChild(item);
    });
  }

  const foot = h('div', 'impress-deck-menu-foot');
  const footItem = (iconName, label, danger, onClick) => {
    const item = h('button', 'impress-deck-menu-item');
    item.type = 'button';
    item.setAttribute('role', 'menuitem');
    if (danger) item.classList.add('impress-deck-menu-item--danger');
    const ic = h('span', 'impress-deck-menu-foot-icon');
    item.appendChild(ic);
    void setIcon(ic, iconName, { size: 14 });
    const labelEl = h('span', 'impress-deck-menu-title', label);
    item.appendChild(labelEl);
    item.addEventListener('click', () => {
      closeDeckMenu();
      onClick();
    });
    foot.appendChild(item);
  };
  footItem('ui/plus', 'New presentation', false, () => void newDeck());
  footItem('ui/download', 'Import .odp', false, pickOdpFile);
  footItem('ui/upload', 'Export .odp', false, () => void exportOdp());
  footItem('ui/trash', 'Delete presentation', true, () => void removeCurrent());
  deckMenuPopup.appendChild(foot);
}

function openDeckMenu() {
  ensureDeckMenu();
  renderDeckMenuItems();
  deckMenuPopup.classList.remove('hidden');
  const r = deckMenuBtn.getBoundingClientRect();
  const left = Math.max(12, Math.min(r.left, window.innerWidth - 280 - 12));
  deckMenuPopup.style.left = `${left}px`;
  deckMenuPopup.style.top = `${r.bottom + 8}px`;
  deckMenuOpen = true;
  deckMenuBtn.setAttribute('aria-expanded', 'true');
  document.addEventListener('pointerdown', onDeckMenuOutside, true);
  document.addEventListener('keydown', onDeckMenuKey, true);
}

function toggleDeckMenu() {
  if (deckMenuOpen) closeDeckMenu();
  else openDeckMenu();
}

/* ── Import / export ────────────────────────────────────────── */

function pickOdpFile() {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = '.odp,application/vnd.oasis.opendocument.presentation';
  input.addEventListener('change', () => {
    const file = input.files?.[0];
    if (file) void importOdp(file);
  });
  input.click();
}

async function importOdp(file) {
  const form = new FormData();
  form.append('file', file);
  try {
    await apiFetch('/api/presentations/import', { method: 'POST', body: form });
    toast(`Imported ${file.name}`, { type: 'info' });
    await refreshDecks();
    await openNewest();
  } catch (e) {
    toast(e.message || 'Import failed — is this a valid .odp file?', { type: 'error' });
  }
}

async function exportOdp() {
  if (!current) return;
  try {
    const blob = await apiFetch(
      `/api/presentations/${encodeURIComponent(current.id)}/export`,
      { responseType: 'blob' },
    );
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${current.title || 'presentation'}.odp`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  } catch (e) {
    toast(e.message || 'Export failed', { type: 'error' });
  }
}

/* ── Present mode ───────────────────────────────────────────── */

/* Input guards for the show. The overlay covers the whole screen (including the
 * toolbar button that started it), so "advance" must only fire for a press that
 * began on the overlay — and never for the click that started the show. A short
 * arm delay after the start additionally swallows the second click of an
 * impatient double-click and any stray Enter/Space, so the starting gesture can
 * never be read as "next slide". */
const PRESENT_ARM_MS = 500;
let presentPressFromOverlay = false;
let presentArmedAt = 0;

/** True during the brief window right after the show starts. */
function presentInputLocked() {
  return performance.now() - presentArmedAt < PRESENT_ARM_MS;
}

function ensurePresentEl() {
  if (presentEl) return;
  presentEl = h('div', 'impress-present hidden');
  // Focusable so the show owns the keyboard: without this, the toolbar's
  // Present button keeps focus and Space/Enter re-trigger it (restarting the
  // deck) instead of advancing.
  presentEl.tabIndex = -1;
  presentEl.addEventListener('pointerdown', (e) => {
    presentPressFromOverlay = presentEl.contains(e.target) && !inPresentControls(e.target);
  });
  presentEl.addEventListener('pointercancel', () => { presentPressFromOverlay = false; });
  presentEl.addEventListener('keydown', onPresentKey);
  presentEl.addEventListener('click', (e) => {
    const fromOverlay = presentPressFromOverlay && presentEl.contains(e.target);
    presentPressFromOverlay = false;
    if (!fromOverlay || inPresentControls(e.target)) return;
    if (e.detail > 1) return;      // 2nd click of a double-click on Present
    if (presentInputLocked()) return;
    nextSlide();
  });

  // Keep the full-screen button (and the slide's fit) in sync when the user
  // enters/leaves full screen with the button, Esc/F11, or resizes the window.
  document.addEventListener('fullscreenchange', onPresentViewportChange);
  window.addEventListener('resize', onPresentViewportChange);
  // The show is a window-sized preview inside the tile; only the full-screen
  // control promotes it to the browser's top layer.
  tileEl.appendChild(presentEl);
}

/** Re-fit the show after a viewport change (window resize, full screen). */
function onPresentViewportChange() {
  syncPresentFullBtn();
  fitPresentSlide();
  // Leaving/entering full screen relayouts a frame later in some browsers.
  requestAnimationFrame(fitPresentSlide);
}

/** Scale the current slide to fill the overlay. The 640×360 slide is the
 *  logical canvas; the transform is inline so it must be re-applied on every
 *  size change (and never while the overlay is hidden, where it measures 0). */
function fitPresentSlide() {
  const node = presentEl?.querySelector('.impress-present-stage > .impress-slide');
  if (!node) return;
  const w = presentEl.clientWidth;
  const h = presentEl.clientHeight;
  if (!w || !h) return;   // hidden — the stylesheet's scale(1) stands in
  node.style.transformOrigin = 'center center';
  node.style.transform = `scale(${Math.min(w / 640, h / 360)})`;
}

/** The overlay's control bar: back to the editor + real (browser) full screen. */
function buildPresentControls() {
  const controls = h('div', 'impress-present-controls');
  const control = (iconName, label, onClick) => {
    const btn = button({ icon: iconName, variant: 'ghost', onClick });
    btn.classList.add('ui-btn--icon', 'impress-present-btn');
    btn.title = label;
    btn.setAttribute('aria-label', label);
    // A control press is not a "next slide" press.
    btn.addEventListener('pointerdown', (e) => e.stopPropagation());
    btn.addEventListener('click', (e) => e.stopPropagation());
    controls.appendChild(btn);
    return btn;
  };
  control('ui/arrow-left', 'Back to the editor', stopPresent);
  presentFullBtn = control('ui/expand', 'Full screen', togglePresentFullscreen);
  syncPresentFullBtn();
  return controls;
}

/** True when `target` is one of the overlay's own controls. */
function inPresentControls(target) {
  const controls = presentEl?.querySelector('.impress-present-controls');
  return !!(controls && target && controls.contains(target));
}

function startPresent() {
  if (!current || !slideCount()) return;
  ensurePresentEl();
  presentPressFromOverlay = false;
  presentArmedAt = performance.now();
  presentIndex = 0;
  presentStep = 0;
  presentFrags = [];
  // Show the overlay *before* rendering: the slide is scaled from the
  // overlay's measured size, and a `display: none` overlay measures 0 — which
  // rendered slide 1 at scale(0), i.e. an apparently blank first slide.
  presentEl.classList.remove('hidden');
  renderPresent();
  presentEl.focus({ preventScroll: true });
}

function stopPresent() {
  if (presentIndex < 0) return;
  presentIndex = -1;
  presentStep = 0;
  presentFrags = [];
  presentPressFromOverlay = false;
  if (document.fullscreenElement) void document.exitFullscreen().catch(() => {});
  presentEl?.classList.add('hidden');
  syncPresentFullBtn();
  presentBtn?.focus({ preventScroll: true });
}

/** Keyboard while presenting. Bound to the overlay (which holds focus), so the
 *  rest of the app keeps its own keys while the preview is up. */
function onPresentKey(e) {
  if (presentIndex < 0) return;
  const k = e.key;
  if (k === 'ArrowRight' || k === 'ArrowDown' || k === 'PageDown' || k === ' ' || k === 'Enter') {
    e.preventDefault();
    if (!presentInputLocked()) nextSlide();
  } else if (k === 'ArrowLeft' || k === 'ArrowUp' || k === 'PageUp') {
    e.preventDefault();
    if (!presentInputLocked()) prevSlide();
  } else if (k === 'Escape') {
    // In full screen the browser consumes the first Esc to leave it.
    e.preventDefault();
    stopPresent();
  }
}

/** Toggle real browser full screen for the show. */
async function togglePresentFullscreen() {
  if (!presentEl) return;
  try {
    if (document.fullscreenElement) await document.exitFullscreen();
    else await presentEl.requestFullscreen({ navigationUI: 'hide' });
  } catch (_) {
    toast('Full screen was blocked by the browser', { type: 'info' });
  }
  syncPresentFullBtn();
}

function syncPresentFullBtn() {
  if (!presentFullBtn) return;
  const full = !!document.fullscreenElement;
  presentFullBtn.classList.toggle('is-active', full);
  presentFullBtn.title = full ? 'Exit full screen' : 'Full screen';
  presentFullBtn.setAttribute('aria-label', full ? 'Exit full screen' : 'Full screen');
}

function renderPresent() {
  if (!presentEl || presentIndex < 0) return;
  presentEl.innerHTML = '';
  const slide = current.slides[presentIndex];
  presentFrags = [];
  presentStep = 0;
  if (slide) {
    const node = slideEl(slide, current.theme);
    // The slide carries the fit-to-screen scale as an inline transform, so the
    // entrance animation must run on a wrapper — a CSS animation on the slide
    // itself would override `scale()` and blow the layout up to 640×360.
    const stage = h('div', 'impress-present-stage');
    presentEl.dataset.transition = TRANSITIONS.includes(slide.transition) ? slide.transition : 'none';
    stage.appendChild(node);
    presentEl.appendChild(stage);
    fitPresentSlide();

    // Progressive bullet build: bullets start hidden and arrive one advance at
    // a time (`reveal: 'bullets'`). Hidden keeps layout stable (opacity only).
    presentFrags = slide.reveal === 'bullets'
      ? [...node.querySelectorAll('.impress-slide-bullet')]
      : [];
    revealFragments(0);
  }
  const count = h('div', 'impress-present-count', `${presentIndex + 1} / ${slideCount()}`);
  presentEl.appendChild(count);
  presentEl.appendChild(buildPresentControls());
}

/** Show the first `n` bullet fragments of the current slide. */
function revealFragments(n) {
  presentStep = Math.max(0, Math.min(n, presentFrags.length));
  presentFrags.forEach((frag, i) => frag.classList.toggle('is-hidden', i >= presentStep));
}

function nextSlide() {
  // A slide with a bullet build spends one advance per bullet first.
  if (presentStep < presentFrags.length) {
    revealFragments(presentStep + 1);
    return;
  }
  if (presentIndex < slideCount() - 1) {
    presentIndex++;
    renderPresent();
  } else {
    stopPresent();
  }
}

function prevSlide() {
  // Step back through a bullet build before leaving the slide.
  if (presentStep > 0) {
    revealFragments(presentStep - 1);
    return;
  }
  if (presentIndex > 0) {
    presentIndex--;
    renderPresent();
    // Arriving from the right, the previous slide shows in full.
    revealFragments(presentFrags.length);
  }
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

function toolbarButton(iconName, label, onClick, danger = false) {
  const btn = button({ icon: iconName, variant: 'ghost', onClick });
  btn.classList.add('ui-btn--icon', 'impress-tool');
  if (danger) btn.classList.add('impress-tool--danger');
  btn.title = label;
  btn.setAttribute('aria-label', label);
  return btn;
}

/** Create the Impress tile element (the plugin's window container). */
export function mountImpressTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile impress-tile';
  tileEl.dataset.plugin = IMPRESS_PLUGIN;

  /* Top bar: deck menu + title + theme + save indicator */
  const bar = h('div', 'impress-bar');

  deckMenuBtn = document.createElement('button');
  deckMenuBtn.type = 'button';
  deckMenuBtn.className = 'impress-deck-btn';
  deckMenuBtn.setAttribute('aria-haspopup', 'menu');
  deckMenuBtn.setAttribute('aria-expanded', 'false');
  deckMenuBtn.title = 'Presentations';
  const deckIcon = h('span', 'impress-deck-btn-icon');
  deckMenuBtn.appendChild(deckIcon);
  void setIcon(deckIcon, 'ui/present', { size: 15 });
  const chevron = h('span', 'impress-deck-btn-chevron');
  deckMenuBtn.appendChild(chevron);
  void setIcon(chevron, 'ui/chevron-down', { size: 12 });
  deckMenuBtn.addEventListener('click', toggleDeckMenu);

  titleInput = document.createElement('input');
  titleInput.className = 'impress-title';
  titleInput.type = 'text';
  titleInput.placeholder = 'Untitled';
  titleInput.maxLength = 120;
  titleInput.autocomplete = 'off';
  titleInput.addEventListener('change', () => {
    if (current) {
      current.title = titleInput.value.trim();
      markDirty();
    }
  });

  themeSelect = document.createElement('select');
  themeSelect.className = 'impress-theme';
  themeSelect.title = 'Theme';
  for (const t of THEMES) {
    const opt = document.createElement('option');
    opt.value = t;
    opt.textContent = t;
    themeSelect.appendChild(opt);
  }
  themeSelect.addEventListener('change', () => {
    if (!current) return;
    current.theme = themeSelect.value;
    markDirty();
    renderStage();
    renderStrip();
    updateGlow();
  });

  saveDot = h('span', 'impress-save-dot');
  saveDot.setAttribute('aria-hidden', 'true');

  presentBtn = toolbarButton('ui/play', 'Present', startPresent);

  // Single top bar (Studio-style): deck menu + title + theme + action buttons.
  bar.append(
    deckMenuBtn,
    titleInput,
    themeSelect,
    toolbarButton('ui/plus', 'New presentation', () => void newDeck()),
    toolbarButton('ui/download', 'Import .odp', pickOdpFile),
    toolbarButton('ui/upload', 'Export .odp', () => void exportOdp()),
    toolbarButton('ui/save', 'Save now', () => void persist()),
    toolbarButton('ui/trash', 'Delete presentation', () => void removeCurrent(), true),
    presentBtn,
    saveDot,
  );
  tileEl.appendChild(bar);

  /* Body: strip + stage + inspector */
  const body = h('div', 'impress-body');

  stripEl = h('div', 'impress-strip');

  const stageWrap = h('div', 'impress-stage-wrap');
  stageWrapEl = stageWrap;
  stageEl = h('div', 'impress-stage');
  stageEl.appendChild(emptyState({ title: 'No presentation', body: 'Create one, or ask the AI to build a deck' }));
  stageWrap.appendChild(stageEl);

  const inspector = buildInspector();
  inspectorEl = inspector;
  stageWrap.appendChild(inspector);

  body.append(stripEl, stageWrap);
  tileEl.appendChild(body);

  /* Status line */
  const status = h('div', 'impress-status');
  statusEl = h('span');
  status.appendChild(statusEl);
  tileEl.appendChild(status);

  // Keyboard: left/right move slides (ignore when typing in a field). While
  // presenting, the overlay owns the keyboard (it lives on <body>).
  tileEl.addEventListener('keydown', (e) => {
    if (presentIndex >= 0) return;
    const active = document.activeElement;
    const tag = (active?.tagName || '').toLowerCase();
    if (['input', 'textarea', 'select'].includes(tag) || active?.isContentEditable) return;
    if (e.key === 'ArrowRight') { e.preventDefault(); selectSlide(selIndex + 1); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); selectSlide(selIndex - 1); }
  });

  observeStageSize();
  void openNewest();
  updateGlow();
  return tileEl;
}

function buildInspector() {
  const panel = h('div', 'impress-inspector');

  const layoutField = field('Layout', () => {
    layoutSelect = document.createElement('select');
    layoutSelect.className = 'impress-field-input';
    for (const l of LAYOUTS) {
      const opt = document.createElement('option');
      opt.value = l;
      opt.textContent = l;
      layoutSelect.appendChild(opt);
    }
    layoutSelect.addEventListener('change', () => {
      if (!current) return;
      current.slides[selIndex].layout = layoutSelect.value;
      markDirty();
      renderStage();
      renderStrip();
    });
    return layoutSelect;
  });
  panel.appendChild(layoutField);

  // Animation: transition on entry + bullet build, both per slide.
  const animation = h('div', 'impress-anim');
  const animField = (label, options, labels, onChange) => {
    const wrap = h('div', 'impress-field');
    wrap.appendChild(h('label', 'impress-field-label', label));
    const select = document.createElement('select');
    select.className = 'impress-field-input';
    for (const value of options) {
      const opt = document.createElement('option');
      opt.value = value;
      opt.textContent = labels[value] || value;
      select.appendChild(opt);
    }
    select.addEventListener('change', () => onChange(select.value));
    wrap.appendChild(select);
    return { wrap, select };
  };

  const transitionField = animField('Transition', TRANSITIONS, TRANSITION_LABELS, (value) => {
    if (!current) return;
    current.slides[selIndex].transition = value;
    markDirty();
  });
  transitionSelect = transitionField.select;
  animation.appendChild(transitionField.wrap);

  const revealField = animField('Bullets', REVEALS, REVEAL_LABELS, (value) => {
    if (!current) return;
    current.slides[selIndex].reveal = value;
    markDirty();
  });
  revealSelect = revealField.select;
  animation.appendChild(revealField.wrap);

  // Applying the current slide's animation to the whole deck is the common
  // case — a per-slide-only editor would mean 20 edits for a 20-slide deck.
  const applyAll = button({
    icon: 'ui/loop',
    variant: 'ghost',
    onClick: () => applyAnimationToAll(),
  });
  applyAll.classList.add('ui-btn--icon', 'impress-anim-apply');
  applyAll.title = 'Apply this transition and bullet build to every slide';
  applyAll.setAttribute('aria-label', 'Apply to all slides');
  animation.appendChild(applyAll);

  panel.appendChild(animation);

  // Speaker notes are the one field that has no on-slide home.
  fieldNotes = buildField('Speaker notes', 'textarea');
  const notesInput = fieldNotes.querySelector('textarea');
  notesInput.rows = 3;
  notesInput.style.resize = 'none';
  notesInput.addEventListener('input', () => {
    if (!current) return;
    current.slides[selIndex].notes = notesInput.value;
    markDirty();
  });
  panel.appendChild(fieldNotes);

  // Hint + slide operations.
  panel.appendChild(h('p', 'impress-hint', 'Click any text on the slide to edit it directly.'));

  const ops = h('div', 'impress-ops');
  const opBtn = (iconName, label, onClick, danger = false) => {
    const btn = button({ icon: iconName, variant: 'ghost', onClick });
    btn.classList.add('ui-btn--icon', 'impress-op');
    if (danger) btn.classList.add('impress-op--danger');
    btn.title = label;
    btn.setAttribute('aria-label', label);
    return btn;
  };
  ops.append(
    opBtn('ui/plus', 'Add slide', addSlide),
    opBtn('ui/chevron-left', 'Move up', () => moveSlide(-1)),
    opBtn('ui/chevron-right', 'Move down', () => moveSlide(1)),
    opBtn('ui/trash', 'Delete slide', removeSlide, true),
  );
  panel.appendChild(ops);

  return panel;
}

/** Build a labelled form field (used for the layout selector + notes). */
function buildField(label, kind) {
  const wrap = h('div', 'impress-field');
  const lbl = h('label', 'impress-field-label', label);
  const el = document.createElement(kind === 'textarea' ? 'textarea' : 'input');
  el.className = 'impress-field-input';
  if (kind === 'input') el.type = 'text';
  else el.rows = 4;
  el.autocomplete = 'off';
  wrap.append(lbl, el);
  return wrap;
}

function field(label, make) {
  const wrap = h('div', 'impress-field');
  const lbl = h('label', 'impress-field-label', label);
  wrap.append(lbl, make());
  return wrap;
}

function addSlide() {
  if (!current) return;
  current.slides.push(newSlide('content'));
  selIndex = current.slides.length - 1;
  markDirty();
  renderStrip();
  renderStage();
  renderInspector();
}

function removeSlide() {
  if (!current) return;
  if (current.slides.length <= 1) {
    current.slides = [newSlide('title')];
  } else {
    current.slides.splice(selIndex, 1);
  }
  selIndex = Math.min(selIndex, current.slides.length - 1);
  markDirty();
  renderStrip();
  renderStage();
  renderInspector();
}

function moveSlide(delta) {
  if (!current) return;
  const to = selIndex + delta;
  if (to < 0 || to >= current.slides.length) return;
  const arr = current.slides;
  [arr[selIndex], arr[to]] = [arr[to], arr[selIndex]];
  selIndex = to;
  markDirty();
  renderStrip();
}

/** Deactivated mid-edit: flush, drop the window. */
export function unmountImpressTile() {
  if (current && dirty) void persist();
  stopPresent();
  presentEl?.remove();
  document.removeEventListener('fullscreenchange', onPresentViewportChange);
  window.removeEventListener('resize', onPresentViewportChange);
  window.clearTimeout(thumbRefreshTimer);
  resizeObserver?.disconnect();
  tileEl?.remove();
  tileEl = null;
  stripEl = null;
  stageEl = null;
  presentEl = null;
  stageWrapEl = null;
  inspectorEl = null;
  stageSlideNode = null;
  resizeObserver = null;
  deckMenuBtn = null;
  presentBtn = null;
  presentPressFromOverlay = false;
  titleInput = null;
  themeSelect = null;
  saveDot = null;
  statusEl = null;
  layoutSelect = null;
  transitionSelect = null;
  revealSelect = null;
  fieldNotes = null;
  presentStep = 0;
  presentFrags = [];
  presentFullBtn = null;
  thumbRefreshTimer = null;
}

/** The tile element (or null when the Impress window is not mounted). */
export function getImpressTileElement() {
  return tileEl;
}

/* ── AI wiring ──────────────────────────────────────────────── */

function onAgentActions(e) {
  const actions = e.detail || [];
  const slideActions = actions.filter((a) => /^slide_/.test(a?.action || ''));
  if (!slideActions.length) return;

  // Always surface the Impress window when the AI touches presentations.
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: IMPRESS_PLUGIN } }));

  const created = slideActions.some((a) => a.action === 'slide_create' && a.result === 'ok');
  const wrote = slideActions.some((a) => /^slide_(write|edit)$/.test(a.action) && a.result === 'ok');
  const read = slideActions.some((a) => a.action === 'slide_read' && a.result === 'ok');
  const deleted = slideActions.some((a) => a.action === 'slide_delete' && a.result === 'ok');

  const touchedId = slideActions
    .map((a) => a.data?.deck_id)
    .find((id) => !!id);

  if (created) {
    void refreshDecks().then(() => window.setTimeout(() => {
      if (dirty) return; // never clobber the user's unsaved edits
      if (touchedId) {
        const found = decks.find((d) => d.id === touchedId);
        if (found) void openDeck(found);
      } else {
        void openNewest();
      }
    }, 250));
  } else if (wrote || read || deleted) {
    void (async () => {
      // Do NOT persist a stale local deck over an AI write to the SAME deck —
      // that would clobber the AI's changes with the user's old slides.
      if (dirty && !(wrote && touchedId === current?.id)) await persist();
      await refreshDecks();
      if (touchedId && current?.id !== touchedId) {
        const found = decks.find((d) => d.id === touchedId);
        if (found) void openDeck(found);
      } else if (wrote && current) {
        const full = decks.find((d) => d.id === current.id);
        if (full) void openDeck(full);
      } else if (deleted) {
        void openNewest();
      }
    })();
  }
}

let wired = false;
export function wireImpressEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  window.addEventListener('beforeunload', (e) => {
    if (current && dirty) {
      e.preventDefault();
      e.returnValue = '';
    }
  });
}

export default {
  name: 'impress',
  icon: 'ui/present',
  mount: mountImpressTile,
  unmount: unmountImpressTile,
  getElement: getImpressTileElement,
  wireEvents: wireImpressEvents,
};
