import { apiFetch } from './api.js';

let watchId = null;
let activeTripId = null;
let lastSubmit = 0;
let highFrequency = false;
const SUBMIT_INTERVAL_MS = 30000;

const GPS_OPTS_NORMAL = { enableHighAccuracy: true, maximumAge: 5000, timeout: 15000 };
const GPS_OPTS_NAV = { enableHighAccuracy: true, maximumAge: 2000, timeout: 10000 };

let lastEmitAt = 0;
let lastEmitLat = null;
let lastEmitLon = null;

export function setHighFrequencyGps(on) {
  if (highFrequency === on) return;
  highFrequency = on;
  lastEmitAt = 0;
  lastEmitLat = null;
  lastEmitLon = null;
  if (watchId != null) {
    stopGpsTracking();
    startGpsTracking();
  }
}

function metersBetween(lat1, lon1, lat2, lon2) {
  const R = 6371000;
  const dLat = (lat2 - lat1) * Math.PI / 180;
  const dLon = (lon2 - lon1) * Math.PI / 180;
  const a = Math.sin(dLat / 2) ** 2
    + Math.cos(lat1 * Math.PI / 180) * Math.cos(lat2 * Math.PI / 180) * Math.sin(dLon / 2) ** 2;
  return R * 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));
}

function shouldEmitPosition(lat, lon) {
  if (!highFrequency) return true;
  const now = Date.now();
  if (lastEmitLat == null) return true;
  const dist = metersBetween(lastEmitLat, lastEmitLon, lat, lon);
  if (dist >= 10 || now - lastEmitAt >= 1000) return true;
  return false;
}

function markEmitted(lat, lon) {
  lastEmitAt = Date.now();
  lastEmitLat = lat;
  lastEmitLon = lon;
}

export function startGpsTracking() {
  if (!navigator.geolocation) return;

  watchId = navigator.geolocation.watchPosition(
    (pos) => {
      const { latitude: lat, longitude: lon, heading, speed, altitude, accuracy } = pos.coords;
      if (!shouldEmitPosition(lat, lon)) return;
      markEmitted(lat, lon);

      const gpsDot = document.getElementById('gps-dot');
      if (gpsDot) gpsDot.className = 'dot on';

      const derivedHeading = heading ?? deriveHeading(lat, lon, highFrequency);
      window.dispatchEvent(new CustomEvent('gps:update', {
        detail: { lat, lon, heading: derivedHeading, speed, altitude, accuracy },
      }));

      submitLocation(lat, lon, derivedHeading, speed, altitude);
    },
    () => {
      const gpsDot = document.getElementById('gps-dot');
      if (gpsDot) gpsDot.className = 'dot off';
    },
    highFrequency ? GPS_OPTS_NAV : GPS_OPTS_NORMAL
  );
}

let prevLat = null;
let prevLon = null;

function deriveHeading(lat, lon, navMode) {
  if (prevLat == null) {
    prevLat = lat;
    prevLon = lon;
    return currentHeadingCache;
  }
  const dist = metersBetween(prevLat, prevLon, lat, lon);
  if (navMode && dist < 5) return currentHeadingCache;
  if (dist < 2) return currentHeadingCache;

  const fromLat = prevLat;
  const fromLon = prevLon;
  prevLat = lat;
  prevLon = lon;

  const dLon = (lon - fromLon) * Math.PI / 180;
  const y = Math.sin(dLon) * Math.cos(lat * Math.PI / 180);
  const x = Math.cos(fromLat * Math.PI / 180) * Math.sin(lat * Math.PI / 180)
    - Math.sin(fromLat * Math.PI / 180) * Math.cos(lat * Math.PI / 180) * Math.cos(dLon);
  currentHeadingCache = ((Math.atan2(y, x) * 180 / Math.PI) + 360) % 360;
  return currentHeadingCache;
}

let currentHeadingCache = 0;

async function submitLocation(lat, lon, heading, speed, altitude) {
  const now = Date.now();
  if (now - lastSubmit < SUBMIT_INTERVAL_MS) return;
  if (!activeTripId) return;
  lastSubmit = now;
  try {
    await apiFetch('/api/locations', {
      method: 'POST',
      body: JSON.stringify({
        latitude: lat,
        longitude: lon,
        heading: heading ?? undefined,
        speed: speed ?? undefined,
        altitude: altitude ?? undefined,
        trip_id: activeTripId,
      }),
    });
  } catch (_) {}
}

export async function refreshActiveTrip() {
  try {
    const res = await apiFetch('/api/trips/active');
    activeTripId = res.data?.id || null;
    const nameEl = document.getElementById('trip-name');
    if (nameEl) nameEl.textContent = res.data?.name || 'No active trip';
    return res.data;
  } catch (_) {
    return null;
  }
}

export function getActiveTripId() {
  return activeTripId;
}

export function stopGpsTracking() {
  if (watchId != null) {
    navigator.geolocation.clearWatch(watchId);
    watchId = null;
  }
}
