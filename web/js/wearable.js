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
import { getBackground, setBackground, renderBackgroundPresets, uploadBackground } from './background.js';
import { pickFiles } from './files.js';
import { loadAiModelChoices, setAiModelChoice } from './settings/assistant.js';
import {
  iconButton, modal, select, toggleRow, setIcon, listThemes, getActiveTheme, getThemeManifest, switchTheme,
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
    if (e.target.closest('button')) return;
    actions.listen?.();
  });

  const dotGrid = mountVoiceDots(dots, { face: true });

  function tick() {
    const now = new Date();
    const zone = getWeatherState()?.timezone || undefined;
    const locale = getLocale();
    time.textContent = formatLocationTime(now, zone, locale);
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
      settingsDialog?.close();
      node.remove();
    },
  };
}

/* ── Settings: wallpaper, AI model, theme, animation ─────────── */

let settingsDialog = null;

function labelled(text, control) {
  const wrap = el('label', 'wearable-field');
  wrap.append(el('span', 'wearable-field-label', text), control);
  return wrap;
}

function buildSettings() {
  const wallpapers = el('div', 'bg-preset-grid wearable-wallpapers');
  wallpapers.setAttribute('role', 'radiogroup');
  wallpapers.setAttribute('aria-label', t('wearable.wallpaper', null, 'Wallpaper'));
  const paintWallpapers = () => renderBackgroundPresets(wallpapers, {
    onSelect: (preset) => {
      setBackground({ mode: 'image', preset: preset.id, url: null });
      paintWallpapers();
    },
    onUpload: async () => {
      const [file] = await pickFiles({ accept: 'image/*' });
      if (file && await uploadBackground(file)) paintWallpapers();
    },
  });

  let provider = 'ollama';
  const model = select({ onChange: (value) => setAiModelChoice(provider, value) });
  const loadModels = async () => {
    try {
      const choices = await loadAiModelChoices();
      provider = choices.provider;
      model.setOptions(choices.options);
      model.select.value = choices.value;
    } catch (_) {
      model.setOptions([{ value: '', label: t('common.defaultServer', null, 'Default (server)') }]);
    }
  };

  const theme = select({ onChange: (value) => switchTheme(value, { origin: theme.select }) });
  const loadThemes = () => listThemes().then((themes) => {
    theme.setOptions(themes.map((name) => ({
      value: name,
      label: name === getActiveTheme() ? (getThemeManifest()?.label || name) : name.charAt(0).toUpperCase() + name.slice(1),
    })));
    theme.select.value = getActiveTheme();
  });

  const effect = select({
    options: orbEffectOptions(),
    onChange: (value) => setImmersive({ orb_effect: value }),
  });

  const leave = toggleRow({
    label: t('wearable.label', null, 'Wearable'),
    checked: true,
    onChange: (on) => { if (!on) setWearable(false); },
  });

  const dialog = modal({
    title: t('hud.settings', null, 'Settings'),
    label: t('hud.settings', null, 'Settings'),
    body: [
      wallpapers,
      labelled(t('settings.assistant.aiModel', null, 'AI model'), model),
      labelled(t('settings.appearance.theme', null, 'Theme'), theme),
      labelled(t('wearable.animation', null, 'Animation'), effect),
      leave,
    ],
  });
  dialog.el.classList.add('wearable-settings-dialog');

  return {
    ...dialog,
    refresh() {
      paintWallpapers();
      void loadModels();
      void loadThemes();
      effect.select.value = getImmersive().orb_effect;
    },
  };
}

function openSettings() {
  settingsDialog ||= buildSettings();
  settingsDialog.refresh();
  settingsDialog.open();
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
    settingsDialog = null;
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
