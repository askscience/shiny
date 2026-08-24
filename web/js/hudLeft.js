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
