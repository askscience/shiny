/**
 * youtube.js — the YouTube plugin's window.
 *
 * Radio-style flat surface: ambient thumbnail glow behind everything, a hero
 * (idle brand mark, or the embedded player while a video plays), a search bar
 * (in-tile, works with the keyboard plugin) and a 16:9 thumbnail grid below.
 *
 * The grid has three sources, all rendered the same way:
 *   • the in-tile search      → GET  /api/youtube/search
 *   • AI search results       → `artifact:saved` cards
 *   • "Up next" suggestions   → GET  /api/youtube/suggest   (Rust ranker)
 *
 * Playing a video swaps the grid to its suggestions; the back button restores
 * whatever the grid showed before (`lastResults`).
 */
import { apiFetch } from '/js/api.js';
import {
  setIcon, searchBar, emptyState, spinner, setTileGlow, glowUrl, glowFromImageUrl,
} from '/ui/index.js';

export const YOUTUBE_PLUGIN = 'youtube';
const BG_KEY = 'youtube.background';

let tileEl = null;
let heroEl = null;       // hero container (idle view or player + info)
let playerWrapEl = null; // embed player wrapper (16:9)
let frameEl = null;      // embed iframe
let infoEl = null;       // playing info row (title + channel + back)
let infoTitleEl = null;
let infoSubEl = null;
let idleEl = null;       // idle hero view
let gridEl = null;
let gridLabelEl = null;  // "Up next" / results caption above the grid
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

  idleEl?.classList.toggle('hidden', playing);
  playerWrapEl?.classList.toggle('hidden', !playing);
  infoEl?.classList.toggle('hidden', !playing);
  frameEl?.classList.toggle('hidden', !playing);

  if (playing) {
    if (infoTitleEl) {
      infoTitleEl.textContent = current.title || 'Now playing';
      infoTitleEl.title = current.title || '';
    }
    if (infoSubEl) infoSubEl.textContent = current.channel || 'YouTube';
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
    title: v.title || 'YouTube video',
    channel: v.channel || '',
    thumbnail: v.thumbnail || thumbFor(v.video_id),
  };
  frameEl.src = `https://www.youtube.com/embed/${v.video_id}?autoplay=1&rel=0`;
  setCategoriesVisible(false);
  renderHero();
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
      label: based && based !== v.title ? `Up next · because you watched “${based}”` : 'Up next',
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
  // div[role=button], not <button>: button elements collapse their content
  // contribution when the grid scrolls, shrinking cells under the art.
  const cell = document.createElement('div');
  cell.className = 'yt-cell';
  cell.dataset.videoId = v.video_id || '';
  cell.setAttribute('role', 'button');
  cell.tabIndex = 0;
  if (current?.video_id === v.video_id) cell.classList.add('yt-cell--current');

  const num = document.createElement('span');
  num.className = 'yt-cell-num';
  num.textContent = String(idx + 1).padStart(2, '0');

  const art = document.createElement('span');
  art.className = 'yt-cell-art';
  const img = document.createElement('img');
  img.src = v.thumbnail || thumbFor(v.video_id);
  img.alt = '';
  img.loading = 'lazy';
  img.onerror = () => img.remove();
  art.appendChild(img);

  if (v.duration) {
    const dur = document.createElement('span');
    dur.className = 'yt-cell-dur';
    dur.textContent = v.duration;
    art.appendChild(dur);
  }

  const meta = document.createElement('span');
  meta.className = 'yt-cell-meta';
  const name = document.createElement('span');
  name.className = 'yt-cell-name';
  name.textContent = v.title || 'Video';
  meta.appendChild(name);
  if (v.channel) {
    const channel = document.createElement('span');
    channel.className = 'yt-cell-channel';
    channel.textContent = v.channel;
    meta.appendChild(channel);
  }

  cell.append(num, art, meta);
  cell.addEventListener('click', () => playVideo(v));
  cell.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      playVideo(v);
    }
  });
  return cell;
}

/** Caption above the grid ("Up next", …); hidden when empty. */
function setGridLabel(label) {
  if (!gridLabelEl) return;
  gridLabelEl.textContent = label || '';
  gridLabelEl.title = label || '';
  gridLabelEl.classList.toggle('hidden', !label);
}

function renderGrid(results, term, { label = '' } = {}) {
  if (!gridEl) return;
  setGridLabel(label);
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
      ? emptyState({ icon: 'ui/search', title: 'No videos found', body: `Nothing matched “${term}”.` })
      : emptyState({ icon: 'ui/youtube', title: 'Search YouTube', body: 'Type above — or ask the AI to find a video.' }));
    gridEl.appendChild(wrap);
    return;
  }
  results.forEach((v, i) => gridEl.appendChild(videoCell(v, i)));
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
      title: 'Search unavailable',
      body: e.message || 'Try again in a moment',
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
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'yt-category';
    chip.textContent = titleCase(name);
    chip.title = c?.count ? `Watched ${c.count}×` : 'Category';
    chip.addEventListener('click', () => void openCategory(name));
    categoriesEl.appendChild(chip);
  }
}

/** The idle homepage: a shelf of videos per category, most-watched first. */
async function loadHomepage() {
  if (!tileEl || current) return;
  renderGrid(null, null, { label: 'For you' }); // spinner while shelves load
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
    renderGrid(list, null, { label: 'For you' });
  }
}

/** Category shelves — a clickable label plus one row of videos per category. */
function renderHome(sections) {
  if (!gridEl) return;
  setGridLabel('For you');
  gridEl.classList.add('yt-grid--shelves');
  gridEl.innerHTML = '';
  for (const section of sections) {
    const videos = Array.isArray(section?.videos) ? section.videos : [];
    if (!videos.length) continue;
    const sec = document.createElement('section');
    sec.className = 'yt-section';

    const head = document.createElement('button');
    head.type = 'button';
    head.className = 'yt-section-title';
    head.textContent = `${titleCase(section.category)} · ${videos.length}`;
    head.title = `Show all ${titleCase(section.category)}`;
    head.addEventListener('click', () => void openCategory(section.category));

    const row = document.createElement('div');
    row.className = 'yt-shelf';
    videos.forEach((v, i) => row.appendChild(videoCell(v, i)));

    sec.append(head, row);
    gridEl.appendChild(sec);
  }
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
    title: play?.params?.title || art.title || 'Video',
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
  heroEl.className = 'yt-hero';

  // Idle view: brand mark + title.
  idleEl = document.createElement('div');
  idleEl.className = 'yt-hero-idle';
  const mark = document.createElement('span');
  mark.className = 'yt-hero-mark';
  void setIcon(mark, 'ui/youtube', { size: 30 });
  const idleText = document.createElement('div');
  idleText.className = 'yt-hero-text';
  const h1 = document.createElement('div');
  h1.className = 'yt-hero-title';
  h1.textContent = 'YouTube';
  const sub = document.createElement('div');
  sub.className = 'yt-hero-sub';
  sub.textContent = 'Search videos — or ask the AI';
  idleText.append(h1, sub);
  idleEl.append(mark, idleText);

  // Player + info row.
  playerWrapEl = document.createElement('div');
  playerWrapEl.className = 'yt-player hidden';
  frameEl = document.createElement('iframe');
  frameEl.className = 'yt-frame hidden';
  frameEl.setAttribute('allow', 'autoplay; encrypted-media; fullscreen; picture-in-picture');
  frameEl.setAttribute('allowfullscreen', 'true');
  frameEl.setAttribute('referrerpolicy', 'strict-origin-when-cross-origin');
  frameEl.setAttribute('title', 'YouTube player');
  playerWrapEl.appendChild(frameEl);

  infoEl = document.createElement('div');
  infoEl.className = 'yt-hero-info hidden';
  const infoText = document.createElement('div');
  infoText.className = 'yt-hero-text';
  infoTitleEl = document.createElement('div');
  infoTitleEl.className = 'yt-hero-info-title';
  infoSubEl = document.createElement('div');
  infoSubEl.className = 'yt-hero-sub';
  infoText.append(infoTitleEl, infoSubEl);
  const backBtn = document.createElement('button');
  backBtn.type = 'button';
  backBtn.className = 'yt-hero-back';
  backBtn.title = 'Back';
  backBtn.setAttribute('aria-label', 'Back to YouTube search');
  void setIcon(backBtn, 'ui/close', { size: 15 });
  backBtn.addEventListener('click', reset);
  infoEl.append(infoText, backBtn);

  heroEl.append(idleEl, playerWrapEl, infoEl);
  tileEl.appendChild(heroEl);

  /* Search */
  searchEl = searchBar({ placeholder: 'Search YouTube…' });
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
  idleEl = null;
  gridEl = null;
  gridLabelEl = null;
  categoriesEl = null;
  searchEl = null;
}

/** The tile element (or null when the YouTube window is not mounted). */
export function getYoutubeTileElement() {
  return tileEl;
}

export default {
  name: 'youtube',
  icon: 'ui/youtube',
  mount: mountYoutubeTile,
  unmount: unmountYoutubeTile,
  getElement: getYoutubeTileElement,
  wireEvents: wireYoutubeEvents,
};
