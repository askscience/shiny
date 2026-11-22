/**
 * touchbarHost.js — the desktop's side of the Touch Bar surface.
 *
 * On a T2 MacBook the kiosk shell draws a page of the app on the Touch Bar
 * (`web/touchbar.html`, `js/touchbarSurface/bar.js`). That page shows what
 * this module publishes and reports what was touched; everything it does
 * happens here, in the desktop page, with the modules that already do it.
 *
 * What goes on the bar:
 *
 *  - the assistant button, with the voice state and level;
 *  - the focused window's own controls: a window's surface module may export
 *    `touchBar()` returning `[{ id, label, icon, active, disabled, onClick }]`
 *    (the same shape as its `contextMenu()` entries, which are used when it
 *    has no `touchBar()`), followed by full screen and close;
 *  - with **Settings → Touch Bar → Dock on the Touch Bar**, the whole dock:
 *    the bar is mirrored from the real one in `#hud-header`, so every app,
 *    chip and plugin widget is there without a second list to maintain, and
 *    a tap is a click on the real control (its menu opens on the screen);
 *  - sliders for screen brightness and volume, the battery and the clock.
 *
 * Nothing here runs until a bar says hello, so a machine without one pays
 * for a BroadcastChannel and nothing else.
 */

import { apiFetch } from './api.js';
import { currentSink, nodeIcon } from './audioShared.js';
import { coreWindowTitle, isCoreWindow } from './coreWindows.js';
import {
  activeWorkspaceIndex, closeWindow, createWorkspace, getFocus, getFullscreen,
  switchWorkspace, toggleFullscreen, workspaceCount,
} from './desktop.js';
import { sessionEntries } from './powerMenu.js';
import { closeChat, isTextInputOpen } from './textInput.js';
import { t } from './i18n.js';
import { getImmersive, getTouchBarDock, getTouchBarEnabled, isAiEnabled } from './preferences.js';
import { getPluginSurface } from './tiles.js';
import { loadIconSvg } from '../ui/icon.js';
import { isTouchBarActive } from './touchbarShared.js';
import {
  CHANNEL, backlightFor, clamp01, normalizeContextItems,
} from './touchbarSurface/barShared.js';

let channel = null;
let talk = null;
let linked = false;
let publishQueued = false;
/** id → what a tap on that bar item does. Rebuilt with every snapshot. */
let handlers = new Map();
const iconCache = new Map();
const sliders = { volume: null, brightness: null };
let lastActivity = Date.now();
let backlight = 2;

const surface = () => window.__shinyTouchBarSurface === true;

function post(message) {
  try { channel?.postMessage(message); } catch (_) { /* closed */ }
}

function shell(message) {
  try { window.ipc?.postMessage(`liteaura:touchbar:${JSON.stringify(message)}`); } catch (_) { /* no shell */ }
}

/* ── Icons ────────────────────────────────────────────────────── */

/** An icon by name, as markup. Resolved once; a snapshot is sent again when
 *  one that was still loading arrives. */
function namedIcon(name) {
  if (!name) return null;
  if (iconCache.has(name)) return iconCache.get(name);
  iconCache.set(name, null);
  Promise.resolve(loadIconSvg(name)).then((svg) => {
    if (!svg) return;
    iconCache.set(name, { svg });
    schedule();
  }).catch(() => {});
  return null;
}

/** The icon a HUD control is showing right now. */
function shownIcon(el) {
  const svg = el.querySelector('svg');
  if (svg) return { svg: svg.outerHTML };
  const img = el.querySelector('img');
  if (img?.src) return { img: img.src };
  return null;
}

const shown = (el) => !!el && !el.classList.contains('hidden') && !el.hidden;
const labelOf = (el) => el.getAttribute('aria-label') || el.title || '';
/** The text of a part of a HUD control, if the dock shows it. Names,
 *  percentages, the date and the weather text each have a setting
 *  (Settings → Appearance), applied by the stylesheet: a part it hides is
 *  not on the Touch Bar either. */
function textOf(el, selector) {
  const part = el.querySelector(selector);
  if (!part || getComputedStyle(part).display === 'none') return '';
  return (part.textContent || '').trim();
}

/* ── What the bar shows ───────────────────────────────────────── */

function windowTitle(name) {
  if (isCoreWindow(name)) return coreWindowTitle(name);
  return name.charAt(0).toUpperCase() + name.slice(1);
}

/** The focused window's controls. */
function contextFor(name) {
  if (!name) return null;
  const module = getPluginSurface(name);
  let entries = [];
  try {
    if (typeof module?.touchBar === 'function') entries = module.touchBar({ name });
    else if (typeof module?.contextMenu === 'function') entries = module.contextMenu({ name, touchBar: true });
  } catch (err) {
    console.warn(`touchBar() for '${name}' failed`, err);
  }
  const items = normalizeContextItems(entries).map((entry) => {
    handlers.set(`ctx:${name}:${entry.id}`, entry.onClick);
    return {
      id: entry.id,
      label: entry.label,
      icon: namedIcon(entry.icon),
      active: entry.active,
      disabled: entry.disabled,
      showLabel: entry.showLabel || !entry.icon,
    };
  });
  const full = getFullscreen() === name;
  const windowItems = [
    {
      id: 'window:fullscreen',
      label: full
        ? t('touchbar.exitFullscreen', null, 'Exit full screen')
        : t('touchbar.fullscreen', null, 'Full screen'),
      icon: namedIcon('ui/expand'),
      active: full,
      run: () => toggleFullscreen(name),
    },
    {
      id: 'window:close',
      label: t('touchbar.closeWindow', null, 'Close window'),
      icon: namedIcon('ui/close'),
      run: () => closeWindow(name),
    },
  ];
  for (const entry of windowItems) {
    handlers.set(`ctx:${name}:${entry.id}`, entry.run);
    items.push({ id: entry.id, label: entry.label, icon: entry.icon, active: entry.active, showLabel: !entry.icon });
  }
  return { name, title: windowTitle(name), items };
}

/** The app tray, mirrored from the real one. */
function dockApps(focused) {
  const apps = [];
  for (const button of document.querySelectorAll('#hud-plugins .hud-plugin-btn')) {
    if (!shown(button)) continue;
    const name = button.dataset.plugin;
    if (!name) continue;
    const id = `app:${name}`;
    handlers.set(id, () => button.click());
    apps.push({
      id,
      label: labelOf(button) || windowTitle(name),
      icon: shownIcon(button),
      active: button.classList.contains('is-active'),
      focused: focused === name,
    });
  }
  return apps;
}

function clockItem() {
  const clock = document.querySelector('.hud-clock');
  if (!clock) return null;
  handlers.set('clock', () => clock.click());
  return {
    id: 'clock',
    kind: 'clock',
    label: labelOf(clock),
    text: textOf(clock, '#hud-clock-time'),
    sub: textOf(clock, '#hud-clock-date'),
  };
}

function chipItem(el) {
  if (!shown(el)) return null;
  const id = `chip:${el.id}`;
  handlers.set(id, () => el.click());
  const text = [textOf(el, '[id$="-label"]'), textOf(el, '[id$="-pct"], #hud-meteo-temp')]
    .filter(Boolean)
    .join(' ');
  return {
    id,
    kind: 'chip',
    label: labelOf(el),
    icon: shownIcon(el),
    text,
    on: el.getAttribute('aria-expanded') === 'true',
  };
}

/** The dock's actions that are not worth a place of their own on the bar:
 *  they sit behind one button, in a menu on the bar itself. */
const MENU_BUTTONS = ['hud-updates', 'hud-clipboard', 'hud-theme', 'settings-btn', 'plugins-btn'];

function menuFor() {
  const items = [];
  for (const id of MENU_BUTTONS) {
    const el = document.getElementById(id);
    if (!shown(el)) continue;
    handlers.set(`menu:${id}`, () => el.click());
    items.push({
      id: `menu:${id}`,
      label: el.title || labelOf(el),
      icon: shownIcon(el),
      // The updater lights up when there is something to install.
      on: el.classList.contains('is-active'),
    });
  }
  for (const entry of sessionEntries()) {
    handlers.set(`menu:session:${entry.id}`, entry.onClick);
    items.push({
      id: `menu:session:${entry.id}`,
      label: entry.label,
      icon: namedIcon(entry.icon),
      danger: entry.danger,
    });
  }
  return {
    label: t('common.more', null, 'More'),
    icon: namedIcon('ui/more'),
    items,
    // The button itself says when something behind it wants attention.
    on: items.some((item) => item.on),
  };
}

/** The status end of the bar: the battery and the clock, or the dock's whole
 *  right side when the dock lives on the bar. */
function statusItems(dock) {
  const items = [];
  if (dock) {
    for (const el of document.querySelectorAll('#hud-top > button')) {
      // Volume has its slider on the bar already; the actions and the
      // session are in the bar's own menu.
      if (el.id === 'hud-audio' || el.id === 'hud-power' || MENU_BUTTONS.includes(el.id)) continue;
      const item = chipItem(el);
      if (item) items.push(item);
    }
    // The weather sits by the clock, as it does in the dock.
    const meteo = chipItem(document.getElementById('hud-meteo') || document.createElement('i'));
    if (meteo) items.push(meteo);
  } else {
    const battery = chipItem(document.getElementById('hud-battery') || document.createElement('i'));
    if (battery) items.push(battery);
  }
  const clock = clockItem();
  if (clock) items.push(clock);
  return items;
}

/** The dock's own volume icons: muted, silent, or one per loudness step.
 *  While a finger is on the slider the step follows the finger, not the
 *  last status the server sent. */
function volumeIcon(volume) {
  if (volume.muted) return 'hud/volume-muted';
  if (volume.value <= 0) return 'hud/volume-0';
  if (volume.headphones) return 'hud/headphones';
  return `hud/volume-${Math.max(1, Math.min(3, Math.ceil(volume.value * 3)))}`;
}

function sliderItems() {
  const items = [];
  if (sliders.brightness != null) {
    items.push({
      id: 'brightness',
      label: t('touchbar.brightness', null, 'Screen brightness'),
      icon: namedIcon('ui/brightness'),
      value: sliders.brightness,
    });
  }
  if (sliders.volume) {
    items.push({
      id: 'volume',
      label: t('touchbar.volume', null, 'Volume'),
      icon: namedIcon(volumeIcon(sliders.volume)),
      value: sliders.volume.value,
      muted: sliders.volume.muted,
    });
  }
  return items;
}

function snapshot() {
  handlers = new Map();
  const dock = getTouchBarDock();
  const focused = getFocus();
  const count = workspaceCount();
  const immersive = getImmersive();
  return {
    v: 1,
    ai: isAiEnabled(),
    // The dots on the bar are the dock's: same effect, same reactions.
    orb: { effect: immersive.orb_effect, react: immersive.orb_react },
    voice: document.documentElement.dataset.voice || 'idle',
    accent: getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || null,
    context: contextFor(focused),
    apps: dock ? dockApps(focused) : [],
    spaces: dock ? { count, active: activeWorkspaceIndex() } : null,
    menu: dock ? menuFor() : null,
    sliders: sliderItems(),
    status: statusItems(dock),
  };
}

function publish() {
  publishQueued = false;
  if (!linked) return;
  if (!isTouchBarActive(getTouchBarEnabled(), true)) {
    // Turned off in Settings: the bar goes back to an empty plate.
    post({ t: 'bye' });
    return;
  }
  post({ t: 'state', state: snapshot() });
}

/** Coalesce a burst of changes (a window opening touches a dozen things). */
function schedule() {
  if (!linked || publishQueued) return;
  publishQueued = true;
  setTimeout(publish, 40);
}

/** Ask for a fresh snapshot: a window whose `touchBar()` items changed calls
 *  this (or dispatches `touchbar:refresh`). */
export function refreshTouchBarSurface() {
  schedule();
}

/* ── Sliders ──────────────────────────────────────────────────── */

async function readSliders() {
  try {
    const audio = await apiFetch('/api/audio/status', { authRedirect: false });
    const sink = currentSink(audio?.data);
    sliders.volume = sink
      ? {
        id: sink.id,
        max: sink.max_volume || 100,
        value: clamp01((sink.volume_percent || 0) / (sink.max_volume || 100)),
        muted: !!sink.muted,
        headphones: nodeIcon(sink, 'sink') === 'hud/headphones',
      }
      : null;
  } catch (_) { /* no audio server */ }
  try {
    const screen = await apiFetch('/api/screen/brightness', { authRedirect: false });
    const data = screen?.data;
    sliders.brightness = data && data.available !== false && Number.isFinite(data.percent)
      ? clamp01(data.percent / 100)
      : null;
  } catch (_) { /* no backlight */ }
  schedule();
}

let slideBusy = false;
let slideNext = null;

/** Apply a slider move. Moves arrive faster than the server answers, so only
 *  the latest one waiting is kept. */
async function slide(id, value) {
  if (slideBusy) {
    slideNext = { id, value };
    return;
  }
  slideBusy = true;
  try {
    if (id === 'volume' && sliders.volume) {
      const percent = Math.round(clamp01(value) * sliders.volume.max);
      sliders.volume.value = clamp01(value);
      // The icon changes with the level (and at zero): show it right away.
      schedule();
      await apiFetch('/api/audio/volume', {
        method: 'POST',
        body: JSON.stringify({ target: 'sink', id: sliders.volume.id, percent }),
      });
    } else if (id === 'brightness') {
      sliders.brightness = clamp01(value);
      await apiFetch('/api/screen/brightness', {
        method: 'POST',
        // All the way down is the backlight off; the slider (and the
        // brightness keys) bring it back.
        body: JSON.stringify({ percent: Math.round(clamp01(value) * 100) }),
      });
    }
  } catch (_) { /* the next move, or the next status read, puts it right */ }
  slideBusy = false;
  if (slideNext) {
    const next = slideNext;
    slideNext = null;
    void slide(next.id, next.value);
  }
}

async function toggleMute() {
  if (!sliders.volume) return;
  const muted = !sliders.volume.muted;
  sliders.volume.muted = muted;
  schedule();
  try {
    await apiFetch('/api/audio/mute', {
      method: 'POST',
      body: JSON.stringify({ target: 'sink', id: sliders.volume.id, muted }),
    });
  } catch (_) { /* read back below */ }
  void readSliders();
}

/* ── The bar's own light ──────────────────────────────────────── */

function noteActivity() {
  lastActivity = Date.now();
  if (backlight !== 2) applyBacklight();
}

function applyBacklight() {
  const level = backlightFor(Date.now() - lastActivity);
  if (level === backlight) return;
  backlight = level;
  shell({ backlight: level });
  post({ t: 'sleep', on: level === 0 });
}

/**
 * The assistant's button on the bar is the dock's button: a press and a
 * release there are replayed on the real one, so its own gesture rules apply
 * (`sphere.js`: one tap opens the chat, two listen, a hold is the wake word)
 * and stay the only copy of them. Without that button (it has not booted
 * yet) a release is the plain talk action.
 */
let orbSwallowed = false;

function pressOrb(down) {
  // A tap while the chat is open closes it. The press is not replayed: the
  // chat treats any press outside itself as "close", and the release would
  // then count as a fresh tap and open it again.
  if (down && isTextInputOpen()) {
    orbSwallowed = true;
    closeChat();
    return;
  }
  if (orbSwallowed) {
    if (!down) orbSwallowed = false;
    return;
  }
  const orb = document.getElementById('sphere-container');
  if (!orb || typeof PointerEvent !== 'function') {
    if (!down) void talk?.();
    return;
  }
  orb.dispatchEvent(new PointerEvent(down ? 'pointerdown' : 'pointerup', {
    button: 0, pointerId: 1, pointerType: 'touch', isPrimary: true, bubbles: true, cancelable: true,
  }));
}

/* ── Messages from the bar ────────────────────────────────────── */

function onMessage({ data }) {
  if (!data || typeof data !== 'object') return;
  switch (data.t) {
    case 'hello':
      if (!linked) link();
      void readSliders();
      publish();
      break;
    case 'activity':
      noteActivity();
      break;
    case 'orb':
      pressOrb(data.phase === 'down');
      break;
    case 'tap': {
      if (data.id === 'volume') { void toggleMute(); break; }
      const run = handlers.get(data.id);
      if (!run) break;
      try { run(); } catch (err) { console.warn('touch bar action failed', err); }
      schedule();
      break;
    }
    case 'slide':
      void slide(data.id, data.value);
      if (data.final) setTimeout(readSliders, 400);
      break;
    case 'space':
      if (data.add) createWorkspace();
      else switchWorkspace(Number.isInteger(data.index) ? data.index : data.dir === 'prev' ? 'prev' : 'next');
      break;
    case 'fkey':
      if (Number.isInteger(data.n) && data.n >= 1 && data.n <= 12) shell({ fkey: data.n });
      break;
    default:
      break;
  }
}

/** The first hello: start watching everything the bar mirrors. */
function link() {
  linked = true;
  const header = document.getElementById('hud-header');
  if (header && typeof MutationObserver === 'function') {
    new MutationObserver(schedule).observe(header, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
      attributeFilter: ['class', 'hidden', 'title', 'aria-label', 'aria-expanded', 'src'],
    });
  }
  for (const name of [
    'desktop:changed', 'windows:changed', 'plugins:changed', 'touchbar:settings',
    'touchbar:refresh', 'battery:changed', 'menu:change', 'desktop:immersive',
  ]) {
    window.addEventListener(name, schedule);
  }
  window.addEventListener('voice:state', (event) => {
    post({ t: 'voice', state: event.detail?.state || 'idle' });
  });
  window.addEventListener('agent:step', (event) => {
    post({ t: 'step', message: event.detail?.message || null, action: event.detail?.action || null });
  });
  let levelAt = 0;
  window.addEventListener('voice:level', (event) => {
    const now = performance.now();
    if (now - levelAt < 33) return;
    levelAt = now;
    const detail = event.detail;
    post({ t: 'level', level: clamp01(typeof detail === 'number' ? detail : detail?.level), pan: Number(detail?.pan) || 0 });
  });
  // The volume chip's text changes with the volume: read the sliders again.
  const audio = document.getElementById('hud-audio');
  if (audio && typeof MutationObserver === 'function') {
    let timer = 0;
    new MutationObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(readSliders, 300);
    }).observe(audio, { subtree: true, childList: true, characterData: true, attributes: true });
  }
  if (surface()) {
    for (const name of ['pointerdown', 'pointermove', 'keydown', 'wheel']) {
      window.addEventListener(name, () => { lastActivity = Date.now(); if (backlight !== 2) applyBacklight(); },
        { passive: true, capture: true });
    }
    setInterval(applyBacklight, 5000);
  }
  window.addEventListener('pagehide', () => post({ t: 'bye' }));
}

/** Show or hide the on-screen dock according to the setting. The dock only
 *  leaves the screen when a bar is really there to hold it. */
export function applyTouchBarDock() {
  const on = surface() && getTouchBarDock() && isTouchBarActive(getTouchBarEnabled(), true);
  document.documentElement.dataset.touchbarDock = on ? '1' : '0';
  document.documentElement.dataset.touchbarSurface = surface() ? '1' : '0';
}

export function initTouchBarHost(next = {}) {
  talk = next.talk || null;
  applyTouchBarDock();
  window.addEventListener('touchbar:settings', applyTouchBarDock);
  if (channel || typeof BroadcastChannel !== 'function') return;
  channel = new BroadcastChannel(CHANNEL);
  channel.onmessage = onMessage;
}
