import {
  element as h, toolbar, toolButton, button,
  segmented, slider, toggle, toggleRow, field, input, select, controlGroup, levelMeter, icon,
  emptyState, spinner, toast, notify, confirmDialog, modal, paneStack, formatTimecode, setQuiet,
} from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';
import { showBubble, hideBubble, onBubbleChange, toScreenRect, bubbleRect } from './bubble.js';
import { pickWindow, pickRegion, tileRect, tileTitle } from './picker.js';

/**
 * capture/plugin.js — the Capture window: record, go live, library, editor.
 *
 * Views (the segmented control in the window bar):
 *   - Record: preview, source (screen / one window / a region), webcam
 *     bubble, audio with meters, live destinations, quality, REC and LIVE;
 *   - Library: recordings and projects in Videos/LiteAura;
 *   - Edit: the video editor (editor.js), loaded on first use.
 *
 * The engine runs in the plugin (Rust): this window only says what to
 * record and where the window or the bubble is, in screen pixels, while
 * they move. Settings persist per user through /api/capture/settings.
 */

export const CAPTURE_PLUGIN = 'capture';

/** A shared modal with a row of actions under its body. */
export function dialog(title, body, actions = [], { wide = false, onClose } = {}) {
  const parts = [body];
  if (actions.length) {
    const row = h('div', 'capture-dialog-actions');
    row.append(...actions);
    parts.push(row);
  }
  const m = modal({ title, body: parts, wide, onClose, removeOnClose: true });
  return m;
}

const DEFAULTS = {
  source: 'screen',
  fps: 30,
  quality: 'high',
  cursor: true,
  mic: true,
  micDevice: 'default',
  desktop: true,
  desktopDevice: 'default',
  camera: false,
  cameraDevice: '',
  bubble: { size: 240, shape: 'circle', roundness: 1, mirror: true, border: 0, x: null, y: null },
  live: [],
  record: true,
};
const QUALITY_QP = { high: 18, balanced: 22, small: 27 };

let tileEl = null;
let viewEl = null;
let views = {};
let currentView = 'record';
let settings = structuredClone(DEFAULTS);
let devices = null;
let destinations = [];
let platforms = [];
let status = { active: false, state: 'idle' };
let pollTimer = 0;
let meterTimer = 0;
let saveTimer = 0;
let previewImg = null;
let statsEl = null;
let timerEl = null;
let recBtn = null;
let liveBtn = null;
let pauseBtn = null;
let stopBtn = null;
let windowTarget = null;      // the tile being recorded (source = window)
let regionRect = null;        // screen pixels (source = region)
let trackFrame = 0;
let lastSentRect = '';
let editorMod = null;
let meters = {};
const disposers = [];

/* ── settings ───────────────────────────────────────────────── */

async function loadSettings() {
  try {
    const r = await apiFetch('/api/capture/settings');
    settings = { ...structuredClone(DEFAULTS), ...(r?.data || {}) };
    settings.bubble = { ...DEFAULTS.bubble, ...(r?.data?.bubble || {}) };
  } catch (_) { /* defaults */ }
}

function saveSettings() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    apiFetch('/api/capture/settings', { method: 'PUT', body: JSON.stringify(settings) }).catch(() => {});
  }, 400);
}

function set(key, value) {
  settings[key] = value;
  saveSettings();
}

/* ── status polling ─────────────────────────────────────────── */

const isActive = () => !!status.active;

async function poll() {
  try {
    const r = await apiFetch('/api/capture/status');
    const prev = status;
    status = r?.data || { active: false };
    if (prev.active && !status.active) onFinished(status);
    if (!prev.active && status.active) setQuiet(true, CAPTURE_PLUGIN);
    renderStatus();
  } catch (_) { /* server away: try again */ }
  pollTimer = setTimeout(poll, isActive() ? 500 : 2500);
}

function restartPoll() {
  clearTimeout(pollTimer);
  poll();
}

function onFinished(st) {
  setQuiet(false, CAPTURE_PLUGIN);
  stopTracking();
  if (st.state === 'failed') {
    toast(t('plugin.capture.toast.failed', { error: st.error || '' }, `Recording failed: ${st.error || ''}`), { type: 'error', duration: 8000 });
    return;
  }
  if (st.output_id) {
    notify({
      app: 'Capture',
      title: t('plugin.capture.notify.saved', null, 'Recording saved'),
      body: st.output_id,
      icon: 'ui/video',
      plugin: CAPTURE_PLUGIN,
      urgency: 'low',
    });
    if (currentView === 'library') renderLibrary();
  }
}

/* ── geometry: what to record, in screen pixels ─────────────── */

function sourceRect() {
  if (settings.source === 'window' && windowTarget?.isConnected) return toScreenRect(tileRect(windowTarget));
  if (settings.source === 'region' && regionRect) return regionRect;
  return null;
}

function cameraBody() {
  if (!settings.camera) return null;
  const r = bubbleRect();
  if (!r) return null;
  const b = settings.bubble;
  return {
    device: settings.cameraDevice || null,
    rect: toScreenRect(r),
    style: {
      roundness: b.roundness,
      mirror: !!b.mirror,
      border: Math.round((b.border || 0) * (window.devicePixelRatio || 1)),
      border_color: getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#ffffff',
      zoom: 1,
    },
  };
}

/** While recording, follow the window and the bubble as they move. */
function startTracking() {
  stopTracking();
  let last = 0;
  const tick = (now) => {
    trackFrame = requestAnimationFrame(tick);
    if (now - last < 100 || !isActive()) return;
    last = now;
    const body = {};
    const r = sourceRect();
    if (settings.source === 'window' && r) body.rect = r;
    const cam = cameraBody();
    body.camera = cam;
    const key = JSON.stringify(body);
    if (key === lastSentRect) return;
    lastSentRect = key;
    apiFetch('/api/capture/update', { method: 'POST', body: key }).catch(() => {});
  };
  trackFrame = requestAnimationFrame(tick);
}

function stopTracking() {
  if (trackFrame) cancelAnimationFrame(trackFrame);
  trackFrame = 0;
  lastSentRect = '';
}

/* ── start / stop ───────────────────────────────────────────── */

async function start({ live = false, record = true } = {}) {
  if (isActive()) return;
  if (settings.source === 'window' && !windowTarget?.isConnected) {
    const tile = await pickWindow();
    if (!tile) return;
    windowTarget = tile;
  }
  if (settings.source === 'region' && !regionRect) {
    const r = await pickRegion();
    if (!r) return;
    regionRect = r;
  }
  const liveIds = live ? destinations.filter((d) => settings.live.includes(d.id)).map((d) => d.id) : [];
  if (live && !liveIds.length) {
    toast(t('plugin.capture.toast.noDestination', null, 'Turn on at least one live destination first'), { type: 'warning' });
    showPanel('live');
    return;
  }
  const body = {
    source: settings.source,
    rect: sourceRect(),
    window_title: settings.source === 'window' ? tileTitle(windowTarget) : null,
    fps: settings.fps,
    cursor: settings.cursor,
    mic: settings.mic ? settings.micDevice || 'default' : null,
    desktop: settings.desktop ? settings.desktopDevice || 'default' : null,
    camera: cameraBody(),
    record: live ? !!settings.record : record,
    live: liveIds,
    quality: QUALITY_QP[settings.quality] ?? 20,
  };
  try {
    recBtn?.setLoading?.(true);
    await apiFetch('/api/capture/start', { method: 'POST', body: JSON.stringify(body) });
    setQuiet(true, CAPTURE_PLUGIN);
    startTracking();
    restartPoll();
    window.dispatchEvent(new CustomEvent('capture:changed'));
  } catch (e) {
    toast(e.message || String(e), { type: 'error', duration: 8000 });
  } finally {
    recBtn?.setLoading?.(false);
  }
}

async function stop() {
  try { await apiFetch('/api/capture/stop', { method: 'POST', body: '{}' }); } catch (_) { /* noop */ }
  restartPoll();
  window.dispatchEvent(new CustomEvent('capture:changed'));
}

async function togglePause() {
  try { await apiFetch('/api/capture/pause', { method: 'POST', body: '{}' }); } catch (_) { /* noop */ }
  restartPoll();
}

export async function addMarker(label = '') {
  try { await apiFetch('/api/capture/marker', { method: 'POST', body: JSON.stringify({ label }) }); } catch (_) { /* noop */ }
}

/* ── window bar ─────────────────────────────────────────────── */

function buildBar() {
  const viewSwitch = segmented({
    label: t('plugin.capture.view.label', null, 'View'),
    value: currentView,
    options: [
      { value: 'record', label: t('plugin.capture.view.record', null, 'Record'), icon: 'ui/record' },
      { value: 'library', label: t('plugin.capture.view.library', null, 'Library'), icon: 'ui/folder-open' },
      { value: 'edit', label: t('plugin.capture.view.edit', null, 'Edit'), icon: 'ui/scissors' },
    ],
    onChange: (v) => showView(v),
  });
  viewSwitch.classList.add('capture-views');
  timerEl = h('span', 'capture-timer');
  timerEl.textContent = '0:00';
  recBtn = button({
    icon: 'ui/record', label: t('plugin.capture.action.record', null, 'Record'), variant: 'primary', size: 'sm',
    onClick: () => (isActive() ? stop() : start({ record: true })),
  });
  recBtn.classList.add('capture-rec');
  recBtn.dataset.keep = '';
  liveBtn = button({
    icon: 'ui/broadcast', label: t('plugin.capture.action.live', null, 'Go live'), variant: 'ghost', size: 'sm',
    onClick: () => (isActive() ? stop() : start({ live: true })),
  });
  liveBtn.classList.add('capture-live');
  pauseBtn = toolButton('ui/pause', t('plugin.capture.action.pause', null, 'Pause'), togglePause);
  stopBtn = toolButton('ui/stop', t('plugin.capture.action.stop', null, 'Stop'), stop);
  const markBtn = toolButton('ui/pin', t('plugin.capture.action.marker', null, 'Add marker'), () => addMarker());
  markBtn.classList.add('capture-mark');
  const bar = toolbar([viewSwitch, h('span', 'capture-bar-spacer'), timerEl, markBtn, pauseBtn, stopBtn, liveBtn, recBtn], {
    className: 'capture-bar', label: t('plugin.capture.title', null, 'Capture'),
  });
  bar.dataset.windowBar = '';
  return bar;
}

function renderStatus() {
  if (!tileEl) return;
  const active = isActive();
  const paused = !!status.paused;
  tileEl.classList.toggle('is-recording', active && !status.live);
  tileEl.classList.toggle('is-live', active && !!status.live);
  tileEl.classList.toggle('is-paused', paused);
  timerEl.textContent = formatTimecode((status.elapsed_us || 0) / 1e6);
  timerEl.hidden = !active;
  pauseBtn.hidden = !active || !!status.live;
  stopBtn.hidden = !active;
  tileEl.querySelector('.capture-mark').hidden = !active;
  recBtn.hidden = active;
  liveBtn.hidden = active;
  pauseBtn.querySelector('.ui-icon')?.replaceWith(icon(paused ? 'ui/play' : 'ui/pause', { size: 16 }));
  pauseBtn.title = paused ? t('plugin.capture.action.resume', null, 'Resume') : t('plugin.capture.action.pause', null, 'Pause');
  if (previewImg) {
    if (active) {
      previewImg.src = `/api/capture/preview.jpg?t=${Date.now()}`;
      previewImg.hidden = false;
    } else {
      previewImg.hidden = true;
    }
  }
  if (statsEl) {
    if (active) {
      const hl = status.health || {};
      const bits = [
        `${status.width}×${status.height}`,
        `${status.fps} fps`,
        status.encoder,
        t('plugin.capture.stats.cpu', { n: Math.round((status.cpu || 0) + (status.encoder_cpu || 0)) }, `CPU ${Math.round((status.cpu || 0) + (status.encoder_cpu || 0))}%`),
      ];
      if (status.live) bits.push(`${(status.live_labels || []).join(', ')}`);
      if (hl.dropped) bits.push(t('plugin.capture.stats.dropped', { n: hl.dropped }, `${hl.dropped} dropped`));
      statsEl.textContent = bits.filter(Boolean).join(' · ');
      const err = (hl.errors || []).slice(-1)[0];
      statsEl.title = err || '';
    } else {
      statsEl.textContent = describeSource();
    }
  }
}

function describeSource() {
  if (settings.source === 'window') {
    return windowTarget?.isConnected
      ? t('plugin.capture.source.windowIs', { name: tileTitle(windowTarget) }, `Window: ${tileTitle(windowTarget)}`)
      : t('plugin.capture.source.windowPick', null, 'You will pick the window when you start');
  }
  if (settings.source === 'region') {
    return regionRect
      ? t('plugin.capture.source.regionIs', { w: regionRect.w, h: regionRect.h }, `Region ${regionRect.w}×${regionRect.h}`)
      : t('plugin.capture.source.regionPick', null, 'You will draw the region when you start');
  }
  return t('plugin.capture.source.screenIs', null, 'The whole screen');
}

/* ── record view ────────────────────────────────────────────── */

let panelEls = {};
let repickBtn = null;

function showPanel(id) {
  panelEls[id]?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  panelEls[id]?.classList.add('is-flash');
  setTimeout(() => panelEls[id]?.classList.remove('is-flash'), 900);
}

function buildRecordView() {
  const root = h('div', 'capture-record');
  const stage = h('section', 'capture-stage');
  const frame = h('div', 'capture-preview');
  previewImg = document.createElement('img');
  previewImg.className = 'capture-preview-img';
  previewImg.alt = '';
  previewImg.hidden = true;
  const idle = h('div', 'capture-preview-idle');
  idle.appendChild(icon('ui/monitor', { size: 44 }));
  frame.append(previewImg, idle);
  statsEl = h('p', 'capture-stats');
  const openSide = button({ icon: 'ui/settings', label: t('plugin.capture.settings', null, 'Settings'), variant: 'ghost', size: 'sm', onClick: () => nav?.show('side') });
  openSide.classList.add('capture-open-side');
  const foot = h('div', 'capture-stage-foot');
  foot.append(statsEl, openSide);
  stage.append(frame, foot);

  const side = h('aside', 'capture-side');
  const sideBar = h('div', 'capture-side-bar');
  sideBar.dataset.paneBar = '';
  sideBar.appendChild(h('strong', null, t('plugin.capture.settings', null, 'Settings')));
  side.append(sideBar, sourcePanel(), cameraPanel(), audioPanel(), livePanel(), qualityPanel());
  root.append(stage, side);
  const nav = paneStack(root, { panes: [{ id: 'stage', el: stage }, { id: 'side', el: side }] });
  disposers.push(() => nav?.destroy?.());
  return root;
}

function group(id, heading, children) {
  const g = controlGroup({ heading: h('h3', 'capture-group-title', heading), children });
  g.classList.add('capture-group');
  panelEls[id] = g;
  return g;
}

function sourcePanel() {
  const seg = segmented({
    label: t('plugin.capture.source.label', null, 'Source'),
    value: settings.source,
    options: [
      { value: 'screen', label: t('plugin.capture.source.screen', null, 'Screen'), icon: 'ui/monitor' },
      { value: 'window', label: t('plugin.capture.source.window', null, 'Window'), icon: 'ui/window' },
      { value: 'region', label: t('plugin.capture.source.region', null, 'Region'), icon: 'ui/marquee-rect' },
    ],
    onChange: async (v) => {
      set('source', v);
      if (repickBtn) repickBtn.hidden = v === 'screen';
      if (v === 'window') {
        const tile = await pickWindow();
        if (tile) windowTarget = tile;
      } else if (v === 'region') {
        const r = await pickRegion();
        if (r) regionRect = r;
      }
      renderStatus();
    },
  });
  disposers.push(() => seg.destroy?.());
  const repick = button({ icon: 'ui/crop', label: t('plugin.capture.source.repick', null, 'Choose again'), variant: 'ghost', size: 'sm', onClick: async () => {
    if (settings.source === 'window') { const tile = await pickWindow(); if (tile) windowTarget = tile; }
    if (settings.source === 'region') { const r = await pickRegion(); if (r) regionRect = r; }
    renderStatus();
  } });
  repick.hidden = settings.source === 'screen';
  repickBtn = repick;
  const cursor = toggleRow({
    label: t('plugin.capture.source.cursor', null, 'Show the pointer'),
    checked: settings.cursor,
    onChange: (v) => set('cursor', v),
  });
  return group('source', t('plugin.capture.source.label', null, 'Source'), [seg, repick, cursor]);
}

function shapeRoundness(shape) {
  return shape === 'square' ? 0 : shape === 'rounded' ? 0.35 : 1;
}

function cameraPanel() {
  const b = settings.bubble;
  const enable = toggleRow({
    label: t('plugin.capture.camera.enable', null, 'Webcam'),
    checked: settings.camera,
    onChange: (v) => {
      set('camera', v);
      if (v) openBubble(); else hideBubble();
      body.hidden = !v;
    },
  });
  const body = h('div', 'capture-camera-body');
  body.hidden = !settings.camera;
  const devSel = select({
    options: (devices?.cameras || []).map((c) => ({ value: c.path, label: c.name || c.path })),
    value: settings.cameraDevice,
    onChange: (e) => { set('cameraDevice', e.target ? e.target.value : e); if (settings.camera) openBubble(); },
  });
  const shape = segmented({
    label: t('plugin.capture.camera.shape', null, 'Shape'),
    value: b.shape || 'circle',
    options: [
      { value: 'square', label: t('plugin.capture.camera.square', null, 'Square'), icon: 'ui/shape' },
      { value: 'rounded', label: t('plugin.capture.camera.rounded', null, 'Rounded'), icon: 'ui/marquee-rect' },
      { value: 'circle', label: t('plugin.capture.camera.circle', null, 'Circle'), icon: 'ui/marquee-ellipse' },
    ],
    onChange: (v) => {
      b.shape = v;
      b.roundness = shapeRoundness(v);
      radius.value = String(Math.round(b.roundness * 100));
      saveSettings();
      showBubble(bubbleOpts());
    },
  });
  disposers.push(() => shape.destroy?.());
  const radius = slider({
    min: 0, max: 100, value: Math.round((b.roundness ?? 1) * 100),
    onInput: (v) => { b.roundness = v / 100; b.shape = v === 0 ? 'square' : v === 100 ? 'circle' : 'rounded'; saveSettings(); showBubble(bubbleOpts()); },
  });
  radius.setAttribute('aria-label', t('plugin.capture.camera.roundness', null, 'Corner roundness'));
  const size = slider({
    min: 120, max: 520, step: 10, value: b.size || 240,
    onInput: (v) => { b.size = v; saveSettings(); showBubble(bubbleOpts()); },
  });
  size.setAttribute('aria-label', t('plugin.capture.camera.size', null, 'Size'));
  const mirror = toggleRow({
    label: t('plugin.capture.camera.mirror', null, 'Mirror'),
    checked: b.mirror !== false,
    onChange: (v) => { b.mirror = v; saveSettings(); showBubble(bubbleOpts()); },
  });
  const border = toggleRow({
    label: t('plugin.capture.camera.border', null, 'Accent border'),
    checked: (b.border || 0) > 0,
    onChange: (v) => { b.border = v ? 4 : 0; saveSettings(); showBubble(bubbleOpts()); },
  });
  body.append(
    field({ label: t('plugin.capture.camera.device', null, 'Camera'), control: devSel }),
    shape,
    field({ label: t('plugin.capture.camera.roundness', null, 'Corner roundness'), control: radius }),
    field({ label: t('plugin.capture.camera.size', null, 'Size'), control: size }),
    mirror,
    border,
  );
  return group('camera', t('plugin.capture.camera.label', null, 'Webcam'), [enable, body]);
}

function bubbleOpts() {
  const b = settings.bubble;
  return {
    size: b.size || 240,
    roundness: b.roundness ?? 1,
    mirror: b.mirror !== false,
    border: b.border || 0,
    x: b.x, y: b.y,
    device: settings.cameraDevice || '',
  };
}

function openBubble() {
  showBubble(bubbleOpts());
}

function audioPanel() {
  const mics = (devices?.audio || []).filter((a) => !a.monitor);
  const mons = (devices?.audio || []).filter((a) => a.monitor);
  const opts = (list) => [{ value: 'default', label: t('plugin.capture.audio.default', null, 'Default') }, ...list.map((a) => ({ value: a.name, label: a.description }))];
  meters.mic = levelMeter({ vertical: false, className: 'capture-meter' });
  meters.desktop = levelMeter({ vertical: false, className: 'capture-meter' });
  const row = (key, labelKey, label, devKey, list, ico) => {
    const sel = select({ options: opts(list), value: settings[devKey] || 'default', onChange: (e) => set(devKey, e.target ? e.target.value : e) });
    const tg = toggle({ checked: settings[key], onChange: (v) => { set(key, v); wrap.classList.toggle('is-off', !v); } });
    tg.setAttribute('aria-label', label);
    const wrap = h('div', `capture-audio-row${settings[key] ? '' : ' is-off'}`);
    const top = h('div', 'capture-audio-top');
    top.append(icon(ico, { size: 16 }), h('span', 'capture-audio-name', t(labelKey, null, label)), tg);
    wrap.append(top, sel, meters[key]);
    return wrap;
  };
  return group('audio', t('plugin.capture.audio.label', null, 'Sound'), [
    row('mic', 'plugin.capture.audio.mic', 'Microphone', 'micDevice', mics, 'ui/mic'),
    row('desktop', 'plugin.capture.audio.desktop', 'Desktop sound', 'desktopDevice', mons, 'ui/volume'),
  ]);
}

function startMeters() {
  stopMeters();
  const tick = async () => {
    if (!tileEl?.isConnected || currentView !== 'record' || document.hidden) {
      meterTimer = setTimeout(tick, 600);
      return;
    }
    try {
      const q = new URLSearchParams();
      if (settings.mic) q.set('mic', settings.micDevice || 'default');
      if (settings.desktop) q.set('desktop', settings.desktopDevice || 'default');
      const r = await apiFetch(`/api/capture/meter?${q}`);
      const d = r?.data || {};
      meters.mic?.updateDb(settings.mic && d.mic ? d.mic.peak_db : -90);
      meters.desktop?.updateDb(settings.desktop && d.desktop ? d.desktop.peak_db : -90);
    } catch (_) { /* noop */ }
    meterTimer = setTimeout(tick, 120);
  };
  tick();
}

function stopMeters() {
  clearTimeout(meterTimer);
  meterTimer = 0;
}

/* ── live destinations ──────────────────────────────────────── */

let liveListEl = null;

function livePanel() {
  liveListEl = h('div', 'capture-dest-list');
  const add = button({ icon: 'ui/plus', label: t('plugin.capture.live.add', null, 'Add destination'), variant: 'ghost', size: 'sm', onClick: () => editDestination(null) });
  const rec = toggleRow({
    label: t('plugin.capture.live.alsoRecord', null, 'Also save a recording'),
    checked: settings.record,
    onChange: (v) => set('record', v),
  });
  renderDestinations();
  return group('live', t('plugin.capture.live.label', null, 'Live'), [liveListEl, add, rec]);
}

function renderDestinations() {
  if (!liveListEl) return;
  liveListEl.textContent = '';
  if (!destinations.length) {
    liveListEl.appendChild(h('p', 'capture-hint', t('plugin.capture.live.empty', null, 'Add YouTube, Twitch, Kick, Facebook or any RTMP server with its stream key.')));
    return;
  }
  for (const d of destinations) {
    const row = h('div', 'capture-dest');
    const on = toggle({
      checked: settings.live.includes(d.id),
      onChange: (v) => {
        settings.live = v ? [...new Set([...settings.live, d.id])] : settings.live.filter((x) => x !== d.id);
        saveSettings();
      },
    });
    on.setAttribute('aria-label', d.name);
    const name = h('div', 'capture-dest-name');
    name.append(h('strong', null, d.name), h('span', 'capture-dest-key', d.has_key ? d.key_hint : t('plugin.capture.live.noKey', null, 'no key')));
    row.append(on, name,
      toolButton('ui/pencil', t('plugin.capture.live.edit', null, 'Edit'), () => editDestination(d)),
      toolButton('ui/trash', t('plugin.capture.live.remove', null, 'Remove'), async () => {
        if (!await confirmDialog(t('plugin.capture.live.removeAsk', { name: d.name }, `Remove ${d.name}?`))) return;
        await apiFetch(`/api/capture/destinations/${encodeURIComponent(d.id)}`, { method: 'DELETE' }).catch(() => {});
        await loadDestinations();
      }, { danger: true }));
    liveListEl.appendChild(row);
  }
}

async function loadDestinations() {
  try {
    const [d, p] = await Promise.all([apiFetch('/api/capture/destinations'), platforms.length ? null : apiFetch('/api/capture/platforms')]);
    destinations = d?.data || [];
    if (p) platforms = p.data || [];
  } catch (_) { /* noop */ }
  renderDestinations();
}

function editDestination(d) {
  const form = h('form', 'capture-dest-form');
  const platform = select({
    options: platforms.map((p) => ({ value: p.id, label: p.name })),
    value: d?.platform || 'youtube',
    onChange: () => sync(),
  });
  const name = input({ value: d?.name || '', placeholder: t('plugin.capture.live.namePh', null, 'My channel') });
  const server = input({ value: d?.server || '', placeholder: 'rtmps://…' });
  const key = input({ type: 'password', value: '', placeholder: d?.has_key ? d.key_hint : t('plugin.capture.live.keyPh', null, 'Stream key') });
  key.autocomplete = 'off';
  const help = h('p', 'capture-hint');
  const sync = () => {
    const p = platforms.find((x) => x.id === (platform.select?.value || platform.value));
    server.placeholder = p?.server || 'rtmps://…';
    help.textContent = p?.server
      ? t('plugin.capture.live.helpServer', null, 'The server is filled in for you: paste only the stream key from the platform\'s dashboard.')
      : t('plugin.capture.live.helpBoth', null, 'Copy the server address and the stream key from the platform\'s dashboard.');
  };
  sync();
  form.append(
    field({ label: t('plugin.capture.live.platform', null, 'Platform'), control: platform }),
    field({ label: t('plugin.capture.live.name', null, 'Name'), control: name }),
    field({ label: t('plugin.capture.live.server', null, 'Server'), control: server }),
    field({ label: t('plugin.capture.live.key', null, 'Stream key'), control: key }),
    help,
  );
  const dlg = dialog(
    d ? t('plugin.capture.live.editTitle', null, 'Edit destination') : t('plugin.capture.live.addTitle', null, 'New live destination'),
    form,
    [
      button({ label: t('plugin.capture.cancel', null, 'Cancel'), icon: 'ui/close', variant: 'ghost', onClick: () => dlg.close() }),
      button({
        label: t('plugin.capture.save', null, 'Save'),
        icon: 'ui/check',
        onClick: async () => {
          const body = {
            platform: platform.select?.value || platform.value,
            name: name.value.trim() || undefined,
            server: server.value.trim() || undefined,
          };
          if (key.value.trim()) body.key = key.value.trim();
          try {
            const url = d ? `/api/capture/destinations/${encodeURIComponent(d.id)}` : '/api/capture/destinations';
            const r = await apiFetch(url, { method: d ? 'PUT' : 'POST', body: JSON.stringify(body) });
            if (!d && r?.data?.id) { settings.live = [...new Set([...settings.live, r.data.id])]; saveSettings(); }
            dlg.close();
            await loadDestinations();
          } catch (e) {
            toast(e.message || String(e), { type: 'error' });
          }
        },
      }),
    ],
  );
  dlg.open();
  setTimeout(() => (d ? key : name).focus?.(), 50);
}

function qualityPanel() {
  const fps = segmented({
    label: t('plugin.capture.quality.fps', null, 'Frame rate'),
    value: String(settings.fps),
    options: [{ value: '30', label: '30 fps' }, { value: '60', label: '60 fps' }],
    onChange: (v) => set('fps', Number(v)),
  });
  const q = segmented({
    label: t('plugin.capture.quality.label', null, 'Quality'),
    value: settings.quality,
    options: [
      { value: 'high', label: t('plugin.capture.quality.high', null, 'High') },
      { value: 'balanced', label: t('plugin.capture.quality.balanced', null, 'Balanced') },
      { value: 'small', label: t('plugin.capture.quality.small', null, 'Small files') },
    ],
    onChange: (v) => set('quality', v),
  });
  disposers.push(() => { fps.destroy?.(); q.destroy?.(); });
  const enc = h('p', 'capture-hint');
  enc.textContent = devices
    ? t('plugin.capture.quality.encoders', { rec: devices.record_encoder, live: devices.live_encoder }, `Recording: ${devices.record_encoder} · Live: ${devices.live_encoder}`)
    : '';
  const folder = h('p', 'capture-hint capture-folder');
  folder.textContent = devices?.folder || '';
  return group('quality', t('plugin.capture.quality.label', null, 'Quality'), [fps, q, enc, folder]);
}

/* ── library view ───────────────────────────────────────────── */

function buildLibraryView() {
  const root = h('div', 'capture-library');
  return root;
}

async function renderLibrary() {
  const root = views.library;
  if (!root) return;
  root.textContent = '';
  root.appendChild(spinner());
  let recs = [];
  let projects = [];
  try {
    const [r, p] = await Promise.all([apiFetch('/api/capture/recordings'), apiFetch('/api/capture/projects')]);
    recs = r?.data || [];
    projects = p?.data || [];
  } catch (_) { /* noop */ }
  root.textContent = '';
  const projHead = h('div', 'capture-lib-head');
  projHead.append(h('h3', null, t('plugin.capture.library.projects', null, 'Projects')),
    button({ icon: 'ui/plus', label: t('plugin.capture.library.newProject', null, 'New project'), variant: 'ghost', size: 'sm', onClick: () => newProject([]) }));
  root.appendChild(projHead);
  const plist = h('div', 'capture-lib-grid');
  for (const p of projects) {
    const card = h('button', 'capture-lib-card capture-lib-card--project');
    card.type = 'button';
    card.append(icon('ui/layers', { size: 22 }), h('strong', null, p.title), h('span', 'capture-lib-meta', `${formatTimecode(p.duration || 0)} · ${p.clips} ${t('plugin.capture.library.clips', null, 'clips')}`));
    card.addEventListener('click', () => openProject(p.id));
    plist.appendChild(card);
  }
  if (!projects.length) plist.appendChild(h('p', 'capture-hint', t('plugin.capture.library.noProjects', null, 'No projects yet. Open a recording to start one.')));
  root.appendChild(plist);

  root.appendChild(h('h3', 'capture-lib-title', t('plugin.capture.library.recordings', null, 'Recordings')));
  if (!recs.length) {
    root.appendChild(emptyState({ icon: 'ui/video', title: t('plugin.capture.library.empty', null, 'No recordings yet'), body: t('plugin.capture.library.emptyHint', null, 'Press Record to make the first one.') }));
    return;
  }
  const grid = h('div', 'capture-lib-grid');
  for (const r of recs) {
    const card = h('div', 'capture-lib-card');
    const thumb = h('div', 'capture-lib-thumb');
    const img = document.createElement('img');
    img.loading = 'lazy';
    img.alt = '';
    img.src = `/api/capture/media/thumbs?id=${encodeURIComponent(r.id)}&n=1`;
    thumb.appendChild(img);
    const dur = r.meta?.duration_us ? formatTimecode(r.meta.duration_us / 1e6) : '';
    const meta = h('span', 'capture-lib-meta', [dur, `${(r.bytes / 1048576).toFixed(1)} MB`].filter(Boolean).join(' · '));
    const actions = h('div', 'capture-lib-actions');
    actions.append(
      toolButton('ui/scissors', t('plugin.capture.library.edit', null, 'Edit'), () => newProject([r.id], r.id.replace(/\.[^.]+$/, ''))),
      toolButton('ui/play', t('plugin.capture.library.play', null, 'Play'), () => playRecording(r)),
      toolButton('ui/trash', t('plugin.capture.library.delete', null, 'Delete'), async () => {
        if (!await confirmDialog(t('plugin.capture.library.deleteAsk', { name: r.id }, `Delete ${r.id}?`))) return;
        await apiFetch(`/api/capture/recordings/${encodeURIComponent(r.id)}`, { method: 'DELETE' }).catch(() => {});
        renderLibrary();
      }, { danger: true }),
    );
    card.append(thumb, h('strong', 'capture-lib-name', r.id), meta, actions);
    grid.appendChild(card);
  }
  root.appendChild(grid);
}

function playRecording(r) {
  const v = document.createElement('video');
  v.className = 'capture-player';
  v.controls = true;
  v.autoplay = true;
  v.src = `/api/capture/media?id=${encodeURIComponent(r.id)}`;
  const dlg = dialog(r.id, v, [], { wide: true, onClose: () => { v.pause(); v.removeAttribute('src'); v.load(); } });
  dlg.open();
}

async function newProject(media, title) {
  try {
    const r = await apiFetch('/api/capture/projects', { method: 'POST', body: JSON.stringify({ media, title: title || t('plugin.capture.library.untitled', null, 'New video') }) });
    if (r?.data?.id) openProject(r.data.id);
  } catch (e) {
    toast(e.message || String(e), { type: 'error' });
  }
}

async function openProject(id) {
  await showView('edit');
  editorMod?.open(id);
}

/* ── views ──────────────────────────────────────────────────── */

async function showView(v) {
  currentView = v;
  tileEl?.querySelector('.capture-views')?.setValue?.(v, { silent: true });
  for (const [k, el] of Object.entries(views)) el.hidden = k !== v;
  if (v === 'record') startMeters(); else stopMeters();
  if (v === 'library') renderLibrary();
  if (v === 'edit') {
    if (!editorMod) {
      const mod = await import('./editor.js');
      editorMod = mod.default;
      views.edit.appendChild(editorMod.mount());
    }
    editorMod.show?.();
  }
}

/* ── tile lifecycle ─────────────────────────────────────────── */

export function mountCaptureTile() {
  if (tileEl) return tileEl;
  tileEl = h('section', 'tile capture-tile');
  tileEl.dataset.plugin = CAPTURE_PLUGIN;
  tileEl.appendChild(buildBar());
  viewEl = h('div', 'capture-body');
  tileEl.appendChild(viewEl);
  const loading = spinner();
  viewEl.appendChild(loading);
  (async () => {
    await loadSettings();
    try { devices = (await apiFetch('/api/capture/devices'))?.data || null; } catch (_) { devices = null; }
    await loadDestinations();
    if (!tileEl) return;
    loading.remove();
    views = { record: buildRecordView(), library: buildLibraryView(), edit: h('div', 'capture-edit') };
    viewEl.append(views.record, views.library, views.edit);
    await showView(currentView);
    if (settings.camera) openBubble();
    disposers.push(onBubbleChange((st) => { settings.bubble.x = st.x; settings.bubble.y = st.y; saveSettings(); }));
    renderStatus();
  })();
  poll();
  return tileEl;
}

export function unmountCaptureTile() {
  clearTimeout(pollTimer);
  stopMeters();
  stopTracking();
  for (const d of disposers.splice(0)) { try { d(); } catch (_) { /* noop */ } }
  editorMod?.unmount?.();
  editorMod = null;
  // The bubble stays on screen while recording; otherwise it closes with the window.
  if (!isActive()) hideBubble();
  tileEl?.remove();
  tileEl = null;
  views = {};
  panelEls = {};
  previewImg = null;
  statsEl = null;
  liveListEl = null;
}

export function getCaptureTileElement() {
  return tileEl;
}

let wired = false;
export function wireCaptureEvents() {
  if (wired) return;
  wired = true;
  // The agent asks to record or go live: the window confirms (hud.js handles
  // it when the window is closed).
  window.addEventListener('capture:start-request', (e) => {
    const req = e.detail || {};
    if (req.source) set('source', req.source);
    if (req.webcam != null) set('camera', !!req.webcam);
    if (req.mic != null) set('mic', !!req.mic);
    if (req.desktop != null) set('desktop', !!req.desktop);
    if (req.live?.length) settings.live = req.live.map((d) => d.id);
    if (settings.camera) openBubble();
    start({ live: !!req.live?.length, record: req.record !== false });
  });
  window.addEventListener('capture:open-project', (e) => {
    if (e.detail?.id) openProject(e.detail.id);
  });
}

export function captureContextMenu() {
  const active = isActive();
  return [
    {
      type: 'item',
      label: active ? t('plugin.capture.action.stop', null, 'Stop') : t('plugin.capture.action.record', null, 'Record'),
      icon: active ? 'ui/stop' : 'ui/record',
      onClick: () => (active ? stop() : start({ record: true })),
    },
    { type: 'item', label: t('plugin.capture.action.marker', null, 'Add marker'), icon: 'ui/pin', disabled: !active, onClick: () => addMarker() },
  ];
}

export default {
  name: CAPTURE_PLUGIN,
  icon: 'apps/capture',
  mount: mountCaptureTile,
  unmount: unmountCaptureTile,
  getElement: getCaptureTileElement,
  wireEvents: wireCaptureEvents,
  contextMenu: captureContextMenu,
};
