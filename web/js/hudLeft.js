/**
 * Top HUD bar.
 * - Clock + local weather: CORE chrome — always on, plugin-free.
 * - Saved places menu: traveler plugin content — only with the plugin.
 */

import {
  getSavedDestinations,
  getActiveDestination,
  setActiveDestination,
  getSummaries,
  destinationKeyForSummary,
} from './artifactStore.js';
import { openSavedArtifact } from './artifacts.js';
import { getCurrentPosition } from './map.js';
import { notifyMenuChange } from './menuState.js';
import { setIcon } from '../ui/index.js';
import { tempText, weatherIconStem, weatherLabel } from './weatherShared.js';
import { formatLocationDate, formatLocationTime } from './clockShared.js';

const clockEl = document.querySelector('.hud-clock');
const clockTimeEl = document.getElementById('hud-clock-time');
const clockDateEl = document.getElementById('hud-clock-date');
const meteoEl = document.getElementById('hud-meteo');
const meteoIconEl = document.getElementById('hud-meteo-icon');
const meteoTempEl = document.getElementById('hud-meteo-temp');
const meteoLabelEl = document.getElementById('hud-meteo-label');
const tripsEl = document.getElementById('hud-saved-trips');

let clockTimer = null;
let lastWeatherKey = '';
let lastWeatherAt = 0;
const WEATHER_TTL_MS = 10 * 60 * 1000;

/**
 * The latest fix + forecast, `null` until the first one lands. The chip, the
 * clock's zone, the clock popover and the weather menu all read it, and every
 * new copy rides out on `weather:update`.
 */
let weatherState = null;
/** IANA zone of the located place — the clock follows it, not the OS. */
let locationTimeZone = null;
/** Why there is nothing to show yet ('Location unavailable', …); null when fine. */
let weatherError = null;

/** Time and date of the located place (the OS zone until one is known). */
function formatClock() {
  const now = new Date();
  const locale = navigator.language || 'en-US';
  if (clockTimeEl) {
    clockTimeEl.textContent = formatLocationTime(now, locationTimeZone, locale);
  }
  if (clockDateEl) {
    clockDateEl.textContent = formatLocationDate(now, locationTimeZone, locale);
  }
}

/** Open-Meteo's parallel `daily` arrays as one row per day. */
function forecastDays(daily) {
  if (!Array.isArray(daily?.time)) return [];
  return daily.time.map((date, i) => ({
    date,
    code: daily.weather_code?.[i] ?? 0,
    min: daily.temperature_2m_min?.[i],
    max: daily.temperature_2m_max?.[i],
  }));
}

/** Nothing to show yet — record and say which half of the chain failed. */
function markWeatherUnavailable(reason) {
  weatherError = reason;
  window.dispatchEvent(new CustomEvent('weather:update', { detail: weatherState }));
  if (weatherState) return;
  if (meteoTempEl) meteoTempEl.textContent = '—';
  if (meteoLabelEl) meteoLabelEl.textContent = reason;
  meteoEl?.classList.add('unavailable');
}

async function refreshLocalWeather(lat, lon) {
  if (!meteoEl || lat == null || lon == null) return;
  const key = `${lat.toFixed(2)},${lon.toFixed(2)}`;
  const now = Date.now();
  if (key === lastWeatherKey && now - lastWeatherAt < WEATHER_TTL_MS) return;

  meteoEl.classList.add('loading');
  try {
    const url = new URL('https://api.open-meteo.com/v1/forecast');
    url.searchParams.set('latitude', String(lat));
    url.searchParams.set('longitude', String(lon));
    url.searchParams.set('current', 'temperature_2m,weather_code');
    // The menu's five days, and the place's own zone — the clock reads its
    // time from here rather than from the OS.
    url.searchParams.set('daily', 'weather_code,temperature_2m_max,temperature_2m_min');
    url.searchParams.set('forecast_days', '5');
    url.searchParams.set('timezone', 'auto');

    const res = await fetch(url);
    if (!res.ok) throw new Error('forecast failed');
    const data = await res.json();
    const cur = data.current;
    if (!cur) throw new Error('no current');

    const code = cur.weather_code ?? 0;
    weatherState = {
      lat,
      lon,
      timezone: data.timezone || null,
      timezoneAbbreviation: data.timezone_abbreviation || null,
      current: { code, temp: cur.temperature_2m },
      daily: forecastDays(data.daily),
      at: now,
    };
    locationTimeZone = weatherState.timezone;
    weatherError = null;
    if (clockEl && locationTimeZone) {
      clockEl.title = `Time in ${locationTimeZone} — click for the calendar`;
    }

    if (meteoIconEl) {
      void setIcon(meteoIconEl, `insights/${weatherIconStem(code)}`);
    }
    if (meteoTempEl) meteoTempEl.textContent = tempText(weatherState.current.temp);
    if (meteoLabelEl) meteoLabelEl.textContent = weatherLabel(code);
    meteoEl.title = `Weather at your position${locationTimeZone ? ` — ${locationTimeZone}` : ''}`;
    meteoEl.classList.remove('unavailable');
    lastWeatherKey = key;
    lastWeatherAt = now;
    formatClock();
    window.dispatchEvent(new CustomEvent('weather:update', { detail: weatherState }));
  } catch {
    // A later refresh keeps the last good reading; only the first failure has
    // nothing to show, and it must say so instead of leaving an empty chip.
    markWeatherUnavailable('Weather unavailable');
  } finally {
    meteoEl.classList.remove('loading');
  }
}

/* ── Saved-places menu (replaces the chip row) ─────────────── */

let menuTrigger = null;
let menuPopup = null;
let menuOpen = false;

function ensureMenuPopup() {
  if (menuPopup) return;
  menuPopup = document.createElement('div');
  menuPopup.className = 'ui-hud-menu-popup hidden';
  menuPopup.setAttribute('role', 'menu');
  menuPopup.setAttribute('aria-label', 'Saved places');
  document.body.appendChild(menuPopup);
}

function closeTripMenu() {
  if (!menuOpen) return;
  menuOpen = false;
  if (menuTrigger) menuTrigger.setAttribute('aria-expanded', 'false');
  menuPopup?.classList.add('hidden');
  document.removeEventListener('pointerdown', onMenuOutside, true);
  document.removeEventListener('keydown', onMenuKey, true);
  window.removeEventListener('resize', closeTripMenu);
  notifyMenuChange();
}

function onMenuOutside(e) {
  if (menuPopup && !menuPopup.contains(e.target) && menuTrigger && !menuTrigger.contains(e.target)) {
    closeTripMenu();
  }
}

function onMenuKey(e) {
  if (e.key === 'Escape') closeTripMenu();
}

function openTripMenu() {
  ensureMenuPopup();
  renderMenuItems();
  menuPopup.classList.remove('hidden');
  const r = menuTrigger.getBoundingClientRect();
  const left = Math.max(12, Math.min(r.left, window.innerWidth - menuPopup.offsetWidth - 12));
  menuPopup.style.left = `${left}px`;
  menuPopup.style.top = `${r.bottom + 8}px`;
  menuOpen = true;
  menuTrigger.setAttribute('aria-expanded', 'true');
  document.addEventListener('pointerdown', onMenuOutside, true);
  document.addEventListener('keydown', onMenuKey, true);
  window.addEventListener('resize', closeTripMenu);
  notifyMenuChange();
}

function toggleTripMenu() {
  if (menuOpen) closeTripMenu();
  else if (menuTrigger) openTripMenu();
}

function renderMenuItems() {
  if (!menuPopup) return;
  menuPopup.innerHTML = '';
  const destinations = getSavedDestinations();
  const active = getActiveDestination();

  if (!destinations.length) {
    const empty = document.createElement('div');
    empty.className = 'ui-hud-menu-empty';
    empty.textContent = 'No saved places yet';
    menuPopup.appendChild(empty);
    return;
  }

  destinations.forEach((dest) => {
    const item = document.createElement('button');
    item.type = 'button';
    item.className = 'ui-hud-menu-item';
    if (dest.key === active) item.classList.add('is-active');
    item.setAttribute('role', 'menuitem');

    const label = document.createElement('span');
    label.className = 'ui-hud-menu-item-label';
    label.textContent = dest.label;
    item.appendChild(label);

    const check = document.createElement('span');
    check.className = 'ui-hud-menu-check';
    item.appendChild(check);
    void setIcon(check, 'ui/check', { size: 14 });

    item.addEventListener('click', () => {
      closeTripMenu();
      void selectDestination(dest);
    });
    menuPopup.appendChild(item);
  });
}

function buildTripMenuTrigger(destinations, active) {
  const trigger = document.createElement('button');
  trigger.type = 'button';
  trigger.className = 'ui-hud-menu-trigger';
  trigger.setAttribute('aria-haspopup', 'menu');
  trigger.setAttribute('aria-expanded', 'false');

  const label = document.createElement('span');
  label.className = 'ui-hud-menu-label';
  const activeDest = destinations.find((d) => d.key === active);
  label.textContent = activeDest?.label || 'Trips';
  trigger.appendChild(label);

  const chevron = document.createElement('span');
  chevron.className = 'ui-hud-menu-chevron';
  trigger.appendChild(chevron);
  void setIcon(chevron, 'ui/chevron-down', { size: 13 });

  trigger.addEventListener('click', (e) => {
    e.stopPropagation();
    toggleTripMenu();
  });
  return trigger;
}

function renderInto(container, destinations, active) {
  if (!container) return;
  closeTripMenu();
  container.innerHTML = '';
  container.classList.toggle('empty', !destinations.length);
  menuTrigger = buildTripMenuTrigger(destinations, active);
  container.appendChild(menuTrigger);
}

function renderSavedTrips() {
  const destinations = getSavedDestinations();
  const active = getActiveDestination();
  renderInto(tripsEl, destinations, active);
}

async function selectDestination(dest) {
  setActiveDestination(dest.key);
  renderSavedTrips();

  const list = getSummaries().filter((s) => destinationKeyForSummary(s) === dest.key);
  const plan =
    list.find((s) => (s.type || s.artifact_type) === 'travel_plan') ||
    list.find((s) => s.theme === 'overview') ||
    list[0];

  const id = plan?.id || dest.artifactId;
  if (id) await openSavedArtifact(id);
}

function onPositionUpdate() {
  const pos = getCurrentPosition();
  if (pos?.lat != null && pos?.lon != null) {
    void refreshLocalWeather(pos.lat, pos.lon);
  }
}

/** One geolocation fix straight from the browser — no map/plugin needed. */
function refreshWeatherAtCurrentPosition() {
  if (!navigator.geolocation) {
    markWeatherUnavailable('Location unavailable');
    return;
  }
  navigator.geolocation.getCurrentPosition(
    (pos) => void refreshLocalWeather(pos.coords.latitude, pos.coords.longitude),
    // A denied permission (the shell's policy) or a missing position source
    // lands here: the chip has to name that instead of sitting silent at "—°".
    () => markWeatherUnavailable('Location unavailable'),
    { maximumAge: WEATHER_TTL_MS, timeout: 12000 },
  );
}

/** The latest fix + forecast, for the weather menu and the clock popover. */
export function getWeatherState() {
  return weatherState;
}

/** Why the chip/menu have nothing to show, or `null` once a fix landed. */
export function getWeatherError() {
  return weatherError;
}

/** Ask for a fresh fix now — the weather menu's first open. */
export function refreshWeatherNow() {
  refreshWeatherAtCurrentPosition();
}

let clockInited = false;
let tripsInited = false;

/** Core chrome: clock + weather. Works with zero plugins installed. */
export function initHudClock() {
  if (clockInited) return;
  clockInited = true;

  if (clockEl) clockEl.title = 'Time and calendar';
  formatClock();
  clockTimer = setInterval(formatClock, 1000);

  refreshWeatherAtCurrentPosition();
  // Traveler's GPS tracker refines the fix via gps:update; without it the
  // periodic browser fix keeps the weather fresh on its own.
  window.addEventListener('gps:update', onPositionUpdate);
  setInterval(refreshWeatherAtCurrentPosition, WEATHER_TTL_MS);
}

/** Traveler plugin content: saved-destination menu in the HUD top bar. */
export function initHudTrips() {
  if (tripsInited) return;
  tripsInited = true;

  renderSavedTrips();
  window.addEventListener('artifact:dock', renderSavedTrips);
  window.addEventListener('artifact:saved', renderSavedTrips);
  window.addEventListener('artifact:updated', renderSavedTrips);
}
