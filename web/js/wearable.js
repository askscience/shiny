/**
 * wearable.js — the wearable face: an interface for a watch-sized screen that
 * also runs on a desktop or a phone (Settings → Desktop → Wearable).
 *
 * While it is on:
 *
 *  - the screen is the user's wallpaper (#background) under the voice bar's dot
 *    grid, stretched over the whole screen (voiceDots.js `face`: larger dots,
 *    semi-transparent, a circular vignette), moving with the same effect and
 *    the same signal the voice bar uses;
 *  - on top: a large clock, the date small under it, the weather (hudLeft.js
 *    keeps both up to date), and one icon button for a few settings —
 *    wallpaper, AI model, theme, animation — and the switch back;
 *  - the desktop is not shown at all (`html[data-wearable]`, css/wearable.css):
 *    no window opens on screen, plugins work in the background, and the server
 *    is told so with every request (desktop snapshot `wearable: true`);
 *  - the assistant always waits for its wake word: whenever it is idle the
 *    wake listener is armed again. A tap on the face listens once.
 */

import { getWearable, setWearable, isAiEnabled } from './preferences.js';
import { mountVoiceDots, orbEffectOptions } from './voiceDots.js';
import { getImmersive, setImmersive } from './preferences.js';
import { getWeatherState } from './hudLeft.js';
import { formatLocationTime, formatLocationDate } from './clockShared.js';
import { weatherIconStem, tempText, weatherLabel } from './weatherShared.js';
import { getLocale, t } from './i18n.js';
import { isListening, startListening } from './voice.js';
import { isTurnActive } from './agent.js';
import { isTextInputOpen } from './textInput.js';
import { getBackground, setBackground, BACKGROUND_PRESETS, activePreset, uploadBackground } from './background.js';
import { pickFiles } from './files.js';
import { loadAiModelChoices, setAiModelChoice } from './settings/assistant.js';
import {
  icon, iconButton, setIcon, listThemes, getActiveTheme, getThemeManifest, switchTheme,
} from '../ui/index.js';

/** After the assistant goes idle, wait this long before listening again. */
const REARM_MS = 900;
/** A safety net: check this often that the wake listener is up. */
const WATCH_MS = 5000;

let face = null;
let actions = { listen: null };

function root() { return document.documentElement; }

/** True while the wearable face is on screen. */
export function isWearable() {
  return root().dataset.wearable === '1';
}

/** What a tap on the face does (app.js: listen once). */
export function setWearableActions({ listen = null } = {}) {
  actions = { listen };
}

/* ── The face ──────────────────────────────────────────────────── */

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function buildFace() {
  const node = el('div', 'wearable');
  node.id = 'wearable';
  node.setAttribute('role', 'region');
  node.setAttribute('aria-label', t('wearable.label', null, 'Wearable'));

  const dots = el('div', 'wearable-dots');
  const clock = el('div', 'wearable-clock');
  const time = el('div', 'wearable-time');
  const date = el('div', 'wearable-date');
  const weather = el('div', 'wearable-weather');
  const weatherIcon = el('span', 'wearable-weather-icon');
  weatherIcon.setAttribute('aria-hidden', 'true');
  const weatherText = el('span', 'wearable-weather-text');
  weather.append(weatherIcon, weatherText);
  clock.append(time, date, weather);

  const settings = iconButton({
    icon: 'ui/settings',
    label: t('hud.settings', null, 'Settings'),
    variant: 'quiet',
    iconSize: 22,
    onClick: () => openSettings(),
  });
  settings.classList.add('wearable-settings');

  node.append(dots, clock, settings);
  // A tap anywhere on the face (not the button) listens once.
  node.addEventListener('click', (e) => {
    if (e.target.closest('button, .wearable-panel')) return;
    actions.listen?.();
  });

  const dotGrid = mountVoiceDots(dots, { face: true });

  function tick() {
    const now = new Date();
    const zone = getWeatherState()?.timezone || undefined;
    const locale = getLocale();
    // A 12-hour clock keeps its AM/PM small, so the digits fit a round face.
    const text = formatLocationTime(now, zone, locale);
    const parts = text.match(/^(.*?\d)\s*([^\d\s:.]+\.?\s?[^\d\s:]*)$/u);
    if (parts) {
      time.replaceChildren(parts[1], el('span', 'wearable-time-suffix', parts[2]));
    } else {
      time.textContent = text;
    }
    date.textContent = formatLocationDate(now, zone, locale);
  }

  function paintWeather() {
    const w = getWeatherState();
    if (!w?.current) {
      weather.classList.add('hidden');
      return;
    }
    weather.classList.remove('hidden');
    void setIcon(weatherIcon, `insights/${weatherIconStem(w.current.code)}`);
    weatherText.textContent = `${tempText(w.current.temp)} ${weatherLabel(w.current.code)}`;
  }

  tick();
  paintWeather();
  const clockTimer = setInterval(tick, 1000);
  window.addEventListener('weather:update', paintWeather);
  window.addEventListener('locale:change', tick);

  document.body.appendChild(node);
  return {
    node,
    destroy() {
      clearInterval(clockTimer);
      window.removeEventListener('weather:update', paintWeather);
      window.removeEventListener('locale:change', tick);
      dotGrid.destroy();
      panel = null;
      node.remove();
    },
  };
}

/* ── Settings: wallpaper, AI model, theme, animation ─────────── */

/*
 * A panel made for a round face, not a desktop dialog: it covers the face, its
 * content stays inside the inscribed circle, and nothing opens a menu. The
 * wallpapers are a row of round swatches; the AI model, the theme and the
 * animation are rows that step to the next choice on a tap; two icons close it
 * or leave the wearable face.
 */

let panel = null;

function cycle(options, current, step = 1) {
  if (!options.length) return null;
  const i = options.findIndex((o) => o.value === current);
  return options[(i + step + options.length) % options.length];
}

function buildPanel(host) {
  const node = el('div', 'wearable-panel hidden');
  node.setAttribute('role', 'dialog');
  node.setAttribute('aria-modal', 'true');
  node.setAttribute('aria-label', t('hud.settings', null, 'Settings'));
  const body = el('div', 'wearable-panel-body');

  /* Wallpapers: the built-in ones, the user's photo, and "+" to pick one. */
  const swatches = el('div', 'wearable-swatches');
  swatches.setAttribute('role', 'radiogroup');
  swatches.setAttribute('aria-label', t('wearable.wallpaper', null, 'Wallpaper'));
  function swatch(label, image, active, onClick) {
    const b = el('button', `wearable-swatch${active ? ' is-active' : ''}`);
    b.type = 'button';
    b.setAttribute('role', 'radio');
    b.setAttribute('aria-checked', String(active));
    b.setAttribute('aria-label', label);
    b.title = label;
    if (image) b.style.backgroundImage = `url("${image}")`;
    b.addEventListener('click', onClick);
    return b;
  }
  function paintSwatches() {
    const bg = getBackground();
    const active = bg.url ? null : activePreset(bg)?.id;
    const items = BACKGROUND_PRESETS.map((preset) => swatch(preset.label, preset.src, active === preset.id, () => {
      setBackground({ mode: 'image', preset: preset.id, url: null });
      paintSwatches();
    }));
    if (bg.url) items.push(swatch(t('background.yourImage', null, 'Your image'), bg.url, true, () => {}));
    const add = swatch(t('background.useOwnImage', null, 'Use your own image'), null, false, async () => {
      const [file] = await pickFiles({ accept: 'image/*' });
      if (file && await uploadBackground(file)) paintSwatches();
    });
    add.classList.add('wearable-swatch--add');
    add.removeAttribute('role');
    add.removeAttribute('aria-checked');
    add.append(icon('ui/plus', { size: 18 }));
    swatches.replaceChildren(...items, add);
  }

  /* A row: icon, name, current value; a tap steps to the next choice. */
  function row(iconName, label, onStep) {
    const b = el('button', 'wearable-row');
    b.type = 'button';
    const value = el('span', 'wearable-row-value');
    b.append(icon(iconName, { size: 18 }), el('span', 'wearable-row-label', label), value);
    b.addEventListener('click', () => void onStep());
    return { el: b, set: (text) => { value.textContent = text || ''; b.setAttribute('aria-label', `${label}: ${text || ''}`); } };
  }

  let models = { provider: 'ollama', options: [], value: '' };
  const model = row('ui/brain', t('settings.assistant.aiModel', null, 'AI model'), async () => {
    const next = cycle(models.options, models.value);
    if (!next) return;
    models.value = next.value;
    setAiModelChoice(models.provider, next.value);
    model.set(next.label);
  });

  let themes = [];
  const themeLabel = (name) => (name === getActiveTheme() ? (getThemeManifest()?.label || name) : name.charAt(0).toUpperCase() + name.slice(1));
  const theme = row('ui/palette', t('settings.appearance.theme', null, 'Theme'), async () => {
    const next = cycle(themes.map((n) => ({ value: n })), getActiveTheme());
    if (!next) return;
    await switchTheme(next.value);
    theme.set(themeLabel(next.value));
  });

  const effect = row('ui/fx', t('wearable.animation', null, 'Animation'), () => {
    const options = orbEffectOptions();
    const next = cycle(options, getImmersive().orb_effect);
    if (!next) return;
    setImmersive({ orb_effect: next.value });
    effect.set(next.label);
  });

  const actions = el('div', 'wearable-panel-actions');
  actions.append(
    iconButton({ icon: 'ui/close', label: t('common.close', null, 'Close'), variant: 'quiet', iconSize: 20, onClick: () => api.close() }),
    iconButton({ icon: 'ui/monitor', label: t('wearable.leave', null, 'Leave the wearable face'), variant: 'quiet', iconSize: 20, onClick: () => setWearable(false) }),
  );

  body.append(swatches, model.el, theme.el, effect.el, actions);
  node.append(body);
  // A tap outside the content closes it, like the phone's sheets.
  node.addEventListener('click', (e) => { if (e.target === node) api.close(); });
  node.addEventListener('keydown', (e) => { if (e.key === 'Escape') { e.stopPropagation(); api.close(); } });
  host.append(node);

  const api = {
    open() {
      paintSwatches();
      const fx = orbEffectOptions().find((o) => o.value === getImmersive().orb_effect);
      effect.set(fx?.label);
      theme.set(themeLabel(getActiveTheme()));
      model.set('…');
      void loadAiModelChoices().then((choices) => {
        models = choices;
        model.set(choices.options.find((o) => o.value === choices.value)?.label);
      }).catch(() => model.set(t('common.defaultServer', null, 'Default (server)')));
      void listThemes().then((list) => { themes = list; });
      node.classList.remove('hidden');
      host.classList.add('is-settings');
      node.querySelector('button')?.focus();
    },
    close() {
      node.classList.add('hidden');
      host.classList.remove('is-settings');
    },
    destroy() { node.remove(); },
  };
  return api;
}

function openSettings() {
  if (!face) return;
  panel ||= buildPanel(face.node);
  panel.open();
}

/* ── Always listening for the wake word ──────────────────────── */

let rearmTimer = null;
let watchTimer = null;
let failures = 0;
let retryAt = 0;

function voiceReady() {
  const bar = document.getElementById('sphere-container');
  return !!bar && !bar.classList.contains('disabled');
}

/** Arm the wake listener when nothing else is going on. */
function armWake() {
  if (!isWearable() || !isAiEnabled() || !voiceReady()) return;
  // A locked screen does not listen; neither does a turn in flight, a reply
  // being spoken (the conversation window follows it) or a typed message.
  if (document.body.classList.contains('lock-active')) return;
  if (isListening() || isTurnActive() || isTextInputOpen()) return;
  // The voice bar going ready (sphere.js setVoiceReady) is an idle state too,
  // so the first arm needs no event of its own.
  const state = document.documentElement.dataset.voice || 'idle';
  if (state !== 'idle' && state !== 'error') return;
  if (Date.now() < retryAt) return;
  startListening('wake')
    .then(() => { failures = 0; })
    .catch(() => {
      // No microphone, or it was refused: back off instead of asking every
      // few seconds (5 s, 10 s, 20 s … up to a minute).
      failures += 1;
      retryAt = Date.now() + Math.min(60000, WATCH_MS * 2 ** (failures - 1));
    });
}

function scheduleWake() {
  clearTimeout(rearmTimer);
  rearmTimer = setTimeout(armWake, REARM_MS);
}

function onVoiceState(e) {
  if ((e.detail?.state || 'idle') === 'idle') scheduleWake();
}

function startWakeLoop() {
  window.addEventListener('voice:state', onVoiceState);
  window.addEventListener('lock:change', scheduleWake);
  clearInterval(watchTimer);
  watchTimer = setInterval(armWake, WATCH_MS);
  scheduleWake();
}

function stopWakeLoop() {
  window.removeEventListener('voice:state', onVoiceState);
  window.removeEventListener('lock:change', scheduleWake);
  clearTimeout(rearmTimer);
  clearInterval(watchTimer);
}

/* ── On / off ─────────────────────────────────────────────────── */

function apply() {
  const on = getWearable();
  if (on === isWearable() && (!on || face)) return;
  if (on) {
    root().dataset.wearable = '1';
    document.body.classList.add('wearable-active');
    face ||= buildFace();
    startWakeLoop();
  } else {
    delete root().dataset.wearable;
    document.body.classList.remove('wearable-active');
    stopWakeLoop();
    face?.destroy();
    face = null;
  }
  // Windows, the voice bar and the browser's pages re-check what is on screen.
  window.dispatchEvent(new CustomEvent('wearable:applied', { detail: { on } }));
  window.dispatchEvent(new Event('resize'));
}

let inited = false;

/** Start following the preference (once, after the desktop is up). */
export function initWearable() {
  if (inited) return;
  inited = true;
  window.addEventListener('wearable:change', apply);
  // A sign-in or an update reload brings this user's preference in.
  window.addEventListener('preferences:loaded', apply);
  apply();
}
