/**
 * Top HUD bar: clock, local weather at GPS, horizontal saved destinations.
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
      meteoIconEl.src = `/icons/insights/${weatherIconStem(code)}.svg`;
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

function renderSavedTrips() {
  if (!tripsEl) return;
  const destinations = getSavedDestinations();
  const active = getActiveDestination();
  tripsEl.innerHTML = '';

  if (!destinations.length) {
    tripsEl.classList.add('empty');
    const hint = document.createElement('span');
    hint.className = 'hud-trips-empty';
    hint.textContent = 'No saved places yet';
    tripsEl.appendChild(hint);
    return;
  }

  tripsEl.classList.remove('empty');

  destinations.forEach((dest) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'hud-trip-chip';
    if (dest.key === active) btn.classList.add('active');
    btn.textContent = dest.label;
    btn.title = `Open ${dest.label}`;
    btn.setAttribute('aria-label', dest.label);
    btn.addEventListener('click', () => selectDestination(dest));
    tripsEl.appendChild(btn);
  });
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

export function initHudLeft() {
  formatClock();
  clockTimer = setInterval(formatClock, 1000);

  onPositionUpdate();
  window.addEventListener('gps:update', onPositionUpdate);

  renderSavedTrips();
  window.addEventListener('artifact:dock', renderSavedTrips);
  window.addEventListener('artifact:saved', renderSavedTrips);
  window.addEventListener('artifact:updated', renderSavedTrips);
}
