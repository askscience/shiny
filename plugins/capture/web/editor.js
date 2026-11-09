import {
  element as h, toolbar, toolButton, button, menuSurface, menuItem,
  segmented, slider, toggleRow, field, input, textarea, select, icon, emptyState, spinner, toast,
  notify, timeline, secondsScale, drawWaveform, formatTimecode, tabStrip, tab,
} from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';
import { askInChat } from '../../js/textInput.js';
import { pickPaths } from '../../js/filePicker.js';

/**
 * capture/editor.js — the video editor.
 *
 * The project lives on the server (one JSON file); every change is an
 * operation sent to it (the same operations the AI uses), so the user and
 * the assistant always edit the same document. The window polls the
 * revision and reloads when the assistant saved something.
 *
 * The preview composes the timeline in the page: one <video> per visible
 * clip kept in sync with a master clock, HTML graphics in sandboxed
 * iframes seeked to the same time, captions as text. The export (server)
 * is the exact render; the preview is for judging the edit.
 */

let root = null;
let project = null;
let tl = null;
let stageEl = null;
let stageInner = null;
let captionEl = null;
let timeEl = null;
let playBtn = null;
let sideEl = null;
let sideTab = 'inspect';
let titleInput = null;
let playing = false;
let clock = { t: 0, at: 0, from: 0 };
let raf = 0;
let pollTimer = 0;
let selectedId = null;
let summary = null;
const players = new Map();   // clip id -> <video>|<audio>
const frames = new Map();    // overlay id -> <iframe>
const peaksCache = new Map();
let busy = false;

const OVERLAY_TRACK_KINDS = new Set(['overlay', 'captions']);

/* ── server ─────────────────────────────────────────────────── */

async function load(id) {
  const r = await apiFetch(`/api/capture/projects/${encodeURIComponent(id)}`);
  project = r?.data || null;
  render();
}

async function send(body) {
  if (!project) return null;
  busy = true;
  try {
    const r = await apiFetch(`/api/capture/projects/${encodeURIComponent(project.id)}`, {
      method: 'PUT',
      body: JSON.stringify({ revision: project.revision, ...body }),
    });
    project = r?.data || project;
    render();
    return r?.data;
  } catch (e) {
    if (/conflict/i.test(e.message || '')) {
      await load(project.id);
      toast(t('plugin.capture.editor.reloaded', null, 'The project changed (the assistant edited it). Reloaded.'), { type: 'info' });
    } else {
      toast(e.message || String(e), { type: 'error' });
    }
    return null;
  } finally {
    busy = false;
  }
}

const ops = (list) => send({ ops: list });

function startPolling() {
  stopPolling();
  const tick = async () => {
    if (project && root?.isConnected && !busy && !document.hidden) {
      try {
        const r = await apiFetch(`/api/capture/projects/${encodeURIComponent(project.id)}`);
        if (r?.data && r.data.revision !== project.revision) {
          project = r.data;
          render();
        }
      } catch (_) { /* noop */ }
    }
    pollTimer = setTimeout(tick, 3000);
  };
  pollTimer = setTimeout(tick, 3000);
}

function stopPolling() {
  clearTimeout(pollTimer);
  pollTimer = 0;
}

/* ── helpers ────────────────────────────────────────────────── */

const media = (id) => project?.media?.find((m) => m.id === id);
const track = (id) => project?.tracks?.find((x) => x.id === id);
const clipLen = (c) => Math.max(0, (c.out - c.in) / c.speed);
const mediaUrl = (m) => `/api/capture/media?path=${encodeURIComponent(m.path)}`;

function duration() {
  if (!project) return 0;
  let d = 0;
  for (const c of project.clips) d = Math.max(d, c.at + clipLen(c));
  for (const o of project.overlays) d = Math.max(d, o.at + o.dur);
  for (const c of project.captions) d = Math.max(d, c.at + c.dur);
  return d;
}

function itemsFromProject() {
  const items = [];
  for (const c of project.clips) {
    const m = media(c.media);
    const tr = track(c.track);
    items.push({
      id: c.id, track: c.track, start: c.at, end: c.at + clipLen(c),
      label: `${m?.label || ''}${c.speed !== 1 ? ` · ${c.speed}×` : ''}`,
      className: `capture-clip capture-clip--${tr?.kind || 'video'}`,
      ghost: c.ghost, version: project.revision, kind: 'clip',
    });
  }
  for (const o of project.overlays) {
    items.push({ id: o.id, track: o.track, start: o.at, end: o.at + o.dur, label: o.name || t('plugin.capture.editor.graphic', null, 'Graphic'), className: 'capture-clip capture-clip--overlay', ghost: o.ghost, kind: 'overlay' });
  }
  const capTrack = project.tracks.find((x) => x.kind === 'captions')?.id;
  if (capTrack) {
    for (const c of project.captions) items.push({ id: c.id, track: capTrack, start: c.at, end: c.at + c.dur, label: c.text, className: 'capture-clip capture-clip--caption', kind: 'caption' });
  }
  return items;
}

function cutMarks() {
  const out = [];
  for (const p of project?.proposals || []) {
    for (const op of p.ops) {
      if (op.op === 'cut_ranges') for (const [a, b] of op.ranges || []) out.push({ t: a, end: b, className: 'is-cut', label: p.summary });
    }
  }
  for (const m of project?.markers || []) out.push({ t: m.t, label: m.label });
  return out;
}

async function peaksFor(m, trackIdx) {
  const key = `${m.id}:${trackIdx}`;
  if (!peaksCache.has(key)) {
    peaksCache.set(key, apiFetch(`/api/capture/media/peaks?path=${encodeURIComponent(m.path)}&track=${trackIdx}`).then((r) => r?.data).catch(() => null));
  }
  return peaksCache.get(key);
}

function renderItem(body, item, { width }) {
  body.textContent = '';
  if (item.kind !== 'clip') return;
  const c = project.clips.find((x) => x.id === item.id);
  const m = c && media(c.media);
  if (!m) return;
  const tr = track(c.track);
  if (tr?.kind === 'audio') {
    const cv = document.createElement('canvas');
    cv.className = 'capture-clip-wave';
    body.appendChild(cv);
    peaksFor(m, c.audio_track).then((peaks) => {
      if (peaks && cv.isConnected) drawWaveform(cv, { peaks, from: c.in, to: c.out, gain: c.volume || 1, alpha: 0.6 });
    });
  } else if (m.has_video || m.kind === 'image') {
    const strip = h('div', 'capture-clip-thumbs');
    const n = 12;
    const md = Math.max(0.1, m.duration || clipLen(c));
    strip.style.backgroundImage = `url("/api/capture/media/thumbs?path=${encodeURIComponent(m.path)}&n=${n}")`;
    // The strip covers the whole media: scale and shift it to this clip's part.
    const span = (c.out - c.in) / md;
    strip.style.backgroundSize = `${(100 / Math.max(0.01, span)).toFixed(2)}% 100%`;
    strip.style.backgroundPosition = `${span >= 0.999 ? 0 : ((c.in / md) / (1 - span)) * 100}% 0`;
    body.appendChild(strip);
    if (width < 40) strip.style.opacity = '0.5';
  }
}

/* ── timeline ───────────────────────────────────────────────── */

function buildTimeline() {
  tl = timeline({
    className: 'capture-timeline',
    zoom: 40, minZoom: 1, maxZoom: 600,
    makeScale: (z) => secondsScale({ pps: z }),
    tracks: [],
    items: [],
    renderItem,
    markers: cutMarks,
    renderHead: (tr) => {
      const box = h('div', 'capture-track-head');
      box.appendChild(icon(tr.kind === 'audio' ? 'ui/volume' : tr.kind === 'overlay' ? 'ui/magic-wand' : tr.kind === 'captions' ? 'ui/type' : 'ui/video', { size: 14 }));
      box.appendChild(h('span', 'capture-track-name', tr.label));
      if (tr.kind !== 'captions' && tr.kind !== 'overlay') {
        const mute = toolButton(tr.kind === 'audio' ? (tr.muted ? 'ui/mic-off' : 'ui/mic') : (tr.hidden ? 'ui/incognito' : 'ui/eye'),
          tr.kind === 'audio' ? t('plugin.capture.editor.mute', null, 'Mute') : t('plugin.capture.editor.hide', null, 'Hide'),
          () => ops([{ op: 'set_track', track: tr.id, ...(tr.kind === 'audio' ? { muted: !tr.muted } : { hidden: !tr.hidden }) }]));
        box.appendChild(mute);
      }
      return box;
    },
    onSelect: (id) => { selectedId = id; if (id) sideTab = 'inspect'; renderSide(); },
    onSeek: (tt) => seek(tt),
    canDrop: (item, trackId) => {
      const from = track(item.track)?.kind;
      const to = track(trackId)?.kind;
      return from === to;
    },
    minStart: (item, d) => {
      if (item.kind !== 'clip') return 0;
      const c = project.clips.find((x) => x.id === item.id);
      return Math.max(0, d.start - c.in / c.speed);
    },
    maxEnd: (item, d) => {
      if (item.kind !== 'clip') return Infinity;
      const c = project.clips.find((x) => x.id === item.id);
      const m = media(c.media);
      if (!m || m.kind === 'image') return Infinity;
      return d.start + (m.duration - c.in) / c.speed;
    },
    onMove: (id, { start, track: tr }) => {
      const item = itemsFromProject().find((x) => x.id === id);
      if (item?.kind === 'clip') ops([{ op: 'move_clip', clip: id, at: start, track: tr }]);
      else if (item?.kind === 'overlay') ops([{ op: 'update_overlay', id, at: start }]);
      else if (item?.kind === 'caption') moveCaption(id, start);
    },
    onTrim: (id, { start, end, edge, from }) => {
      const item = itemsFromProject().find((x) => x.id === id);
      if (item?.kind === 'clip') {
        const c = project.clips.find((x) => x.id === id);
        if (edge === 'start') ops([{ op: 'trim_clip', clip: id, at: start, in: c.in + (start - from.start) * c.speed }]);
        else ops([{ op: 'trim_clip', clip: id, out: c.in + (end - c.at) * c.speed }]);
      } else if (item?.kind === 'overlay') {
        ops([{ op: 'update_overlay', id, at: start, dur: end - start }]);
      } else if (item?.kind === 'caption') {
        const cap = project.captions.find((x) => x.id === id);
        send({ project: { ...project, captions: project.captions.map((x) => (x.id === id ? { ...cap, at: start, dur: end - start } : x)) } });
      }
    },
    contextMenu: (id, e) => clipMenu(id, e),
    onKey: (e) => onKey(e),
  });
  return tl.el;
}

function moveCaption(id, at) {
  send({ project: { ...project, captions: project.captions.map((x) => (x.id === id ? { ...x, at } : x)) } });
}

function clipMenu(id, e) {
  const surface = menuSurface({ className: 'capture-menu', onEscape: () => surface.remove() });
  const add = (label, fn, disabled = false) => surface.appendChild(menuItem({ label, disabled, onClick: () => { surface.remove(); fn(); } }));
  const item = itemsFromProject().find((x) => x.id === id);
  if (item?.kind === 'clip') {
    add(t('plugin.capture.editor.split', null, 'Split at the playhead'), () => splitAtPlayhead(id));
    add(t('plugin.capture.editor.speed2', null, 'Speed 2×'), () => ops([{ op: 'set_clip', clip: id, speed: 2 }]));
    add(t('plugin.capture.editor.speed1', null, 'Normal speed'), () => ops([{ op: 'set_clip', clip: id, speed: 1 }]));
  }
  add(t('plugin.capture.editor.delete', null, 'Delete'), () => deleteItem(id));
  surface.classList.remove('hidden');
  document.body.appendChild(surface);
  surface.style.left = `${e.clientX}px`;
  surface.style.top = `${e.clientY}px`;
  surface.focusFirst?.();
  setTimeout(() => document.addEventListener('pointerdown', function off(ev) {
    if (!surface.contains(ev.target)) { surface.remove(); document.removeEventListener('pointerdown', off, true); }
  }, true));
}

function deleteItem(id) {
  const item = itemsFromProject().find((x) => x.id === id);
  if (!item) return;
  if (item.kind === 'clip') ops([{ op: 'delete_clip', clip: id }]);
  else if (item.kind === 'overlay') ops([{ op: 'delete_overlay', id }]);
  else ops([{ op: 'delete_caption', id }]);
  selectedId = null;
}

function splitAtPlayhead(id) {
  const target = id || selectedId;
  const list = target ? [target] : project.clips.filter((c) => clock.t > c.at && clock.t < c.at + clipLen(c)).map((c) => c.id);
  const valid = list.filter((cid) => {
    const c = project.clips.find((x) => x.id === cid);
    return c && clock.t > c.at + 0.05 && clock.t < c.at + clipLen(c) - 0.05;
  });
  if (!valid.length) return;
  ops(valid.map((cid) => ({ op: 'split_clip', clip: cid, t: clock.t })));
}

function onKey(e) {
  if (e.target.closest('input, textarea, [contenteditable]')) return;
  if (e.key === ' ') { e.preventDefault(); togglePlay(); }
  else if (e.key === 's' && !e.ctrlKey) { e.preventDefault(); splitAtPlayhead(); }
  else if ((e.key === 'Delete' || e.key === 'Backspace') && selectedId) { e.preventDefault(); deleteItem(selectedId); }
  else if (e.key === 'ArrowLeft') { e.preventDefault(); seek(clock.t - (e.shiftKey ? 5 : 1 / (project?.fps || 30))); }
  else if (e.key === 'ArrowRight') { e.preventDefault(); seek(clock.t + (e.shiftKey ? 5 : 1 / (project?.fps || 30))); }
  else if (e.key === 'z' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); undo(); }
}

/* ── preview player ─────────────────────────────────────────── */

function buildStage() {
  const wrap = h('section', 'capture-ed-stage');
  stageEl = h('div', 'capture-ed-canvas');
  stageInner = h('div', 'capture-ed-canvas-inner');
  captionEl = h('div', 'capture-ed-caption');
  stageEl.append(stageInner, captionEl);
  const transport = h('div', 'capture-ed-transport');
  playBtn = toolButton('ui/play', t('plugin.capture.editor.play', null, 'Play'), () => togglePlay());
  timeEl = h('span', 'capture-ed-time', '0:00.0');
  const fit = toolButton('ui/expand', t('plugin.capture.editor.fit', null, 'Fit the timeline'), () => tl?.zoomToFit());
  const back = toolButton('ui/rotate-left', t('plugin.capture.editor.start', null, 'Back to the start'), () => seek(0));
  transport.append(back, playBtn, timeEl, h('span', 'capture-bar-spacer'), fit);
  wrap.append(stageEl, transport);
  const ro = new ResizeObserver(() => fitStage());
  ro.observe(wrap);
  return wrap;
}

function fitStage() {
  if (!stageEl || !project) return;
  const box = stageEl.parentElement.getBoundingClientRect();
  const aw = Math.max(80, box.width - 16);
  const ah = Math.max(60, box.height - 56);
  const ratio = project.width / project.height;
  let w = aw;
  let hh = w / ratio;
  if (hh > ah) { hh = ah; w = hh * ratio; }
  stageEl.style.width = `${Math.round(w)}px`;
  stageEl.style.height = `${Math.round(hh)}px`;
  stageEl.style.setProperty('--stage-scale', String(w / project.width));
}

function rectStyle(el, r) {
  el.style.left = `${r.x * 100}%`;
  el.style.top = `${r.y * 100}%`;
  el.style.width = `${r.w * 100}%`;
  el.style.height = `${r.h * 100}%`;
}

function syncPlayers(tt, hard = false) {
  if (!project) return;
  const live = new Set();
  const order = new Map(project.tracks.map((x, i) => [x.id, i]));
  const audioMedia = new Set();
  for (const c of project.clips) {
    if (c.ghost) continue;
    const tr = track(c.track);
    const m = media(c.media);
    if (!m || !tr) continue;
    const active = tt >= c.at && tt < c.at + clipLen(c);
    if (!active) continue;
    const isAudio = tr.kind === 'audio';
    if (isAudio) {
      // One player per media for sound in the preview: a recording's mic and
      // desktop tracks play as its mix (the export uses each track exactly).
      if (audioMedia.has(c.media) || tr.muted || !c.volume) continue;
      audioMedia.add(c.media);
    } else if (tr.hidden) continue;
    live.add(c.id);
    let el = players.get(c.id);
    if (!el) {
      el = document.createElement(isAudio ? 'audio' : (m.kind === 'image' ? 'img' : 'video'));
      el.className = `capture-ed-layer${isAudio ? ' is-audio' : ''}`;
      if (el.tagName !== 'IMG') { el.preload = 'auto'; el.playsInline = true; }
      el.src = mediaUrl(m);
      players.set(c.id, el);
      stageInner.appendChild(el);
    }
    if (!isAudio) {
      rectStyle(el, c.rect || { x: 0, y: 0, w: 1, h: 1 });
      el.style.zIndex = String(10 + (order.get(c.track) || 0));
      el.style.opacity = String(c.opacity ?? 1);
      el.style.borderRadius = c.radius ? `${(c.radius * Math.min(c.rect.w * project.width, c.rect.h * project.height) / 2) * Number(getComputedStyle(stageEl).getPropertyValue('--stage-scale') || 1)}px` : '';
    }
    if (el.tagName === 'IMG') continue;
    const want = c.in + (tt - c.at) * c.speed;
    el.muted = isAudio ? false : !(c.volume > 0) || tr.muted;
    el.volume = Math.min(1, Math.max(0, (c.volume ?? 1) * (tr.volume ?? 1)));
    el.playbackRate = c.speed;
    if (hard || Math.abs((el.currentTime || 0) - want) > (playing ? 0.3 : 0.04)) {
      try { el.currentTime = want; } catch (_) { /* not ready */ }
    }
    if (playing && el.paused) el.play().catch(() => {});
    if (!playing && !el.paused) el.pause();
  }
  for (const [id, el] of players) {
    if (!live.has(id)) {
      if (el.pause) el.pause();
      el.remove();
      players.delete(id);
    }
  }
  // Graphics.
  const liveO = new Set();
  for (const o of project.overlays) {
    if (o.ghost && !selectedId) continue;
    if (tt < o.at || tt >= o.at + o.dur) continue;
    liveO.add(o.id);
    let f = frames.get(o.id);
    const src = `/api/capture/overlay/${encodeURIComponent(project.id)}/${encodeURIComponent(o.id)}?r=${project.revision}`;
    if (!f || f.dataset.src !== src) {
      f?.remove();
      f = document.createElement('iframe');
      f.className = 'capture-ed-overlay';
      f.setAttribute('sandbox', 'allow-scripts');
      f.setAttribute('scrolling', 'no');
      f.dataset.src = src;
      f.src = src;
      f.addEventListener('load', () => post(f, playing ? 'play' : 'seek', clock.t - o.at));
      frames.set(o.id, f);
      stageInner.appendChild(f);
    }
    rectStyle(f, o.rect);
    f.style.zIndex = '200';
    f.style.opacity = String(o.opacity ?? 1);
    const w = o.rect.w * project.width;
    const s = Number(getComputedStyle(stageEl).getPropertyValue('--stage-scale') || 1);
    f.style.width = `${w}px`;
    f.style.height = `${o.rect.h * project.height}px`;
    f.style.transform = `scale(${s})`;
    if (hard || !playing) post(f, 'seek', tt - o.at);
  }
  for (const [id, f] of frames) if (!liveO.has(id)) { f.remove(); frames.delete(id); }
  // Captions.
  const cap = project.captions.find((c) => tt >= c.at && tt < c.at + c.dur);
  captionEl.textContent = cap?.text || '';
  captionEl.hidden = !cap;
}

function post(frame, kind, tt) {
  try { frame.contentWindow?.postMessage({ capture: kind, t: Math.max(0, tt) }, '*'); } catch (_) { /* noop */ }
}

function seek(tt) {
  clock.t = Math.max(0, Math.min(tt, duration() || 0));
  clock.from = clock.t;
  clock.at = performance.now();
  tl?.setPlayhead(clock.t, { follow: playing });
  if (timeEl) timeEl.textContent = formatTimecode(clock.t, 1);
  syncPlayers(clock.t, true);
  if (playing) for (const [id, f] of frames) {
    const o = project.overlays.find((x) => x.id === id);
    if (o) post(f, 'play', clock.t - o.at);
  }
}

function togglePlay() {
  if (playing) pause(); else play();
}

function play() {
  if (!project || playing) return;
  if (clock.t >= duration() - 0.05) clock.t = 0;
  playing = true;
  clock.from = clock.t;
  clock.at = performance.now();
  playBtn.querySelector('.ui-icon')?.replaceWith(icon('ui/pause', { size: 16 }));
  syncPlayers(clock.t, true);
  for (const [id, f] of frames) {
    const o = project.overlays.find((x) => x.id === id);
    if (o) post(f, 'play', clock.t - o.at);
  }
  const loop = () => {
    if (!playing) return;
    clock.t = clock.from + (performance.now() - clock.at) / 1000;
    if (clock.t >= duration()) { pause(); seek(duration()); return; }
    tl?.setPlayhead(clock.t, { follow: true });
    timeEl.textContent = formatTimecode(clock.t, 1);
    syncPlayers(clock.t);
    raf = requestAnimationFrame(loop);
  };
  raf = requestAnimationFrame(loop);
}

function pause() {
  playing = false;
  cancelAnimationFrame(raf);
  playBtn?.querySelector('.ui-icon')?.replaceWith(icon('ui/play', { size: 16 }));
  for (const el of players.values()) el.pause?.();
  for (const [id, f] of frames) {
    const o = project?.overlays.find((x) => x.id === id);
    if (o) post(f, 'seek', clock.t - o.at);
  }
}

/* ── side panel: inspector, text, AI ────────────────────────── */

function buildSide() {
  sideEl = h('aside', 'capture-ed-side');
  return sideEl;
}

function renderSide() {
  if (!sideEl || !project) return;
  sideEl.textContent = '';
  const strip = tabStrip({ label: t('plugin.capture.editor.panels', null, 'Panels'), className: 'capture-ed-tabs', windowBar: false });
  for (const [id, label] of [['inspect', t('plugin.capture.editor.inspect', null, 'Clip')], ['text', t('plugin.capture.editor.text', null, 'Text')], ['ai', t('plugin.capture.editor.ai', null, 'AI')]]) {
    strip.appendChild(tab({ label, active: sideTab === id, onSelect: () => { sideTab = id; renderSide(); } }));
  }
  const pending = project.proposals?.length || 0;
  if (pending) strip.lastChild.dataset.badge = String(pending);
  const body = h('div', 'capture-ed-side-body');
  sideEl.append(strip, body);
  if (sideTab === 'inspect') renderInspector(body);
  else if (sideTab === 'text') renderText(body);
  else renderAI(body);
}

function num(label, value, { min = 0, max = 1, step = 0.01, fmt = (v) => v, onChange }) {
  const s = slider({ min, max, step, value, onInput: (v) => { out.textContent = fmt(v); } });
  s.addEventListener('change', () => onChange(Number(s.value)));
  s.setAttribute('aria-label', label);
  const out = h('span', 'capture-num-out', fmt(value));
  const row = h('div', 'capture-num');
  row.append(s, out);
  return field({ label, control: row });
}

function renderInspector(body) {
  const item = selectedId && itemsFromProject().find((x) => x.id === selectedId);
  if (!item) {
    body.appendChild(emptyState({ icon: 'ui/properties', title: t('plugin.capture.editor.noSelection', null, 'Select a clip'), body: t('plugin.capture.editor.noSelectionHint', null, 'Speed, volume, fades and placement show here.') }));
    return;
  }
  if (item.kind === 'clip') {
    const c = project.clips.find((x) => x.id === item.id);
    const tr = track(c.track);
    body.append(h('h3', 'capture-ed-h', media(c.media)?.label || ''));
    const speeds = [0.25, 0.5, 1, 1.5, 2, 4, 8];
    const sp = segmented({
      label: t('plugin.capture.editor.speed', null, 'Speed'),
      value: String(speeds.includes(c.speed) ? c.speed : 1),
      options: speeds.map((v) => ({ value: String(v), label: `${v}×` })),
      onChange: (v) => ops([{ op: 'set_clip', clip: c.id, speed: Number(v) }]),
    });
    body.append(field({ label: t('plugin.capture.editor.speed', null, 'Speed'), control: sp }));
    body.append(num(t('plugin.capture.editor.volume', null, 'Volume'), c.volume, { max: 2, fmt: (v) => `${Math.round(v * 100)}%`, onChange: (v) => ops([{ op: 'set_clip', clip: c.id, volume: v }]) }));
    body.append(num(t('plugin.capture.editor.fadeIn', null, 'Fade in'), c.fade_in, { max: 5, step: 0.1, fmt: (v) => `${Number(v).toFixed(1)} s`, onChange: (v) => ops([{ op: 'set_clip', clip: c.id, fade_in: v }]) }));
    body.append(num(t('plugin.capture.editor.fadeOut', null, 'Fade out'), c.fade_out, { max: 5, step: 0.1, fmt: (v) => `${Number(v).toFixed(1)} s`, onChange: (v) => ops([{ op: 'set_clip', clip: c.id, fade_out: v }]) }));
    if (tr?.kind === 'video') {
      const presets = {
        full: { x: 0, y: 0, w: 1, h: 1 },
        br: { x: 0.74, y: 0.66, w: 0.22, h: 0.22 * (project.width / project.height) },
        bl: { x: 0.04, y: 0.66, w: 0.22, h: 0.22 * (project.width / project.height) },
        left: { x: 0, y: 0, w: 0.5, h: 1 },
        right: { x: 0.5, y: 0, w: 0.5, h: 1 },
      };
      const pl = segmented({
        label: t('plugin.capture.editor.place', null, 'Placement'),
        value: Object.entries(presets).find(([, r]) => Math.abs(r.x - c.rect.x) < 0.01 && Math.abs(r.w - c.rect.w) < 0.01)?.[0] || '',
        options: [
          { value: 'full', label: t('plugin.capture.editor.full', null, 'Full') },
          { value: 'br', label: t('plugin.capture.editor.cornerR', null, 'Corner ↘') },
          { value: 'bl', label: t('plugin.capture.editor.cornerL', null, 'Corner ↙') },
          { value: 'left', label: t('plugin.capture.editor.half', null, 'Half') },
        ],
        onChange: (v) => ops([{ op: 'set_clip', clip: c.id, rect: presets[v], ...(v === 'br' || v === 'bl' ? { radius: 1 } : {}) }]),
      });
      body.append(field({ label: t('plugin.capture.editor.place', null, 'Placement'), control: pl }));
      body.append(num(t('plugin.capture.editor.radius', null, 'Corner roundness'), c.radius, { fmt: (v) => `${Math.round(v * 100)}%`, onChange: (v) => ops([{ op: 'set_clip', clip: c.id, radius: v }]) }));
      body.append(num(t('plugin.capture.editor.opacity', null, 'Opacity'), c.opacity, { fmt: (v) => `${Math.round(v * 100)}%`, onChange: (v) => ops([{ op: 'set_clip', clip: c.id, opacity: v }]) }));
    }
    const m = media(c.media);
    if (tr?.kind === 'audio' && (m?.tracks?.length || 0) > 1) {
      const sel = select({
        options: m.tracks.map((name, i) => ({ value: String(i), label: t(`plugin.capture.editor.track.${name}`, null, name) })),
        value: String(c.audio_track),
        onChange: (v) => ops([{ op: 'set_clip', clip: c.id, audio_track: Number(v) }]),
      });
      body.append(field({ label: t('plugin.capture.editor.source', null, 'Sound source'), control: sel }));
    }
    const actions = h('div', 'capture-ed-actions');
    actions.append(
      button({ icon: 'ui/scissors', label: t('plugin.capture.editor.split', null, 'Split at the playhead'), variant: 'ghost', size: 'sm', onClick: () => splitAtPlayhead(c.id) }),
      button({ icon: 'ui/trash', label: t('plugin.capture.editor.delete', null, 'Delete'), variant: 'ghost', size: 'sm', onClick: () => deleteItem(c.id) }),
    );
    body.append(actions);
  } else if (item.kind === 'overlay') {
    const o = project.overlays.find((x) => x.id === item.id);
    const name = input({ value: o.name || '' });
    name.addEventListener('change', () => ops([{ op: 'update_overlay', id: o.id, name: name.value }]));
    const code = textarea({ value: o.html });
    code.classList.add('capture-code');
    code.spellcheck = false;
    const apply = button({ icon: 'ui/check', label: t('plugin.capture.editor.applyHtml', null, 'Apply HTML'), size: 'sm', onClick: () => ops([{ op: 'update_overlay', id: o.id, html: code.value }]) });
    const ask = input({ placeholder: t('plugin.capture.editor.askGraphic', null, 'Ask the AI to change this graphic…') });
    ask.addEventListener('keydown', (e) => {
      if (e.key !== 'Enter' || !ask.value.trim()) return;
      askInChat(t('plugin.capture.ai.changeGraphic', { name: o.name || o.id, id: o.id, project: project.id, text: ask.value.trim() },
        `In the Capture project ${project.id}, change the graphic "${o.name || o.id}" (id ${o.id}): ${ask.value.trim()}`));
      ask.value = '';
    });
    body.append(field({ label: t('plugin.capture.editor.name', null, 'Name'), control: name }), ask, field({ label: 'HTML', control: code }), apply);
    if (o.ghost) body.prepend(h('p', 'capture-hint', t('plugin.capture.editor.ghostHint', null, 'Proposed by the AI: accept it in the AI panel.')));
  } else {
    const c = project.captions.find((x) => x.id === item.id);
    const text = textarea({ value: c.text });
    text.addEventListener('change', () => send({ project: { ...project, captions: project.captions.map((x) => (x.id === c.id ? { ...x, text: text.value } : x)) } }));
    body.append(field({ label: t('plugin.capture.editor.caption', null, 'Caption'), control: text }));
  }
}

async function renderText(body) {
  body.appendChild(spinner());
  try {
    summary = (await apiFetch(`/api/capture/projects/${encodeURIComponent(project.id)}/summary`))?.data || null;
  } catch (_) { summary = null; }
  body.textContent = '';
  const segs = summary?.timeline_transcript || [];
  const running = (summary?.transcribing || []).filter((s) => s.state?.state === 'running');
  if (!segs.length) {
    const failed = (summary?.transcribing || []).find((s) => s.state?.state === 'failed');
    body.append(emptyState({
      icon: 'ui/type',
      title: running.length ? t('plugin.capture.text.running', null, 'Transcribing…') : t('plugin.capture.text.none', null, 'No transcript yet'),
      body: failed ? failed.state.error : t('plugin.capture.text.hint', null, 'Transcribe the voice to edit the video by editing its text, and to let the AI follow what you say.'),
    }));
    if (!running.length) {
      body.append(button({ icon: 'ui/type', label: t('plugin.capture.text.transcribe', null, 'Transcribe'), size: 'sm', onClick: async () => {
        await apiFetch(`/api/capture/projects/${encodeURIComponent(project.id)}/transcribe`, { method: 'POST', body: '{}' }).catch((e) => toast(e.message, { type: 'error' }));
        renderSide();
      } }));
    } else {
      setTimeout(() => { if (sideTab === 'text') renderSide(); }, 3000);
    }
    return;
  }
  const chosen = new Set();
  const list = h('div', 'capture-transcript');
  const cut = button({ icon: 'ui/scissors', label: t('plugin.capture.text.cut', null, 'Cut the selected lines'), size: 'sm', variant: 'ghost', onClick: () => {
    const ranges = segs.filter((s) => chosen.has(s[0])).map((s) => [s[1], s[2]]);
    if (ranges.length) ops([{ op: 'cut_ranges', ranges }]);
  } });
  cut.disabled = true;
  for (const s of segs) {
    const row = h('div', 'capture-line');
    const time = h('button', 'capture-line-time', formatTimecode(s[1]));
    time.addEventListener('click', () => seek(s[1]));
    const txt = h('span', 'capture-line-text', s[3]);
    row.append(time, txt);
    row.addEventListener('click', (e) => {
      if (e.target === time) return;
      if (chosen.has(s[0])) chosen.delete(s[0]); else chosen.add(s[0]);
      row.classList.toggle('is-chosen', chosen.has(s[0]));
      cut.disabled = !chosen.size;
    });
    list.appendChild(row);
  }
  body.append(h('p', 'capture-hint', t('plugin.capture.text.how', null, 'Click lines to select them, then cut: the video follows the text.')), cut, list);
}

const GOALS = ['tutorial', 'vlog', 'short', 'clean'];

function renderAI(body) {
  // Proposals.
  const props = project.proposals || [];
  if (props.length) {
    body.append(h('h3', 'capture-ed-h', t('plugin.capture.ai.proposals', null, 'Proposals')));
    for (const p of props) {
      const card = h('div', 'capture-proposal');
      card.append(h('p', 'capture-proposal-text', p.summary || t('plugin.capture.ai.untitled', null, 'Proposal')),
        h('span', 'capture-hint', t('plugin.capture.ai.ops', { n: p.ops.length }, `${p.ops.length} changes`)));
      const refine = input({ placeholder: t('plugin.capture.ai.refinePh', null, 'Refine: shorter, warmer colours, from the left…') });
      refine.addEventListener('keydown', (e) => {
        if (e.key !== 'Enter' || !refine.value.trim()) return;
        askInChat(t('plugin.capture.ai.refine', { id: p.id, project: project.id, text: refine.value.trim() },
          `Refine proposal ${p.id} of the Capture project ${project.id}: ${refine.value.trim()}`));
        refine.value = '';
      });
      const row = h('div', 'capture-ed-actions');
      row.append(
        button({ icon: 'ui/check', label: t('plugin.capture.ai.accept', null, 'Accept'), size: 'sm', onClick: () => send({ accept: p.id }) }),
        button({ icon: 'ui/close', label: t('plugin.capture.ai.reject', null, 'Reject'), size: 'sm', variant: 'ghost', onClick: () => send({ reject: p.id }) }),
      );
      card.append(row, refine);
      body.append(card);
    }
  }
  // Ask.
  const ask = textarea({ placeholder: t('plugin.capture.ai.askPh', null, 'Tell the AI what to do: "remove the part where I look for the file", "an animated title at the start", "speed up the installation 4×"…') });
  ask.rows = 3;
  const send1 = button({ icon: 'ui/send', label: t('plugin.capture.ai.send', null, 'Ask'), size: 'sm', onClick: () => {
    const text = ask.value.trim();
    if (!text) return;
    askInChat(t('plugin.capture.ai.request', { project: project.id, title: project.title, text }, `Capture project "${project.title}" (id ${project.id}): ${text}`));
    ask.value = '';
  } });
  body.append(h('h3', 'capture-ed-h', t('plugin.capture.ai.ask', null, 'Ask the AI')), ask, send1);

  // Edit everything.
  let goal = 'tutorial';
  const goals = segmented({
    label: t('plugin.capture.ai.goal', null, 'Goal'),
    value: goal,
    options: GOALS.map((g) => ({ value: g, label: t(`plugin.capture.ai.goal.${g}`, null, g) })),
    onChange: (v) => { goal = v; },
  });
  const guide = textarea({ placeholder: t('plugin.capture.ai.guidePh', null, 'Guidelines (optional): pace, tone, what to keep…') });
  guide.rows = 2;
  const all = button({ icon: 'ui/magic-wand', label: t('plugin.capture.ai.editAll', null, 'Edit everything'), size: 'sm', onClick: () => {
    const g = t(`plugin.capture.ai.goal.${goal}`, null, goal);
    askInChat(t('plugin.capture.ai.editAllMsg', { project: project.id, title: project.title, goal: g, guide: guide.value.trim() || '-' },
      `Edit everything in the Capture project "${project.title}" (id ${project.id}). Goal: ${g}. Guidelines: ${guide.value.trim() || '-'}. Work in passes with capture_edit and tell me what you did.`));
  } });
  body.append(h('h3', 'capture-ed-h', t('plugin.capture.ai.editAllTitle', null, 'Edit everything')), goals, guide, all);

  // Style.
  const st = project.style || {};
  body.append(h('h3', 'capture-ed-h', t('plugin.capture.ai.style', null, 'Graphics style')));
  if (st.guide) body.append(h('p', 'capture-style-guide', st.guide));
  else body.append(h('p', 'capture-hint', t('plugin.capture.ai.styleEmpty', null, 'The AI writes the style after the first graphic you keep, then every graphic follows it.')));
  const user = textarea({ value: st.user || '', placeholder: t('plugin.capture.ai.stylePh', null, 'Your guidelines for every graphic: colours, fonts, mood…') });
  user.rows = 2;
  user.addEventListener('change', () => send({ project: { ...project, style: { ...st, user: user.value } } }));
  const lock = toggleRow({ label: t('plugin.capture.ai.lock', null, 'Keep this style'), checked: !!st.locked, onChange: (v) => send({ project: { ...project, style: { ...st, locked: v } } }) });
  const fresh = button({ icon: 'ui/refresh', label: t('plugin.capture.ai.newStyle', null, 'Something completely different'), variant: 'ghost', size: 'sm', onClick: () => {
    askInChat(t('plugin.capture.ai.newStyleMsg', { project: project.id }, `For the Capture project ${project.id}, design a completely different graphics style, write it with capture_style and propose one graphic in it.`));
  } });
  body.append(user, lock, fresh);
}

/* ── bar ────────────────────────────────────────────────────── */

async function undo() {
  const v = project?.versions?.[0];
  if (!v) return;
  await send({ restore: v });
}

async function importMedia() {
  const picked = await pickPaths({ accept: 'video/*,audio/*,image/*', multiple: true });
  const paths = (picked || []).map((e) => e.path).filter(Boolean);
  if (paths.length) await send({ import: paths });
}

function addCaption() {
  send({ ops: [{ op: 'add_caption', at: clock.t, dur: 2.5, text: t('plugin.capture.editor.newCaption', null, 'New caption') }] });
}

function exportMenu(anchor) {
  const surface = menuSurface({ className: 'capture-menu', onEscape: () => surface.remove() });
  const presets = [['source', t('plugin.capture.export.source', null, 'Same as the project')], ['1080p', '1080p'], ['720p', '720p'], ['vertical', t('plugin.capture.export.vertical', null, 'Vertical 9:16 (Shorts, Reels)')], ['square', t('plugin.capture.export.square', null, 'Square 1:1')], ['gif', 'GIF']];
  for (const [id, label] of presets) {
    surface.appendChild(menuItem({ label, onClick: () => { surface.remove(); runExport(id); } }));
  }
  surface.classList.remove('hidden');
  document.body.appendChild(surface);
  const r = anchor.getBoundingClientRect();
  surface.style.left = `${r.left}px`;
  surface.style.top = `${r.bottom + 4}px`;
  surface.focusFirst?.();
  setTimeout(() => document.addEventListener('pointerdown', function off(ev) {
    if (!surface.contains(ev.target)) { surface.remove(); document.removeEventListener('pointerdown', off, true); }
  }, true));
}

async function runExport(preset) {
  try {
    const r = await apiFetch(`/api/capture/projects/${encodeURIComponent(project.id)}/export`, { method: 'POST', body: JSON.stringify({ preset }) });
    const job = r?.data?.job;
    if (!job) return;
    const note = toast(t('plugin.capture.export.started', null, 'Exporting…'), { duration: 3000 });
    void note;
    const poll = async () => {
      const j = (await apiFetch(`/api/capture/jobs/${job}`).catch(() => null))?.data;
      if (!j) return;
      if (exportBtn) exportBtn.dataset.progress = j.state === 'done' ? '' : `${Math.round(j.progress * 100)}%`;
      if (j.state === 'done') {
        notify({ app: 'Capture', title: t('plugin.capture.export.done', null, 'Video exported'), body: j.output_name, icon: 'ui/video', plugin: 'capture', urgency: 'low' });
        if (j.warnings?.length) toast(j.warnings.join(' · '), { type: 'warning', duration: 9000 });
        return;
      }
      if (j.state === 'failed') {
        toast(t('plugin.capture.export.failed', { error: j.error || '' }, `Export failed: ${j.error || ''}`), { type: 'error', duration: 9000 });
        return;
      }
      setTimeout(poll, 700);
    };
    poll();
  } catch (e) {
    toast(e.message || String(e), { type: 'error' });
  }
}

let exportBtn = null;

function buildBar() {
  titleInput = input({ value: '' });
  titleInput.classList.add('capture-ed-title');
  titleInput.setAttribute('aria-label', t('plugin.capture.editor.title', null, 'Project title'));
  titleInput.addEventListener('change', () => ops([{ op: 'set_project', title: titleInput.value.trim() || project.title }]));
  exportBtn = button({ icon: 'ui/download', label: t('plugin.capture.export.label', null, 'Export'), size: 'sm', onClick: (e) => exportMenu(e.currentTarget) });
  const bar = toolbar([
    titleInput,
    toolButton('ui/undo', t('plugin.capture.editor.undo', null, 'Undo'), undo),
    toolButton('ui/scissors', t('plugin.capture.editor.split', null, 'Split at the playhead'), () => splitAtPlayhead()),
    toolButton('ui/plus', t('plugin.capture.editor.import', null, 'Add media'), importMedia),
    toolButton('ui/type', t('plugin.capture.editor.addCaption', null, 'Add a caption'), addCaption),
    toolButton('ui/magic-wand', t('plugin.capture.editor.ai', null, 'AI'), () => { sideTab = 'ai'; renderSide(); }),
    h('span', 'capture-bar-spacer'),
    exportBtn,
  ], { className: 'capture-ed-bar', label: t('plugin.capture.editor.label', null, 'Editor') });
  return bar;
}

/* ── render ─────────────────────────────────────────────────── */

let emptyEl = null;

function render() {
  if (!root) return;
  const has = !!project;
  emptyEl.hidden = has;
  root.querySelector('.capture-ed-work').hidden = !has;
  if (!has) return;
  if (document.activeElement !== titleInput) titleInput.value = project.title;
  tl.setTracks(project.tracks.map((x) => ({ id: x.id, label: x.name || x.id, kind: x.kind, muted: x.muted, hidden: x.hidden, height: x.kind === 'audio' ? 48 : OVERLAY_TRACK_KINDS.has(x.kind) ? 34 : 56 })));
  tl.setItems(itemsFromProject());
  tl.setDuration(duration());
  fitStage();
  syncPlayers(clock.t, true);
  renderSide();
}

function mount() {
  root = h('div', 'capture-editor');
  emptyEl = emptyState({ icon: 'ui/scissors', title: t('plugin.capture.editor.empty', null, 'Open a project'), body: t('plugin.capture.editor.emptyHint', null, 'Pick a recording in the Library, or start a project and add any video, sound or picture.') });
  const work = h('div', 'capture-ed-work');
  const main = h('div', 'capture-ed-main');
  main.append(buildStage(), buildSide());
  work.append(buildBar(), main, buildTimeline());
  root.append(emptyEl, work);
  work.hidden = true;
  return root;
}

async function open(id) {
  pause();
  clock = { t: 0, at: 0, from: 0 };
  selectedId = null;
  for (const el of players.values()) el.remove();
  players.clear();
  for (const f of frames.values()) f.remove();
  frames.clear();
  try {
    await load(id);
    tl.zoomToFit();
    seek(0);
    startPolling();
  } catch (e) {
    toast(e.message || String(e), { type: 'error' });
  }
}

function show() {
  render();
}

function unmount() {
  pause();
  stopPolling();
  for (const el of players.values()) el.remove();
  players.clear();
  frames.clear();
  tl?.destroy();
  tl = null;
  root?.remove();
  root = null;
  project = null;
}

export default { mount, open, show, unmount };
