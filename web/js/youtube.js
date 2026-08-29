/**
 * youtube.js — the YouTube plugin's window.
 *
 * Radio-style flat surface: ambient thumbnail glow behind everything, a hero
 * (idle brand mark, or the embedded player while a video plays), a search bar
 * (in-tile, works with the keyboard plugin) and a 16:9 thumbnail grid below.
 *
 * Playback is driven by the AI (`youtube_play` tool, or tapping a result
 * card) and by the in-tile search (`/api/youtube/search`).
 */
import { apiFetch } from './api.js';
import { setIcon, searchBar, emptyState, spinner } from '../ui/index.js';

export const YOUTUBE_PLUGIN = 'youtube';

let tileEl = null;
let glowEl = null;
let heroEl = null;       // hero container (idle view or player + info)
let playerWrapEl = null; // embed player wrapper (16:9)
let frameEl = null;      // embed iframe
let infoEl = null;       // playing info row (title + channel + back)
let infoTitleEl = null;
let infoSubEl = null;
let idleEl = null;       // idle hero view
let gridEl = null;
let searchEl = null;

let current = null;      // { video_id, title, channel, thumbnail }
let aiResults = [];      // results pushed by the AI (artifact:saved)
let wired = false;

function thumbFor(videoId) {
  return `https://i.ytimg.com/vi/${videoId}/hqdefault.jpg`;
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

  // Ambient glow mirrors the thumbnail, blurred and dimmed.
  if (glowEl) {
    if (playing && current.thumbnail) {
      glowEl.style.backgroundImage = `url("${current.thumbnail}")`;
      glowEl.classList.add('yt-glow--on');
    } else {
      glowEl.style.backgroundImage = '';
      glowEl.classList.remove('yt-glow--on');
    }
  }

  renderGridCurrent();
}

function playVideo(v) {
  if (!tileEl || !v?.video_id) return;
  current = {
    video_id: v.video_id,
    title: v.title || 'YouTube video',
    channel: v.channel || '',
    thumbnail: v.thumbnail || thumbFor(v.video_id),
  };
  frameEl.src = `https://www.youtube.com/embed/${v.video_id}?autoplay=1&rel=0`;
  renderHero();
  // Bring the window forward (desktop pulse / phone switch / full screen).
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: YOUTUBE_PLUGIN } }));
}

function reset() {
  current = null;
  if (frameEl) frameEl.src = 'about:blank';
  renderHero();
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

function renderGrid(results, term) {
  if (!gridEl) return;
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
    renderGrid([], null);
    return;
  }
  renderGrid(null, null);
  try {
    const res = await apiFetch(`/api/youtube/search?q=${encodeURIComponent(term)}`);
    renderGrid(res?.data?.results || [], term);
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

/* ── AI wiring ────────────────────────────────────────────────── */

function onAgentActions(e) {
  for (const action of e.detail || []) {
    if (action?.action === 'youtube_play' && action?.result === 'ok') {
      const id = action?.data?.video_id;
      if (id) playVideo({ video_id: id, title: action?.data?.title });
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

  glowEl = document.createElement('div');
  glowEl.className = 'yt-glow';
  glowEl.setAttribute('aria-hidden', 'true');
  tileEl.appendChild(glowEl);

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
    searchTimer = window.setTimeout(() => void runSearch(e.target.value), 350);
  });
  tileEl.appendChild(searchEl);

  /* Grid */
  gridEl = document.createElement('div');
  gridEl.className = 'yt-grid';
  tileEl.appendChild(gridEl);

  renderHero();
  renderGrid([], null);
  return tileEl;
}

/** Deactivated mid-playback: stop the video, drop the window. */
export function unmountYoutubeTile() {
  reset();
  tileEl?.remove();
  tileEl = null;
  glowEl = null;
  heroEl = null;
  playerWrapEl = null;
  frameEl = null;
  infoEl = null;
  infoTitleEl = null;
  infoSubEl = null;
  idleEl = null;
  gridEl = null;
  searchEl = null;
}

/** The tile element (or null when the YouTube window is not mounted). */
export function getYoutubeTileElement() {
  return tileEl;
}
