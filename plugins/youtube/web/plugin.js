import {
  mediaCard, controlButton, button, chip, searchBar, emptyState, spinner, setTileGlow, glowUrl,
  glowFromImageUrl, metaLine, listReveal, cardEnter, decodeText,
} from '../../ui/index.js';
/**
 * youtube.js — the YouTube plugin's window.
 *
 * Editorial surface: ambient thumbnail glow behind everything, a hero (the
 * embedded player, a large display title and a mono "channel · duration"
 * meta line, shown only while a video plays), a flat search pill (in-tile,
 * works with the keyboard plugin), category chips and a 16:9 thumbnail grid
 * captioned below in body + mono meta, under a mono uppercase label.
 *
 * The grid has three sources, all rendered the same way:
 *   • the in-tile search      → GET  /api/youtube/search
 *   • AI search results       → `artifact:saved` cards
 *   • "Up next" suggestions   → GET  /api/youtube/suggest   (Rust ranker)
 *
 * Playing a video swaps the grid to its suggestions; the back button restores
 * whatever the grid showed before (`lastResults`).
 */
import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';

export const YOUTUBE_PLUGIN = 'youtube';
const BG_KEY = 'youtube.background';

let tileEl = null;
let heroEl = null;       // hero container (idle view or player + info)
let playerWrapEl = null; // embed player wrapper (16:9)
let frameEl = null;      // embed iframe
let infoEl = null;       // playing info row (title + channel + back)
let infoTitleEl = null;
let infoSubEl = null;
let gridEl = null;
let gridLabelEl = null;  // "Up next" / results caption above the grid
let gridLabelTextEl = null;
let gridLabelCountEl = null;
let categoriesEl = null; // idle-homepage category chips
let searchEl = null;

let current = null;      // { video_id, title, channel, thumbnail }
let aiResults = [];      // results pushed by the AI (artifact:saved)
let lastResults = [];    // last search/AI list, restored by the back button
let suggestSeq = 0;      // guards against out-of-order suggestion responses
let tileObserver = null; // keeps the player from overflowing the window
let bgCss = null;        // remembered, lightly-blurred window background
let bgToken = 0;         // guards against out-of-order background loads
let wired = false;

function thumbFor(videoId) {
  return `https://i.ytimg.com/vi/${videoId}/hqdefault.jpg`;
}

/** Accept `{ data: { results: [...] } }`, `{ data: [...] }` and `[...]`.
 *  (The search route used to return a bare array while the UI read
 *  `data.results` — the cause of the "no videos found" bug.) */
function resultList(res) {
  const d = res?.data ?? res;
  if (Array.isArray(d)) return d;
  return Array.isArray(d?.results) ? d.results : [];
}

/* ── Hero ─────────────────────────────────────────────────────── */

function renderHero() {
  if (!tileEl) return;
  const playing = !!current;

  // The hero only exists while a video plays (player + info). With nothing
  // playing there is no hero at all — the search bar sits directly under the
  // window title bar, and the brand mark/name are not repeated.
  heroEl?.classList.toggle('hidden', !playing);
  playerWrapEl?.classList.toggle('hidden', !playing);
  infoEl?.classList.toggle('hidden', !playing);
  frameEl?.classList.toggle('hidden', !playing);

  if (playing) {
    if (infoTitleEl) {
      infoTitleEl.textContent = current.title || t('plugin.youtube.hero.nowPlaying', null, 'Now playing');
      infoTitleEl.title = current.title || '';
    }
    infoSubEl?.set([current.channel || 'YouTube', current.duration]);
  }

  // The window keeps the last video's lightly-blurred image as its ambient
  // background (remembered across playback and reloads); before any video it
  // falls back to the window's Tier 0 colour glow.
  applyBackground();

  renderGridCurrent();
}

/** Paint the remembered background — null clears to the Tier 0 colour glow. */
function applyBackground() {
  setTileGlow(tileEl, bgCss);
}

/** Build and remember a lightly-blurred background from a video's thumbnail.
 *  The blur is baked once at thumbnail size (glowFromImageUrl), so painting
 *  the window costs nothing per frame and the picture stays recognisable. */
async function setBackground(v) {
  if (!v?.thumbnail) return;
  const token = ++bgToken;
  const css = await glowFromImageUrl(v.thumbnail, { size: 320, blur: 7 });
  if (token !== bgToken) return; // a newer video won
  bgCss = css || glowUrl(v.thumbnail); // fall back to the raw thumbnail
  try { localStorage.setItem(BG_KEY, bgCss); } catch (_) { /* full / blocked */ }
  applyBackground();
}

function playVideo(v) {
  if (!v?.video_id) return;
  // Focus FIRST: this also mounts the tile when it isn't up yet. The old
  // order bailed on `!tileEl` before focus, so an agent "play" with the
  // window closed silently did nothing.
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: YOUTUBE_PLUGIN } }));
  if (!tileEl || !frameEl) return; // focus mounts synchronously
  current = {
    video_id: v.video_id,
    title: v.title || t('plugin.youtube.hero.untitled', null, 'YouTube video'),
    channel: v.channel || '',
    duration: v.duration || '',
    thumbnail: v.thumbnail || thumbFor(v.video_id),
  };
  frameEl.src = `https://www.youtube.com/embed/${v.video_id}?autoplay=1&rel=0`;
  setCategoriesVisible(false);
  renderHero();
  void cardEnter(heroEl);
  void setBackground(current);
  void loadSuggestions(current);
}

/** Swap the grid to "Up next" suggestions for the video now playing.
 *  The Rust ranker keys off keywords + channel + watch history; asking also
 *  records the watch. Failures (older server, offline) leave the grid alone. */
async function loadSuggestions(v) {
  if (!v?.video_id) return;
  const seq = ++suggestSeq;
  const params = new URLSearchParams({
    video_id: v.video_id,
    title: v.title || '',
    channel: v.channel || '',
    limit: '12',
  });
  try {
    const res = await apiFetch(`/api/youtube/suggest?${params}`);
    if (seq !== suggestSeq || !gridEl) return; // a newer action won
    const list = resultList(res);
    if (!list.length) return;
    const based = res?.data?.based_on?.title;
    renderGrid(list, null, {
      label: based && based !== v.title
        ? t('plugin.youtube.grid.becauseYouWatched', { title: based }, `Up next · because you watched “${based}”`)
        : t('plugin.youtube.grid.upNext', null, 'Up next'),
    });
  } catch (_) { /* suggestions are optional — keep the current grid */ }
}

function reset() {
  current = null;
  suggestSeq++; // drop any in-flight suggestions render
  if (frameEl) frameEl.src = 'about:blank';
  renderHero();
  // Back to whatever the grid held before playback, or the homepage.
  if (lastResults.length) renderGrid(lastResults, null);
  else void loadHomepage();
}

/* ── Result grid ──────────────────────────────────────────────── */

function renderGridCurrent() {
  if (!gridEl) return;
  gridEl.querySelectorAll('.yt-cell').forEach((cell) => {
    cell.classList.toggle('yt-cell--current', !!current && cell.dataset.videoId === current.video_id);
  });
}

function videoCell(v, idx) {
  const cell = mediaCard({ title: v.title || t('plugin.youtube.grid.untitled', null, 'Video'),
    subtitle: v.channel, src: v.thumbnail || thumbFor(v.video_id), position: idx + 1, duration: v.duration,
    className: 'yt-cell', prefix: 'yt-cell', onClick: () => playVideo(v) });
  cell.dataset.videoId = v.video_id || '';
  cell.classList.toggle('yt-cell--current', current?.video_id === v.video_id);
  return cell;
}

/** Caption above the grid ("Up next", …) with a mono count; hidden when
 *  empty. A changed label decodes in. */
function setGridLabel(label, count = null) {
  if (!gridLabelEl) return;
  const text = label || '';
  if (gridLabelTextEl.dataset.value !== text) {
    gridLabelTextEl.dataset.value = text;
    if (text) decodeText(gridLabelTextEl, text);
    else gridLabelTextEl.textContent = '';
  }
  gridLabelCountEl.textContent = count ? String(count).padStart(2, '0') : '';
  gridLabelEl.title = text;
  gridLabelEl.classList.toggle('hidden', !text);
}

function renderGrid(results, term, { label = '' } = {}) {
  if (!gridEl) return;
  setGridLabel(label, results?.length || null);
  gridEl.classList.remove('yt-grid--shelves');
  gridEl.innerHTML = '';
  if (!results) {
    const wrap = document.createElement('div');
    wrap.className = 'yt-grid-status';
    wrap.appendChild(spinner());
    gridEl.appendChild(wrap);
    return;
  }
  if (!results.length) {
    const wrap = document.createElement('div');
    wrap.className = 'yt-grid-status';
    wrap.appendChild(term
      ? emptyState({
        icon: 'ui/search',
        title: t('plugin.youtube.empty.noResults', null, 'No videos found'),
        body: t('plugin.youtube.empty.noMatch', { term }, `Nothing matched “${term}”.`),
      })
      : emptyState({
        icon: 'ui/youtube',
        title: t('plugin.youtube.empty.searchTitle', null, 'Search YouTube'),
        body: t('plugin.youtube.empty.searchBody', null, 'Type above — or ask the AI to find a video.'),
      }));
    gridEl.appendChild(wrap);
    return;
  }
  const cells = results.map((v, i) => videoCell(v, i));
  gridEl.append(...cells);
  void listReveal(cells);
}

async function runSearch(text) {
  const term = (text || '').trim();
  if (!term) {
    lastResults = [];
    void loadHomepage(); // back to categories + "For you"
    return;
  }
  setCategoriesVisible(false);
  renderGrid(null, null);
  try {
    const res = await apiFetch(`/api/youtube/search?q=${encodeURIComponent(term)}`);
    const list = resultList(res);
    lastResults = list;
    renderGrid(list, term);
  } catch (e) {
    if (!gridEl) return;
    gridEl.innerHTML = '';
    const wrap = document.createElement('div');
    wrap.className = 'yt-grid-status';
    wrap.appendChild(emptyState({
      icon: 'ui/warning',
      title: t('plugin.youtube.empty.unavailableTitle', null, 'Search unavailable'),
      body: e.message || t('plugin.youtube.empty.retry', null, 'Try again in a moment'),
    }));
    gridEl.appendChild(wrap);
  }
}

/* ── Idle homepage: category chips + "For you" ──────────────── */

const SMALL_WORDS = new Set([
  'a', 'an', 'and', 'the', 'of', 'in', 'on', 'for', 'to', 'vs', 'or',
  'e', 'y', 'de', 'la', 'el', 'di', 'da',
]);
const ACRONYMS = new Set(['ai', 'vr', 'ar', '3d', '2d', '4k', '8k', 'os', 'tv', 'uk', 'us', 'diy', 'rpg', 'pc', 'gpu', 'cpu', 'api', 'ml']);

function titleCase(s) {
  const words = String(s || '').trim().split(/\s+/).filter(Boolean);
  return words
    .map((w, i) => {
      const lower = w.toLowerCase();
      if (ACRONYMS.has(lower)) return lower.toUpperCase();
      if (i > 0 && i < words.length - 1 && SMALL_WORDS.has(lower)) return lower;
      return lower.charAt(0).toUpperCase() + lower.slice(1);
    })
    .join(' ');
}

function setCategoriesVisible(on) {
  categoriesEl?.classList.toggle('hidden', !on);
}

/** Chips for the categories the user watches most, most-watched first. */
function renderCategories(cats) {
  if (!categoriesEl) return;
  categoriesEl.innerHTML = '';
  const list = Array.isArray(cats) ? cats.slice(0, 20) : [];
  setCategoriesVisible(list.length > 0 && !current && !searchEl?.input?.value?.trim());
  for (const c of list) {
    const name = typeof c === 'string' ? c : c?.name;
    if (!name) continue;
    const el = chip({ label: titleCase(name), onClick: () => void openCategory(name) });
    el.classList.add('yt-category');
    el.title = c?.count
      ? t('plugin.youtube.category.watched', { count: c.count }, `Watched ${c.count}×`)
      : t('plugin.youtube.category.generic', null, 'Category');
    categoriesEl.appendChild(el);
  }
}

/** The idle homepage: a shelf of videos per category, most-watched first. */
async function loadHomepage() {
  if (!tileEl || current) return;
  renderGrid(null, null, { label: t('plugin.youtube.grid.forYou', null, 'For you') }); // spinner while shelves load
  const [cats, home] = await Promise.all([
    apiFetch('/api/youtube/categories?limit=20').catch(() => null),
    apiFetch('/api/youtube/home?per_category=12&max_categories=20').catch(() => null),
  ]);
  if (!tileEl || current) return;
  renderCategories(cats?.data?.categories || []);
  const sections = home?.data?.sections || [];
  if (sections.length) {
    renderHome(sections);
  } else {
    const list = resultList(home);
    renderGrid(list, null, { label: t('plugin.youtube.grid.forYou', null, 'For you') });
  }
}

/** Category shelves — a clickable label plus one row of videos per category. */
function renderHome(sections) {
  if (!gridEl) return;
  setGridLabel(t('plugin.youtube.grid.forYou', null, 'For you'));
  gridEl.classList.add('yt-grid--shelves');
  gridEl.innerHTML = '';
  const shelves = [];
  for (const section of sections) {
    const videos = Array.isArray(section?.videos) ? section.videos : [];
    if (!videos.length) continue;
    const sec = document.createElement('section');
    sec.className = 'yt-section';

    const head = controlButton();
    head.type = 'button';
    head.className = 'yt-section-title';
    const headName = document.createElement('span');
    headName.className = 'yt-section-name';
    headName.textContent = titleCase(section.category);
    const headCount = document.createElement('span');
    headCount.className = 'yt-section-count';
    headCount.textContent = String(videos.length).padStart(2, '0');
    head.append(headName, headCount);
    head.title = t('plugin.youtube.grid.showAll', { category: titleCase(section.category) },
      `Show all ${titleCase(section.category)}`);
    head.addEventListener('click', () => void openCategory(section.category));

    const row = document.createElement('div');
    row.className = 'yt-shelf';
    videos.forEach((v, i) => row.appendChild(videoCell(v, i)));

    sec.append(head, row);
    gridEl.appendChild(sec);
    shelves.push(sec);
  }
  void listReveal(shelves);
}

/** A category chip: rank videos for that topic and show them in the grid. */
async function openCategory(name) {
  if (!name) return;
  if (searchEl?.input) searchEl.input.value = name;
  setCategoriesVisible(false);
  renderGrid(null, null);
  try {
    const res = await apiFetch(
      `/api/youtube/suggest?${new URLSearchParams({ query: name, limit: '16' })}`,
    );
    const list = resultList(res);
    lastResults = list;
    renderGrid(list, null, { label: titleCase(name) });
  } catch (_) {
    void runSearch(name); // older server: fall back to a plain search
  }
}

/* ── AI wiring ────────────────────────────────────────────────── */

function onAgentActions(e) {
  for (const action of e.detail || []) {
    if (action?.action === 'youtube_play' && action?.result === 'ok') {
      const id = action?.data?.video_id;
      if (id) {
        playVideo({
          video_id: id,
          title: action?.data?.title,
          channel: action?.data?.channel,
          thumbnail: action?.data?.thumbnail,
        });
      }
    }
  }
}

function onArtifactAction(e) {
  const { action } = e.detail || {};
  if (action?.tool !== 'youtube_play') return;
  const id = action?.params?.video_id;
  if (id) playVideo({ video_id: id, title: action?.params?.title });
}

/** AI searches land here — results become the window's own grid (radio-style). */
function onArtifactSaved(e) {
  const art = e.detail;
  if (art?.type !== 'youtube_video' && art?.plugin !== 'youtube') return;
  const play = art?.actions?.find((a) => a.tool === 'youtube_play');
  const id = play?.params?.video_id;
  if (!id) return;
  const video = {
    video_id: id,
    title: play?.params?.title || art.title || t('plugin.youtube.grid.untitled', null, 'Video'),
    channel: art.subtitle?.split('·')[0]?.trim() || '',
    duration: art.subtitle?.split('·')[1]?.trim() || '',
    thumbnail: play?.params?.thumbnail || thumbFor(id),
  };
  // Merge, newest first, deduped — then re-render the grid.
  aiResults = [video, ...aiResults.filter((v) => v.video_id !== id)].slice(0, 24);
  lastResults = aiResults;
  setCategoriesVisible(false);
  renderGrid(aiResults, null);
}

export function wireYoutubeEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  window.addEventListener('artifact:action', onArtifactAction);
  window.addEventListener('artifact:saved', onArtifactSaved);
}

/* ── Tile lifecycle ───────────────────────────────────────────── */

/** Create the YouTube tile element (the plugin's window container). */
export function mountYoutubeTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile yt-tile';
  tileEl.dataset.plugin = YOUTUBE_PLUGIN;

  /* Hero */
  heroEl = document.createElement('div');
  heroEl.className = 'yt-hero hidden';

  // Player + info row.
  playerWrapEl = document.createElement('div');
  playerWrapEl.className = 'yt-player hidden';
  frameEl = document.createElement('iframe');
  frameEl.className = 'yt-frame hidden';
  frameEl.setAttribute('allow', 'autoplay; encrypted-media; fullscreen; picture-in-picture');
  frameEl.setAttribute('allowfullscreen', 'true');
  frameEl.setAttribute('referrerpolicy', 'strict-origin-when-cross-origin');
  frameEl.setAttribute('title', t('plugin.youtube.player.title', null, 'YouTube player'));
  playerWrapEl.appendChild(frameEl);

  infoEl = document.createElement('div');
  infoEl.className = 'yt-hero-info hidden';
  const infoText = document.createElement('div');
  infoText.className = 'yt-hero-text';
  infoTitleEl = document.createElement('div');
  infoTitleEl.className = 'yt-hero-info-title';
  infoSubEl = metaLine([], 'yt-hero-sub');
  infoText.append(infoTitleEl, infoSubEl);
  // Back to the grid: a grey pill led by an arrow, not a close cross.
  const backBtn = button({
    label: t('plugin.youtube.hero.back', null, 'Back'),
    icon: 'ui/arrow-left',
    iconSize: 15,
    variant: 'ghost',
    size: 'sm',
    className: 'yt-hero-back',
    onClick: reset,
  });
  backBtn.setAttribute('aria-label', t('plugin.youtube.hero.backLabel', null, 'Back to YouTube search'));
  infoEl.append(infoText, backBtn);

  heroEl.append(playerWrapEl, infoEl);
  tileEl.appendChild(heroEl);

  /* Search */
  searchEl = searchBar({ placeholder: t('plugin.youtube.search.placeholder', null, 'Search YouTube…') });
  let searchTimer = null;
  searchEl.input.addEventListener('input', (e) => {
    window.clearTimeout(searchTimer);
    const v = e.target.value;
    setCategoriesVisible(v.trim() === '' && !current);
    searchTimer = window.setTimeout(() => void runSearch(v), 350);
  });
  tileEl.appendChild(searchEl);

  /* Category chips (idle homepage) */
  categoriesEl = document.createElement('div');
  categoriesEl.className = 'yt-categories hidden';
  tileEl.appendChild(categoriesEl);

  /* Grid */
  gridLabelEl = document.createElement('div');
  gridLabelEl.className = 'yt-grid-label hidden';
  gridLabelTextEl = document.createElement('span');
  gridLabelTextEl.className = 'yt-grid-label-text';
  gridLabelCountEl = document.createElement('span');
  gridLabelCountEl.className = 'yt-grid-label-count';
  gridLabelEl.append(gridLabelTextEl, gridLabelCountEl);
  tileEl.appendChild(gridLabelEl);
  gridEl = document.createElement('div');
  gridEl.className = 'yt-grid';
  tileEl.appendChild(gridEl);

  // Cap the 16:9 player at ~half the window height, otherwise a wide window
  // makes it taller than the tile and pushes the search bar and the results /
  // "Up next" grid out of view. Recomputed whenever the window resizes.
  const fitPlayer = () => {
    const h = tileEl?.clientHeight || 0;
    if (h) {
      tileEl.style.setProperty('--yt-player-max', `${Math.max(160, Math.round(h * 0.6))}px`);
    }
  };
  tileObserver = new ResizeObserver(fitPlayer);
  tileObserver.observe(tileEl);
  fitPlayer();

  // Restore the remembered background from the last session.
  try {
    const saved = localStorage.getItem(BG_KEY);
    if (saved) bgCss = saved;
  } catch (_) { /* ignore */ }

  renderHero();
  renderGrid([], null);
  void loadHomepage();
  return tileEl;
}

/** Deactivated mid-playback: stop the video, drop the window. */
export function unmountYoutubeTile() {
  reset();
  tileObserver?.disconnect();
  tileObserver = null;
  tileEl?.remove();
  tileEl = null;
  heroEl = null;
  playerWrapEl = null;
  frameEl = null;
  infoEl = null;
  infoTitleEl = null;
  infoSubEl = null;
  gridEl = null;
  gridLabelEl = null;
  gridLabelTextEl = null;
  gridLabelCountEl = null;
  categoriesEl = null;
  searchEl = null;
}

/** The tile element (or null when the YouTube window is not mounted). */
export function getYoutubeTileElement() {
  return tileEl;
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators + window management. */
export function youtubeContextMenu() {
  const playing = !!current;
  return [
    { type: 'item', label: t('plugin.youtube.menu.search', null, 'Search'), icon: 'ui/search', disabled: !searchEl, onClick: () => searchEl?.input?.focus() },
    { type: 'item', label: t('plugin.youtube.menu.home', null, 'Home'), icon: 'ui/youtube', disabled: !tileEl || playing, onClick: () => void loadHomepage() },
    { type: 'item', label: t('plugin.youtube.menu.refresh', null, 'Refresh results'), icon: 'ui/refresh', disabled: !tileEl, onClick: () => void runSearch(searchEl?.input?.value || '') },
    { type: 'separator' },
    { type: 'item', label: t('plugin.youtube.menu.back', null, 'Back'), icon: 'ui/arrow-left', disabled: !playing, onClick: () => reset() },
  ];
}

export default {
  name: 'youtube',
  icon: 'ui/youtube',
  mount: mountYoutubeTile,
  unmount: unmountYoutubeTile,
  getElement: getYoutubeTileElement,
  wireEvents: wireYoutubeEvents,
  contextMenu: youtubeContextMenu,
};
