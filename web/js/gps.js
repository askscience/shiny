import { apiFetch } from './api.js';

let watchId = null;
let activeTripId = null;
let lastSubmit = 0;
const SUBMIT_INTERVAL_MS = 30000;

export function startGpsTracking() {
  if (!navigator.geolocation) return;

  watchId = navigator.geolocation.watchPosition(
    (pos) => {
      const { latitude: lat, longitude: lon, heading, speed, altitude } = pos.coords;
      document.getElementById('gps-dot').className = 'dot on';

      window.dispatchEvent(new CustomEvent('gps:update', {
        detail: { lat, lon, heading: heading ?? deriveHeading(lat, lon), speed, altitude },
      }));

      submitLocation(lat, lon, heading, speed, altitude);
    },
    () => {
      document.getElementById('gps-dot').className = 'dot off';
    },
    { enableHighAccuracy: true, maximumAge: 5000, timeout: 15000 }
  );
}

let prevLat = null;
let prevLon = null;

function deriveHeading(lat, lon) {
  if (prevLat == null) { prevLat = lat; prevLon = lon; return 0; }
  const dLon = (lon - prevLon) * Math.PI / 180;
  const y = Math.sin(dLon) * Math.cos(lat * Math.PI / 180);
  const x = Math.cos(prevLat * Math.PI / 180) * Math.sin(lat * Math.PI / 180) -
    Math.sin(prevLat * Math.PI / 180) * Math.cos(lat * Math.PI / 180) * Math.cos(dLon);
  prevLat = lat;
  prevLon = lon;
  return ((Math.atan2(y, x) * 180 / Math.PI) + 360) % 360;
}

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
