/**
 * Top HUD bar.
 * - Clock + local weather: CORE chrome — always on, plugin-free.
 * - Saved destination chips: traveler plugin content — only with the plugin.
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
import { setIcon } from '../ui/index.js';

const clockTimeEl = document.getElementById('hud-clock-time');
const clockDateEl = document.getElementById('hud-clock-date');
const meteoEl = document.getElementById('hud-meteo');
const meteoIconEl = document.getElementById('hud-meteo-icon');
const meteoTempEl = document.getElementById('hud-meteo-temp');
const meteoLabelEl = document.getElementById('hud-meteo-label');
const tripsEl = document.getElementById('hud-saved-trips');
const tripsMobileEl = document.getElementById('hud-saved-trips-mobile');

const mobileQuery = window.matchMedia('(max-width: 767px)');

let clockTimer = null;
let lastWeatherKey = '';
let lastWeatherAt = 0;
const WEATHER_TTL_MS = 10 * 60 * 1000;

const WMO_LABEL = {
  0: 'Clear',
  1: 'Mainly clear',
  2: 'Partly cloudy',
  3: 'Cloudy',
  45: 'Fog',
  48: 'Fog',
  51: 'Drizzle',
  53: 'Drizzle',
  55: 'Drizzle',
  61: 'Rain',
  63: 'Rain',
  65: 'Heavy rain',
  71: 'Snow',
  73: 'Snow',
  75: 'Snow',
  80: 'Showers',
  81: 'Showers',
  82: 'Heavy showers',
  95: 'Thunderstorm',
  96: 'Thunderstorm',
  99: 'Thunderstorm',
};

function weatherIconStem(code) {
  switch (code) {
    case 0: return 'weather-sun';
    case 1:
    case 2:
    case 3: return 'weather-partly';
    case 45:
    case 48: return 'weather-fog';
    case 51:
    case 53:
    case 55: return 'weather-drizzle';
    case 61:
    case 63:
    case 65:
    case 80:
    case 81:
    case 82: return 'weather-rain';
    case 71:
    case 73:
    case 75:
    case 77: return 'weather-snow';
    case 95:
    case 96:
    case 99: return 'weather-storm';
    default: return 'weather-cloud';
  }
}

function weatherLabel(code) {
  return WMO_LABEL[code] ?? 'Cloudy';
}

function formatClock() {
  const now = new Date();
  const locale = navigator.language || 'en-US';
  if (clockTimeEl) {
    clockTimeEl.textContent = now.toLocaleTimeString(locale, {
      hour: '2-digit',
      minute: '2-digit',
    });
  }
  if (clockDateEl) {
    clockDateEl.textContent = now.toLocaleDateString(locale, {
      weekday: 'short',
      day: 'numeric',
      month: 'short',
    });
  }
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
    url.searchParams.set('timezone', 'auto');

    const res = await fetch(url);
    if (!res.ok) throw new Error('forecast failed');
    const data = await res.json();
    const cur = data.current;
    if (!cur) throw new Error('no current');

    const code = cur.weather_code ?? 0;
    const temp = Math.round(cur.temperature_2m ?? 0);
    if (meteoIconEl) {
      void setIcon(meteoIconEl, `insights/${weatherIconStem(code)}`);
    }
    if (meteoTempEl) meteoTempEl.textContent = `${temp}°`;
    if (meteoLabelEl) meteoLabelEl.textContent = weatherLabel(code);
    lastWeatherKey = key;
    lastWeatherAt = now;
    meteoEl.classList.remove('unavailable');
  } catch {
    if (meteoTempEl) meteoTempEl.textContent = '—';
    if (meteoLabelEl) meteoLabelEl.textContent = 'Weather unavailable';
    meteoEl.classList.add('unavailable');
  } finally {
    meteoEl.classList.remove('loading');
  }
}

function buildTripChip(dest, active) {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'ui-hud-chip';
  if (dest.key === active) btn.classList.add('is-active');
  btn.textContent = dest.label;
  btn.title = `Open ${dest.label}`;
  btn.setAttribute('aria-label', dest.label);
  btn.addEventListener('click', () => selectDestination(dest));
  return btn;
}

function renderInto(container, destinations, active) {
  if (!container) return;
  container.innerHTML = '';

  if (!destinations.length) {
    container.classList.add('empty');
    const hint = document.createElement('span');
    hint.className = 'hud-trips-empty';
    hint.textContent = 'No saved places yet';
    container.appendChild(hint);
    return;
  }

  container.classList.remove('empty');
  destinations.forEach((dest) => {
    container.appendChild(buildTripChip(dest, active));
  });
}

function renderSavedTrips() {
  const destinations = getSavedDestinations();
  const active = getActiveDestination();
  const isMobile = mobileQuery.matches;

  if (isMobile) {
    renderInto(tripsMobileEl, destinations, active);
    if (tripsEl) {
      tripsEl.innerHTML = '';
      tripsEl.classList.add('empty');
    }
  } else {
    renderInto(tripsEl, destinations, active);
    if (tripsMobileEl) {
      tripsMobileEl.innerHTML = '';
      tripsMobileEl.classList.add('empty');
    }
  }
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
  if (!navigator.geolocation) return;
  navigator.geolocation.getCurrentPosition(
    (pos) => void refreshLocalWeather(pos.coords.latitude, pos.coords.longitude),
    () => {},
    { maximumAge: WEATHER_TTL_MS, timeout: 12000 },
  );
}

let clockInited = false;
let tripsInited = false;

/** Core chrome: clock + weather. Works with zero plugins installed. */
export function initHudClock() {
  if (clockInited) return;
  clockInited = true;

  formatClock();
  clockTimer = setInterval(formatClock, 1000);

  refreshWeatherAtCurrentPosition();
  // Traveler's GPS tracker refines the fix via gps:update; without it the
  // periodic browser fix keeps the weather fresh on its own.
  window.addEventListener('gps:update', onPositionUpdate);
  setInterval(refreshWeatherAtCurrentPosition, WEATHER_TTL_MS);
}

/** Traveler plugin content: saved-destination chips in the HUD. */
export function initHudTrips() {
  if (tripsInited) return;
  tripsInited = true;

  renderSavedTrips();
  mobileQuery.addEventListener('change', renderSavedTrips);
  window.addEventListener('artifact:dock', renderSavedTrips);
  window.addEventListener('artifact:saved', renderSavedTrips);
  window.addEventListener('artifact:updated', renderSavedTrips);
}

/** Back-compat: both halves (used where traveler is known active). */
export function initHudLeft() {
  initHudClock();
  initHudTrips();
}
