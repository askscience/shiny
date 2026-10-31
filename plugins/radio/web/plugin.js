import {
  mediaCard, icon, button, searchBar, spinner, emptyState, notify, toast,
  setTileGlowFromUrl, listReveal, decodeText,
} from '../../ui/index.js';
/**
 * radio.js — the Radio plugin's window (compact, editorial, artwork-driven).
 *
 * Layout: a compact now-playing bar that doubles as the window bar (small
 * album artwork from Cover Art Archive + MusicBrainz, a tight display title,
 * a mono meta line, an ink play pill and a grey stop pill) on top of a grid
 * of station artwork from the Radio Browser directory, each captioned in mono
 * below. The artwork is mirrored as a blurred ambient glow behind the surface.
 *
 * Now-playing track titles come from core's /api/radio/nowplaying route,
 * which parses ICY stream metadata server-side (browsers can't).
 *
 * AI wiring:
 * - `radio_play` tool artifacts (type `radio_station`) arrive via
 *   `artifact:saved` and autoplay from the action params' stream_url.
 * - `radio_stop` arrives via the `agent:actions` event and halts playback.
 * - Play/Stop buttons on artifact cards dispatch `artifact:action`; this
 *   module handles both radio actions there too.
 */

import { getToken } from '../../js/api.js';
import { t } from '../../js/i18n.js';

export const RADIO_PLUGIN = 'radio';

// `all` round-robins the live API servers via DNS; the named hosts are
// fallbacks (the old fi1 host no longer resolves).
const API_HOSTS = [
  'https://all.api.radio-browser.info',
  'https://de1.api.radio-browser.info',
  'https://de2.api.radio-browser.info',
];

const NOW_PLAYING_KEY = 'radio.now_playing';
const POLL_MS = 20000;

let tileEl = null;
let gridEl = null;

/* Hero pieces */
let artEl = null;
let trackTitleEl = null;
let trackSubEl = null;
let toggleBtn = null;
let stopBtn = null;

const audio = new Audio();
audio.preload = 'none';

let current = null;         // station { name, streamUrl, favicon, stationuuid, tags, country, bitrate }
let playing = false;
let nowPlaying = null;      // { title, artist }
let artworkUrl = null;      // resolved image URL for the current track/station
let glowSrc = null;         // artwork currently used for the window background
let pollTimer = null;
let titleFrame = 0;         // pending marquee measurement
let lastTitle = null;       // hero title last rendered (decode only on change)

const artworkCache = new Map(); // "artist|title" -> image URL | null

/* ── Playback ───────────────────────────────────────────────── */

function readNowPlaying() {
  try { return JSON.parse(localStorage.getItem(NOW_PLAYING_KEY)) || null; }
  catch (_) { return null; }
}

function writeNowPlaying(station) {
  if (station) localStorage.setItem(NOW_PLAYING_KEY, JSON.stringify(station));
  else localStorage.removeItem(NOW_PLAYING_KEY);
}

function playStation(station, { announce = true } = {}) {
  if (!station?.streamUrl) return;
  current = station;
  writeNowPlaying(station);
  audio.src = station.streamUrl;
  playing = false;
  nowPlaying = null;
  artworkUrl = null;
  renderHero();
  audio.play().then(() => {
    playing = true;
    renderHero();
    startPolling();
    if (announce) {
      notify({
        app: 'Radio',
        title: t('plugin.radio.notify.nowPlaying', null, 'Now playing'),
        body: station.name,
        icon: 'ui/play',
        plugin: RADIO_PLUGIN,
        urgency: 'low',
      });
    }
  }).catch(() => {
    playing = false;
    renderHero();
    toast(
      t('plugin.radio.toast.streamRefused', null, 'Stream refused to play — try another station'),
      { type: 'error' },
    );
  });
}

function togglePlayback() {
  if (!current) return;
  if (playing) {
    audio.pause();
    playing = false;
    stopPolling();
    renderHero();
  } else {
    audio.play().then(() => {
      playing = true;
      renderHero();
      startPolling();
    }).catch(() => {
      playing = false;
      renderHero();
      toast(
        t('plugin.radio.toast.streamRefused', null, 'Stream refused to play — try another station'),
        { type: 'error' },
      );
    });
  }
}

function stopPlayback() {
  audio.pause();
  audio.removeAttribute('src');
  audio.load();
  playing = false;
  current = null;
  nowPlaying = null;
  artworkUrl = null;
  stopPolling();
  writeNowPlaying(null);
  renderHero();
}

/* ── Now-playing metadata (core ICY proxy) ──────────────────── */

function startPolling() {
  stopPolling();
  void pollNowPlaying();
  pollTimer = window.setInterval(() => void pollNowPlaying(), POLL_MS);
}

function stopPolling() {
  if (pollTimer) window.clearInterval(pollTimer);
  pollTimer = null;
}

async function pollNowPlaying() {
  if (document.hidden || !playing || !current?.streamUrl) return;
  try {
    const res = await fetch(
      `/api/radio/nowplaying?url=${encodeURIComponent(current.streamUrl)}`,
      { headers: { Authorization: `Bearer ${getToken()}` } },
    );
    if (!res.ok) return;
    const json = await res.json();
    const title = json?.data?.title?.trim();
    if (title && title !== nowPlaying?.raw) {
      const previous = nowPlaying?.raw || null;
      nowPlaying = { raw: title, ...splitTrackTitle(title) };
      artworkUrl = null;
      renderHero();
      void resolveArtwork();
      // A real track change (not the first detection after starting playback).
      if (previous) {
        notify({
          app: 'Radio',
          title: t('plugin.radio.notify.nowPlaying', null, 'Now playing'),
          body: nowPlaying.artist ? `${nowPlaying.artist} — ${nowPlaying.title}` : nowPlaying.title,
          icon: 'ui/play',
          plugin: RADIO_PLUGIN,
          urgency: 'low',
        });
      }
    }
  } catch (_) { /* transient — next poll retries */ }
}

/** "Artist — Title" (or "Artist - Title") → { artist, title } */
function splitTrackTitle(raw) {
  const m = raw.split(/\s+[—–-]\s+/);
  if (m.length >= 2) return { artist: m[0].trim(), title: m.slice(1).join(' — ').trim() };
  return { artist: null, title: raw };
}

/* ── Artwork: MusicBrainz → Cover Art Archive ───────────────── */

async function fetchJson(url) {
  const res = await fetch(url, {
    headers: { 'User-Agent': 'Shiny/0.1 (https://github.com/askscience/shiny)' },
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

async function resolveArtwork() {
  if (!nowPlaying?.title) return;
  const key = `${nowPlaying.artist || ''}|${nowPlaying.title}`.toLowerCase();
  if (artworkCache.has(key)) {
    artworkUrl = artworkCache.get(key);
    renderHero();
    return;
  }
  try {
    const q = nowPlaying.artist
      ? `recording:"${nowPlaying.title}" AND artist:"${nowPlaying.artist}"`
      : `recording:"${nowPlaying.title}"`;
    const mb = await fetchJson(
      `https://musicbrainz.org/ws/2/recording/?query=${encodeURIComponent(q)}&fmt=json&limit=1`,
    );
    const releaseId = mb?.recordings?.[0]?.releases?.[0]?.id;
    if (!releaseId) throw new Error('no release');
    const caa = await fetchJson(`https://coverartarchive.org/release/${releaseId}`);
    const front = caa?.images?.find((i) => i.front) || caa?.images?.[0];
    const url = front?.thumbnails?.large || front?.image || null;
    artworkCache.set(key, url);
    artworkUrl = url;
    renderHero();
  } catch (_) {
    artworkCache.set(key, null);
  }
}

/* ── Radio Browser API (browser-direct) ─────────────────────── */

async function rbFetch(path) {
  let lastErr = null;
  for (const host of API_HOSTS) {
    try {
      const res = await fetch(`${host}/json/${path}`);
      if (res.ok) return await res.json();
      lastErr = new Error(`Radio Browser error: ${res.status}`);
    } catch (e) {
      lastErr = e;
    }
  }
  throw (lastErr || new Error('Radio Browser unreachable'));
}

async function searchStations(text) {
  const term = text.trim();
  const path = term
    ? `stations/search?name=${encodeURIComponent(term)}&hidebroken=true&order=votes&reverse=true&limit=24`
    : 'stations/topvote/24?hidebroken=true';
  return rbFetch(path);
}

function toStation(raw) {
  return {
    stationuuid: raw.stationuuid,
    name: raw.name,
    streamUrl: raw.url_resolved || raw.url,
    favicon: raw.favicon || '',
    country: raw.country || '',
    tags: raw.tags || '',
    bitrate: raw.bitrate || 0,
    codec: raw.codec || '',
  };
}

/* ── Hero ───────────────────────────────────────────────────── */

function heroTitle() {
  if (nowPlaying?.title) return nowPlaying.title;
  if (current) return current.name;
  // Idle: the bar is the window bar, so it names the window.
  return t('plugin.radio.title', null, 'Radio');
}

function heroSubtitle() {
  if (nowPlaying?.artist) {
    const parts = [nowPlaying.artist, current?.name].filter(Boolean);
    return [...new Set(parts)].join(' · ');
  }
  if (!current) return t('plugin.radio.status.pickStation', null, 'Pick a station');
  const tag = (current.tags || '').split(',').map((t) => t.trim()).filter(Boolean)[0];
  const rate = current.bitrate ? t('plugin.radio.status.kbps', { n: current.bitrate }, `${current.bitrate} kbps`) : '';
  return [tag, current.country, rate].filter(Boolean).join(' · ') || t('plugin.radio.status.streaming', null, 'Streaming');
}

function heroArt() {
  if (artworkUrl) return artworkUrl;
  if (current?.favicon) return current.favicon;
  return null;
}

function renderHero() {
  if (!tileEl || !trackTitleEl) return;
  const hasStation = !!current;
  tileEl.classList.toggle('radio-tile--active', hasStation);
  tileEl.classList.toggle('radio-tile--live', hasStation && playing);

  const titleText = heroTitle();
  // A new ICY track (or station) decodes in; re-renders of the same title
  // (play/pause, artwork arriving) leave it alone.
  if (titleText !== lastTitle) {
    if (lastTitle !== null && (nowPlaying?.title || current)) decodeText(trackTitleEl, titleText);
    else trackTitleEl.textContent = titleText;
    lastTitle = titleText;
  }
  trackTitleEl.title = titleText;
  // Long titles scroll on a marquee loop instead of truncating. Measured on
  // the next frame — by then the window may have been unmounted.
  cancelAnimationFrame(titleFrame);
  const titleNode = trackTitleEl;
  titleFrame = requestAnimationFrame(() => {
    titleFrame = 0;
    if (!titleNode.isConnected) return;
    titleNode.textContent = titleText;
    const over = titleNode.scrollWidth - titleNode.clientWidth;
    titleNode.classList.toggle('radio-hero-title--scroll', over > 8);
    if (over > 8) titleNode.style.setProperty('--scroll-dist', `${-(over + 28)}px`);
  });
  trackSubEl.textContent = heroSubtitle();

  const art = heroArt();
  artEl.classList.toggle('radio-hero-art--empty', !art);
  artEl.innerHTML = '';
  if (art) {
    const img = document.createElement('img');
    img.src = art;
    img.alt = '';
    img.loading = 'lazy';
    img.onerror = () => {
      artEl.classList.add('radio-hero-art--empty');
      artEl.innerHTML = '';
    };
    artEl.appendChild(img);
  } else {
    artEl.innerHTML = '';
  }

  // Ambient glow mirrors the artwork (Tier 1), lightly blurred; with no
  // artwork it falls back to the window's Tier 0 colour glow. Only reload when
  // the artwork actually changes — renderHero runs on every play/pause.
  if (art !== glowSrc) {
    glowSrc = art;
    void setTileGlowFromUrl(tileEl, art, { size: 320, blur: 7 });
  }

  toggleBtn.disabled = !hasStation;
  toggleBtn.textContent = '';
  toggleBtn.appendChild(icon(playing ? 'ui/pause' : 'ui/play', { size: 16 }));
  toggleBtn.title = playing
    ? t('plugin.radio.menu.pause', null, 'Pause')
    : t('plugin.radio.menu.play', null, 'Play');
  toggleBtn.setAttribute('aria-label', toggleBtn.title);

  stopBtn.disabled = !hasStation;
  stopBtn.title = t('plugin.radio.menu.stop', null, 'Stop');
  stopBtn.setAttribute('aria-label', stopBtn.title);

  renderGridCurrent();
}

/* ── Station grid ───────────────────────────────────────────── */

function renderGridCurrent() {
  if (!gridEl) return;
  gridEl.querySelectorAll('.radio-cell').forEach((cell) => {
    const on = current?.stationuuid && cell.dataset.stationuuid === current.stationuuid;
    cell.classList.toggle('radio-cell--current', !!on && playing);
  });
}

function stationCell(s, idx) {
  const cell = mediaCard({ title: s.name, src: s.favicon, position: idx + 1,
    className: 'radio-cell', prefix: 'radio-cell', onClick: () => playStation(s) });
  cell.dataset.stationuuid = s.stationuuid || '';
  cell.classList.toggle('radio-cell--current', current?.stationuuid === s.stationuuid && playing);
  return cell;
}

function renderGrid(stations, term) {
  if (!gridEl) return;
  gridEl.innerHTML = '';
  if (!stations?.length) {
    const wrap = document.createElement('div');
    wrap.className = 'radio-grid-status';
    wrap.appendChild(term
      ? emptyState({
        icon: 'ui/search',
        title: t('plugin.radio.empty.noStations', null, 'No stations found'),
        body: t('plugin.radio.empty.noMatch', { term }, `Nothing matched “${term}”.`),
      })
      : spinner());
    gridEl.appendChild(wrap);
    return;
  }
  const cells = stations.map((raw, i) => stationCell(toStation(raw), i));
  gridEl.append(...cells);
  void listReveal(cells);
}

async function runSearch(text) {
  renderGrid(null, null);
  try {
    renderGrid(await searchStations(text), text);
  } catch (e) {
    if (!gridEl) return;
    gridEl.innerHTML = '';
    const wrap = document.createElement('div');
    wrap.className = 'radio-grid-status';
    wrap.appendChild(emptyState({
      icon: 'ui/warning',
      title: t('plugin.radio.error.unreachable', null, 'Radio Browser unreachable'),
      body: e.message || t('plugin.radio.error.tryAgain', null, 'Try again in a moment'),
    }));
    gridEl.appendChild(wrap);
  }
}

/* ── Artifact cards (AI actions) ────────────────────────────── */

function streamFromArtifact(artifact) {
  const action = artifact?.actions?.find((a) => a.tool === 'radio_play');
  const p = action?.params;
  if (!p?.stream_url) return null;
  return toStation({
    stationuuid: p.stationuuid,
    name: p.name || artifact?.title,
    url_resolved: p.stream_url,
    favicon: p.favicon || '',
  });
}

function onArtifactAction(e) {
  const { action, artifact } = e.detail || {};
  if (action?.tool === 'radio_stop') {
    stopPlayback();
    return;
  }
  if (action?.tool === 'radio_play') {
    const station = streamFromArtifact(artifact) || toStation({
      stationuuid: action?.params?.stationuuid,
      name: action?.params?.name || t('plugin.radio.fallback.station', null, 'Station'),
      url_resolved: action?.params?.stream_url,
      favicon: action?.params?.favicon || '',
    });
    if (station) playStation(station);
  }
}

function onAgentActions(e) {
  for (const action of e.detail || []) {
    if (action?.action === 'radio_stop' && action?.result === 'ok') {
      stopPlayback();
    }
  }
}

function onArtifactSaved(e) {
  const art = e.detail;
  if (art?.type !== 'radio_station') return;
  const station = streamFromArtifact(art);
  if (station) playStation(station);
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

/** Create the Radio tile element (the plugin's window container). */
export function mountRadioTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile radio-tile';
  tileEl.dataset.plugin = RADIO_PLUGIN;

  /* Hero */
  const hero = document.createElement('div');
  hero.className = 'radio-hero';
  hero.dataset.windowBar = '';

  artEl = document.createElement('div');
  artEl.className = 'radio-hero-art radio-hero-art--empty';

  const heroText = document.createElement('div');
  heroText.className = 'radio-hero-text';
  trackTitleEl = document.createElement('div');
  trackTitleEl.className = 'radio-hero-title';
  trackSubEl = document.createElement('div');
  trackSubEl.className = 'radio-hero-sub';
  heroText.append(trackTitleEl, trackSubEl);

  // Transport: play/pause is the ink pill, stop the grey pill.
  const transport = document.createElement('div');
  transport.className = 'radio-transport';
  toggleBtn = button({ icon: 'ui/play', iconSize: 16, variant: 'primary', size: 'sm', onClick: togglePlayback });
  toggleBtn.classList.add('radio-t-btn', 'radio-t-btn--primary');
  stopBtn = button({ icon: 'ui/stop', iconSize: 16, variant: 'ghost', size: 'sm', onClick: stopPlayback });
  stopBtn.classList.add('radio-t-btn');
  transport.append(toggleBtn, stopBtn);

  hero.append(artEl, heroText, transport);
  tileEl.appendChild(hero);

  /* Search */
  const search = searchBar({ placeholder: t('plugin.radio.search.placeholder', null, 'Stations or genres…') });
  let searchTimer = null;
  search.input.addEventListener('input', (e) => {
    window.clearTimeout(searchTimer);
    searchTimer = window.setTimeout(() => void runSearch(e.target.value), 350);
  });
  tileEl.appendChild(search);

  /* Grid */
  gridEl = document.createElement('div');
  gridEl.className = 'radio-grid';
  tileEl.appendChild(gridEl);

  // Restore a manually-picked station; AI picks don't persist across reloads.
  current = readNowPlaying();
  renderHero();

  void runSearch('');
  return tileEl;
}

/** Deactivated mid-playback: stop audio, drop the window. */
export function unmountRadioTile() {
  stopPlayback();
  cancelAnimationFrame(titleFrame);
  titleFrame = 0;
  lastTitle = null;
  tileEl?.remove();
  // Drop references so a later mountRadioTile() builds a fresh tile
  // instead of returning the detached old one.
  tileEl = null;
  gridEl = null;
  glowSrc = null;
  artEl = null;
  trackTitleEl = null;
  trackSubEl = null;
  toggleBtn = null;
  stopBtn = null;
}

/** The tile element (or null when the radio window is not mounted). */
export function getRadioTileElement() {
  return tileEl;
}

/* ── Wiring (registered once) ───────────────────────────────── */

let wired = false;
export function wireRadioEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('artifact:action', onArtifactAction);
  window.addEventListener('artifact:saved', onArtifactSaved);
  window.addEventListener('artifact:updated', onArtifactSaved);
  window.addEventListener('agent:actions', onAgentActions);
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators + window management. */
export function radioContextMenu() {
  const hasStation = !!current;
  return [
    {
      type: 'item',
      label: playing
        ? t('plugin.radio.menu.pause', null, 'Pause')
        : t('plugin.radio.menu.play', null, 'Play'),
      icon: playing ? 'ui/pause' : 'ui/play',
      disabled: !hasStation,
      onClick: () => togglePlayback(),
    },
    {
      type: 'item',
      label: t('plugin.radio.menu.stop', null, 'Stop'),
      icon: 'ui/stop',
      disabled: !hasStation,
      onClick: () => stopPlayback(),
    },
    { type: 'separator' },
    {
      type: 'item',
      label: t('plugin.radio.menu.refresh', null, 'Refresh stations'),
      icon: 'ui/refresh',
      onClick: () => void runSearch(''),
    },
  ];
}

export default {
  name: 'radio',
  icon: 'ui/play',
  mount: mountRadioTile,
  unmount: unmountRadioTile,
  getElement: getRadioTileElement,
  wireEvents: wireRadioEvents,
  contextMenu: radioContextMenu,
};
