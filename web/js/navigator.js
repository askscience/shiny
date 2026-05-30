/**
 * Turn-by-turn navigator mode — minimal UI, live GPS, OSRM/OSM routing.
 */

import {
  drawNavigatorRoute,
  clearNavigation,
  setNavigatorFollow,
  setNavigatorRouteGeometry,
  fetchRouteData,
  getCurrentPosition,
  updateNavigatorCamera,
  refreshGpsPosition,
} from './map.js';
import { setHighFrequencyGps } from './gps.js';
import { clearArtifacts } from './artifacts.js';
import { clearInsightCards } from './insights/insightStore.js';

const banner = document.getElementById('nav-banner');
const instructionEl = document.getElementById('nav-instruction');
const metaEl = document.getElementById('nav-meta');
const destEl = document.getElementById('nav-dest');
const exitBtn = document.getElementById('nav-exit');

let active = false;
let session = null;
let steps = [];
let totalDistanceM = 0;
let totalDurationS = 0;
let traveledM = 0;
let currentStepIdx = 0;
let lastRerouteAt = 0;
let lastRouteDrawIdx = -1;
let lastRouteDrawAt = 0;
const REROUTE_COOLDOWN_MS = 15000;
const ARRIVAL_RADIUS_M = 45;
const OFF_ROUTE_M = 90;

function haversineM(lat1, lon1, lat2, lon2) {
  const R = 6371000;
  const dLat = (lat2 - lat1) * Math.PI / 180;
  const dLon = (lon2 - lon1) * Math.PI / 180;
  const a = Math.sin(dLat / 2) ** 2
    + Math.cos(lat1 * Math.PI / 180) * Math.cos(lat2 * Math.PI / 180) * Math.sin(dLon / 2) ** 2;
  return R * 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));
}

function polylineLengthM(points) {
  let d = 0;
  for (let i = 1; i < points.length; i++) {
    d += haversineM(points[i - 1][0], points[i - 1][1], points[i][0], points[i][1]);
  }
  return d;
}

function closestOnRoute(lat, lon, geometry) {
  let bestDist = Infinity;
  let bestIdx = 0;
  let offRoute = Infinity;
  for (let i = 0; i < geometry.length; i++) {
    const pt = geometry[i];
    const d = haversineM(lat, lon, pt[0], pt[1]);
    if (d < offRoute) offRoute = d;
    if (d < bestDist) {
      bestDist = d;
      bestIdx = i;
    }
  }
  const slice = geometry.slice(0, bestIdx + 1);
  const traveled = slice.length > 1 ? polylineLengthM(slice) : 0;
  return { idx: bestIdx, offRouteM: offRoute, traveledM: traveled };
}

function stepIndexForDistance(traveled) {
  if (!steps.length) return 0;
  let cum = 0;
  for (let i = 0; i < steps.length; i++) {
    cum += steps[i].distance || 0;
    if (traveled <= cum) return i;
  }
  return steps.length - 1;
}

function formatDistance(m) {
  if (m >= 1000) return `${(m / 1000).toFixed(1)} km`;
  return `${Math.round(m)} m`;
}

function formatDuration(sec) {
  const min = Math.max(1, Math.round(sec / 60));
  if (min >= 60) {
    const h = Math.floor(min / 60);
    const m = min % 60;
    return m ? `${h} h ${m} min` : `${h} h`;
  }
  return `${min} min`;
}

function updateBanner() {
  if (!banner || !session) return;

  const remainingM = Math.max(0, totalDistanceM - traveledM);
  const ratio = totalDistanceM > 0 ? remainingM / totalDistanceM : 1;
  const remainingS = totalDurationS * ratio;

  const step = steps[currentStepIdx];
  const instruction = step?.instruction?.trim()
    || (remainingM < ARRIVAL_RADIUS_M ? 'You have arrived' : 'Continue on route');

  if (instructionEl) instructionEl.textContent = instruction;
  if (metaEl) {
    metaEl.textContent = remainingM < ARRIVAL_RADIUS_M
      ? 'Navigation complete'
      : `${formatDistance(remainingM)} · ${formatDuration(remainingS)} remaining`;
  }
  if (destEl) destEl.textContent = session.destination;
}

function showBanner() {
  banner?.classList.remove('hidden');
}

function hideBanner() {
  banner?.classList.add('hidden');
}

function setUiActive(on) {
  document.body.classList.toggle('navigator-active', on);
  if (on) showBanner();
  else hideBanner();
}

async function rerouteFrom(lat, lon) {
  if (!session || Date.now() - lastRerouteAt < REROUTE_COOLDOWN_MS) return;
  lastRerouteAt = Date.now();

  const route = await fetchRouteData(lat, lon, session.to_lat, session.to_lon, session.profile || 'car');
  if (!route?.geometry?.length) return;

  session.geometry = route.geometry;
  steps = route.steps || [];
  setNavigatorRouteGeometry(route.geometry);
  totalDistanceM = (route.distance_km || 0) * 1000;
  totalDurationS = (route.duration_min || 0) * 60;
  traveledM = 0;
  currentStepIdx = 0;
  drawNavigatorRoute(route.geometry, { dest: { lat: session.to_lat, lon: session.to_lon } });
  updateBanner();
}

function onGpsUpdate({ lat, lon, heading, speed }) {
  if (!active || !session?.geometry?.length) return;

  const destDist = haversineM(lat, lon, session.to_lat, session.to_lon);
  if (destDist <= ARRIVAL_RADIUS_M) {
    if (instructionEl) instructionEl.textContent = 'You have arrived';
    if (metaEl) metaEl.textContent = session.destination;
    return;
  }

  const { offRouteM, traveledM: t, idx } = closestOnRoute(lat, lon, session.geometry);
  traveledM = t;
  currentStepIdx = stepIndexForDistance(traveledM);
  updateBanner();

  updateNavigatorCamera({
    lat,
    lon,
    heading,
    routeGeometry: session.geometry,
    routeIdx: idx,
  });

  const now = Date.now();
  if (idx !== lastRouteDrawIdx || now - lastRouteDrawAt > 800) {
    lastRouteDrawIdx = idx;
    lastRouteDrawAt = now;
    drawNavigatorRoute(session.geometry, {
      dest: { lat: session.to_lat, lon: session.to_lon },
      progressIdx: idx,
    });
  }

  if (offRouteM > OFF_ROUTE_M) {
    void rerouteFrom(lat, lon);
  }
}

export function isNavigatorActive() {
  return active;
}

export async function startNavigator(nav) {
  if (!nav?.geometry?.length) return false;

  clearArtifacts();
  clearInsightCards();
  window.dispatchEvent(new CustomEvent('artifact:dock'));

  session = {
    destination: nav.destination || 'Destination',
    to_lat: nav.to_lat,
    to_lon: nav.to_lon,
    geometry: nav.geometry,
    profile: nav.profile || 'car',
  };
  steps = nav.steps || [];
  totalDistanceM = (nav.distance_km || 0) * 1000;
  totalDurationS = (nav.duration_min || 0) * 60;
  traveledM = 0;
  currentStepIdx = 0;
  lastRerouteAt = Date.now();
  active = true;

  setUiActive(true);
  setHighFrequencyGps(true);
  await refreshGpsPosition();

  setNavigatorFollow(true, nav.geometry);

  drawNavigatorRoute(nav.geometry, {
    dest: { lat: nav.to_lat, lon: nav.to_lon },
    fit: false,
  });

  const pos = getCurrentPosition();
  onGpsUpdate({ ...pos, heading: pos.heading });

  updateBanner();
  window.dispatchEvent(new CustomEvent('navigator:started', { detail: session }));
  return true;
}

export function stopNavigator() {
  if (!active) return;
  active = false;
  session = null;
  steps = [];
  setUiActive(false);
  setNavigatorFollow(false);
  setHighFrequencyGps(false);
  clearNavigation();
  window.dispatchEvent(new CustomEvent('navigator:stopped'));
}

export function initNavigator() {
  exitBtn?.addEventListener('click', () => {
    stopNavigator();
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: 'Navigation ended', type: 'info' },
    }));
  });

  window.addEventListener('gps:update', (e) => onGpsUpdate(e.detail));
}
