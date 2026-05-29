let map = null;
let baseTileLayer = null;
let userMarker = null;

const MAP_TILES = {
  dark: 'https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png',
  light: 'https://{s}.basemaps.cartocdn.com/light_all/{z}/{x}/{y}{r}.png',
};
let routeLayer = null;
let routeCasingLayer = null;
let routeTraveledLayer = null;
let destinationMarker = null;
let artifactMarker = null;
let navigationActive = false;
let navigatorFollow = false;
let lastMapPan = { lat: null, lon: null };
let displayBearing = 0;
let bearingInitialized = false;
let navRouteGeometry = null;
let currentHeading = 0;
let currentPos = { lat: 48.8566, lon: 2.3522 };
let gpsAccurate = false;

const NAV_ZOOM = 18;
const NAV_PUCK_FRAC = 0.62;

export function initMap() {
  map = L.map('map', {
    zoomControl: false,
    attributionControl: false,
    dragging: true,
    touchZoom: true,
    scrollWheelZoom: false,
    doubleClickZoom: false,
    boxZoom: false,
    keyboard: false,
  }).setView([currentPos.lat, currentPos.lon], 16);

  const theme = document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
  baseTileLayer = L.tileLayer(MAP_TILES[theme], {
    maxZoom: 19,
    crossOrigin: 'anonymous',
    subdomains: 'abcd',
  }).addTo(map);

  window.addEventListener('theme:change', (e) => {
    setMapTheme(e.detail?.theme);
  });

  userMarker = L.circleMarker([currentPos.lat, currentPos.lon], {
    radius: 7,
    fillColor: '#5eead4',
    fillOpacity: 0.95,
    color: '#fff',
    weight: 2.5,
    className: 'user-marker',
  }).addTo(map);

  window.addEventListener('gps:update', (e) => {
    updatePosition(e.detail);
  });

  window.addEventListener('artifact:pin', (e) => {
    showArtifactPin(e.detail);
  });

  window.addEventListener('artifact:clear', () => {
    if (artifactMarker) {
      map.removeLayer(artifactMarker);
      artifactMarker = null;
    }
  });

  window.addEventListener('artifact:route', (e) => {
    drawRouteGeometry(e.detail.geometry, { fit: e.detail.fit });
  });

  window.addEventListener('map:navigate', async (e) => {
    await navigateToDestination(e.detail);
  });

  window.addEventListener('map:resize', () => {
    if (!map) return;
    map.invalidateSize();
    if (navigatorFollow && lastMapPan.lat != null) {
      centerMapOnNavPuck(lastMapPan.lat, lastMapPan.lon);
      setMapBearing(displayBearing);
    } else if (navigationActive && routeLayer) {
      const bounds = routeLayer.getBounds();
      bounds.extend([currentPos.lat, currentPos.lon]);
      map.fitBounds(bounds, { ...getPanelFitPadding(), maxZoom: 13 });
    }
  });

  requestAnimationFrame(() => map.invalidateSize());
  setTimeout(() => map.invalidateSize(), 200);

  return map;
}

export function setMapTheme(theme) {
  if (!map) return;
  const key = theme === 'light' ? 'light' : 'dark';
  if (baseTileLayer) map.removeLayer(baseTileLayer);
  baseTileLayer = L.tileLayer(MAP_TILES[key], {
    maxZoom: 19,
    crossOrigin: 'anonymous',
    subdomains: 'abcd',
  }).addTo(map);
}

export function setNavigatorFollow(on, routeGeometry = null) {
  navigatorFollow = on;
  navRouteGeometry = routeGeometry;
  setNavigationMode(on);
  const puck = document.getElementById('nav-puck');
  if (puck) puck.classList.toggle('hidden', !on);
  if (userMarker) {
    const el = userMarker.getElement?.();
    if (el) el.style.opacity = on ? '0' : '1';
  }
  if (on && map) {
    lastMapPan = { lat: null, lon: null };
    bearingInitialized = false;
    displayBearing = currentHeading || 0;
    map.setZoom(NAV_ZOOM);
    updateNavigatorCamera({
      lat: currentPos.lat,
      lon: currentPos.lon,
      heading: currentHeading,
      routeGeometry: navRouteGeometry,
    });
    map.invalidateSize();
    requestAnimationFrame(() => map.invalidateSize());
  } else {
    lastMapPan = { lat: null, lon: null };
    displayBearing = 0;
    bearingInitialized = false;
    navRouteGeometry = null;
    clearMapBearing();
    if (userMarker) {
      const el = userMarker.getElement?.();
      if (el) el.style.opacity = '1';
    }
    map?.invalidateSize();
    requestAnimationFrame(() => map?.invalidateSize());
  }
}

export function setNavigatorRouteGeometry(geometry) {
  navRouteGeometry = geometry;
}

function bearingBetween(lat1, lon1, lat2, lon2) {
  const dLon = (lon2 - lon1) * Math.PI / 180;
  const y = Math.sin(dLon) * Math.cos(lat2 * Math.PI / 180);
  const x = Math.cos(lat1 * Math.PI / 180) * Math.sin(lat2 * Math.PI / 180)
    - Math.sin(lat1 * Math.PI / 180) * Math.cos(lat2 * Math.PI / 180) * Math.cos(dLon);
  return (Math.atan2(y, x) * 180 / Math.PI + 360) % 360;
}

function bearingAlongRoute(lat, lon, geometry, fromIdx = 0) {
  if (!geometry?.length) return null;
  let idx = Math.max(0, fromIdx);
  let ahead = 0;
  const target = 35;
  while (idx < geometry.length - 1 && ahead < target) {
    const seg = metersBetween(geometry[idx][0], geometry[idx][1], geometry[idx + 1][0], geometry[idx + 1][1]);
    ahead += seg;
    idx++;
  }
  const tip = geometry[Math.min(idx, geometry.length - 1)];
  return bearingBetween(lat, lon, tip[0], tip[1]);
}

function smoothBearing(target) {
  if (!bearingInitialized) {
    displayBearing = target;
    bearingInitialized = true;
    return displayBearing;
  }
  let diff = ((target - displayBearing + 540) % 360) - 180;
  if (Math.abs(diff) > 120) displayBearing = target;
  else displayBearing = (displayBearing + diff * 0.22 + 360) % 360;
  return displayBearing;
}

function puckOffsetPx() {
  const h = map?.getSize()?.y || window.innerHeight;
  return Math.round(h * (NAV_PUCK_FRAC - 0.5));
}

function getMapStage() {
  return document.getElementById('map-stage');
}

function navMapScale(bearing) {
  const rad = ((bearing % 90) + 90) % 90 * Math.PI / 180;
  const corner = Math.abs(Math.sin(rad)) + Math.abs(Math.cos(rad));
  return Math.max(1.4, corner * 1.08);
}

function setMapBearing(bearing) {
  const stage = getMapStage();
  if (!stage) return;
  const origin = `50% ${NAV_PUCK_FRAC * 100}%`;
  const scale = navMapScale(bearing);
  stage.style.transformOrigin = origin;
  stage.style.transform = `rotate(${-bearing}deg) scale(${scale})`;
}

function clearMapBearing() {
  const stage = getMapStage();
  if (!stage) return;
  stage.style.transform = '';
  stage.style.transformOrigin = '';
}

function centerMapOnNavPuck(lat, lon) {
  map.setView([lat, lon], map.getZoom(), { animate: false });
  map.panBy([0, -puckOffsetPx()], { animate: false });
}

/** Heading-up camera: center GPS under puck, then rotate map stage. */
export function updateNavigatorCamera({ lat, lon, heading, routeGeometry, routeIdx = 0 }) {
  if (!navigatorFollow || !map) return;

  const geom = routeGeometry || navRouteGeometry;
  let targetBearing = bearingAlongRoute(lat, lon, geom, routeIdx);
  if (targetBearing == null && heading != null && !isNaN(heading)) {
    targetBearing = heading;
  }
  if (targetBearing == null && lastMapPan.lat != null) {
    targetBearing = bearingBetween(lastMapPan.lat, lastMapPan.lon, lat, lon);
  }

  const moved = lastMapPan.lat == null
    || metersBetween(lastMapPan.lat, lastMapPan.lon, lat, lon) >= 1;
  if (moved) {
    lastMapPan = { lat, lon };
    centerMapOnNavPuck(lat, lon);
  }

  if (targetBearing != null) {
    smoothBearing(targetBearing);
  }
  setMapBearing(displayBearing);
}

function metersBetween(lat1, lon1, lat2, lon2) {
  const R = 6371000;
  const dLat = (lat2 - lat1) * Math.PI / 180;
  const dLon = (lon2 - lon1) * Math.PI / 180;
  const a = Math.sin(dLat / 2) ** 2
    + Math.cos(lat1 * Math.PI / 180) * Math.cos(lat2 * Math.PI / 180) * Math.sin(dLon / 2) ** 2;
  return R * 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));
}

export function updatePosition({ lat, lon, heading }) {
  currentPos = { lat, lon };
  gpsAccurate = true;
  if (heading != null && !isNaN(heading)) {
    currentHeading = heading;
  }
  userMarker.setLatLng([lat, lon]);

  if (!navigatorFollow && !navigationActive) {
    map.setView([lat, lon], map.getZoom(), { animate: true });
  }
}

function clearRouteLayers() {
  [routeLayer, routeCasingLayer, routeTraveledLayer].forEach((layer) => {
    if (layer) {
      map.removeLayer(layer);
    }
  });
  routeLayer = null;
  routeCasingLayer = null;
  routeTraveledLayer = null;
}

function getPanelFitPadding() {
  const panel = document.getElementById('travel-panel');
  const panelOpen = panel?.classList.contains('visible') && window.innerWidth >= 768;
  const panelW = panelOpen ? (panel.offsetWidth || 420) + 32 : 32;
  const hudTop =
    parseFloat(
      getComputedStyle(document.documentElement).getPropertyValue('--hud-header-total')
    ) || 58;
  return {
    paddingTopLeft: [panelW, hudTop + 12],
    paddingBottomRight: [32, 140],
  };
}

function setNavigationMode(active) {
  navigationActive = active;
  document.body.classList.toggle('nav-active', active && !navigatorFollow);
  if (!map) return;
  if (active && navigatorFollow) {
    map.dragging.disable();
    map.touchZoom.disable();
    map.scrollWheelZoom.disable();
  } else if (active) {
    map.dragging.enable();
    map.touchZoom.enable();
    map.scrollWheelZoom.enable();
  } else {
    map.dragging.disable();
    map.touchZoom.disable();
    map.scrollWheelZoom.disable();
  }
}

export function clearNavigation() {
  clearRouteLayers();
  if (destinationMarker) {
    map.removeLayer(destinationMarker);
    destinationMarker = null;
  }
  if (artifactMarker) {
    map.removeLayer(artifactMarker);
    artifactMarker = null;
  }
  if (!navigatorFollow) {
    setNavigationMode(false);
  }
}

function showDestinationMarker(coords) {
  if (destinationMarker) map.removeLayer(destinationMarker);
  destinationMarker = L.circleMarker([coords.lat, coords.lon], {
    radius: 10,
    fillColor: '#ff6b4a',
    fillOpacity: 1,
    color: '#fff',
    weight: 2.5,
  }).addTo(map);
}

function showArtifactPin(coords) {
  showDestinationMarker(coords);
}

function fitRouteView(points, dest) {
  const bounds = L.latLngBounds(points);
  bounds.extend([currentPos.lat, currentPos.lon]);
  if (dest) bounds.extend([dest.lat, dest.lon]);
  const padding = navigatorFollow
    ? { paddingTopLeft: [40, 80], paddingBottomRight: [40, 200] }
    : getPanelFitPadding();
  map.fitBounds(bounds, { ...padding, maxZoom: navigatorFollow ? NAV_ZOOM : 13 });
}

function normalizeGeometry(geometry) {
  return (geometry || []).map((pt) => {
    if (Array.isArray(pt)) return [pt[0], pt[1]];
    if (pt.lat != null) return [pt.lat, pt.lon];
    return null;
  }).filter(Boolean);
}

function drawRouteLines(points, { dashed = false, progressIdx = null } = {}) {
  const lineOpts = {
    lineCap: 'round',
    lineJoin: 'round',
    ...(dashed ? { dashArray: '10 14', opacity: 0.75 } : {}),
  };

  const traveled = progressIdx != null && progressIdx > 0
    ? points.slice(0, progressIdx + 1)
    : [];
  const remaining = progressIdx != null && progressIdx > 0
    ? points.slice(progressIdx)
    : points;

  if (traveled.length > 1) {
    routeTraveledLayer = L.polyline(traveled, {
      color: 'rgba(148, 163, 184, 0.45)',
      weight: 6,
      opacity: 0.6,
      ...lineOpts,
    }).addTo(map);
  }

  if (!dashed) {
    routeCasingLayer = L.polyline(remaining, {
      color: '#0f172a',
      weight: 11,
      opacity: 0.55,
      ...lineOpts,
    }).addTo(map);
  }

  routeLayer = L.polyline(remaining, {
    color: dashed ? '#fbbf24' : '#5eead4',
    weight: dashed ? 4 : 6,
    opacity: dashed ? 0.85 : 0.95,
    ...lineOpts,
  }).addTo(map);
}

export function drawRouteGeometry(geometry, { fit = false, navigate = false, dest = null, dashed = false } = {}) {
  if (!geometry?.length || !map) return false;
  clearRouteLayers();

  const points = normalizeGeometry(geometry);
  if (points.length < 2) return false;

  drawRouteLines(points, { dashed });

  if (navigate) {
    setNavigationMode(true);
    fitRouteView(points, dest);
  } else if (fit) {
    fitRouteView(points, dest);
  }
  map.invalidateSize();
  return true;
}

/** Navigator: draw route with traveled/remaining split. */
export function drawNavigatorRoute(geometry, { dest = null, fit = false, progressIdx = null } = {}) {
  if (!geometry?.length || !map) return false;
  clearRouteLayers();

  const points = normalizeGeometry(geometry);
  if (points.length < 2) return false;

  drawRouteLines(points, { progressIdx: progressIdx ?? null });

  if (dest) showDestinationMarker(dest);
  if (fit) fitRouteView(points, dest);
  map.invalidateSize();
  return true;
}

async function fetchOsrmRouteFull(fromLat, fromLon, toLat, toLon, profile = 'car') {
  const url = `https://router.project-osrm.org/route/v1/${profile}/${fromLon},${fromLat};${toLon},${toLat}?steps=true&geometries=geojson&overview=full`;
  const res = await fetch(url);
  if (!res.ok) return null;
  const data = await res.json();
  if (data.code !== 'Ok' || !data.routes?.[0]) return null;
  const route = data.routes[0];
  const geometry = route.geometry?.coordinates?.map(([lon, lat]) => [lat, lon]) || [];
  const leg = route.legs?.[0];
  const steps = (leg?.steps || []).map((s) => ({
    distance: s.distance,
    duration: s.duration,
    instruction: s.maneuver?.instruction || '',
  }));
  return {
    geometry,
    steps,
    distance_km: (route.distance || 0) / 1000,
    duration_min: (route.duration || 0) / 60,
  };
}

export async function fetchRouteData(fromLat, fromLon, toLat, toLon, profile = 'car') {
  const { apiFetch } = await import('./api.js');
  try {
    const res = await apiFetch(
      `/api/map/route?from_lat=${fromLat}&from_lon=${fromLon}&to_lat=${toLat}&to_lon=${toLon}&profile=${profile}`
    );
    const d = res.data;
    if (d?.geometry?.length > 1) {
      return {
        geometry: d.geometry,
        steps: d.steps || [],
        distance_km: (d.total_distance_meters || 0) / 1000,
        duration_min: (d.total_duration_seconds || 0) / 60,
      };
    }
  } catch (e) {
    console.warn('Backend route failed:', e);
  }
  return fetchOsrmRouteFull(fromLat, fromLon, toLat, toLon, profile);
}

export function refreshGpsPosition() {
  return new Promise((resolve) => {
    if (!navigator.geolocation) {
      resolve({ ...currentPos, accurate: gpsAccurate });
      return;
    }
    navigator.geolocation.getCurrentPosition(
      (pos) => {
        updatePosition({
          lat: pos.coords.latitude,
          lon: pos.coords.longitude,
          heading: pos.coords.heading ?? currentHeading,
        });
        resolve({ ...currentPos, accurate: true });
      },
      () => resolve({ ...currentPos, accurate: gpsAccurate }),
      { enableHighAccuracy: true, maximumAge: 0, timeout: 12000 }
    );
  });
}

export async function drawRoute(toLat, toLon, { navigate = false, artifactRef = null } = {}) {
  const dest = { lat: toLat, lon: toLon };
  const route = await fetchRouteData(currentPos.lat, currentPos.lon, toLat, toLon, 'car');
  if (route?.geometry?.length > 1) {
    return drawRouteGeometry(route.geometry, { navigate, fit: navigate, dest });
  }

  if (artifactRef) {
    artifactRef._routeError = 'Driving directions could not be loaded';
  }
  return false;
}

export function previewDestination(artifact) {
  const dest = resolveDestination(artifact);
  if (dest) showDestinationMarker(dest);
}

function resolveDestination(artifact) {
  if (artifact?.coordinates?.lat != null && artifact?.coordinates?.lon != null) {
    return artifact.coordinates;
  }
  const action = artifact?.actions?.find((a) => a.tool === 'map_route');
  const p = action?.params;
  if (p?.to_lat != null && p?.to_lon != null) {
    return { lat: Number(p.to_lat), lon: Number(p.to_lon) };
  }
  return null;
}

function drawDirectPath(dest) {
  const points = [
    [currentPos.lat, currentPos.lon],
    [dest.lat, dest.lon],
  ];
  return drawRouteGeometry(points, { navigate: true, dest, dashed: true });
}

export async function navigateToDestination(artifact) {
  if (!map) return { ok: false, mode: null };

  map.invalidateSize();
  delete artifact._routeError;
  delete artifact._routeFallback;

  const dest = resolveDestination(artifact);
  if (!dest) return { ok: false, mode: null };

  await refreshGpsPosition();
  showDestinationMarker(dest);

  const live = await drawRoute(dest.lat, dest.lon, { navigate: true, artifactRef: artifact });
  if (live) return { ok: true, mode: 'driving' };

  const direct = drawDirectPath(dest);
  if (direct) {
    artifact._routeFallback = true;
    return { ok: true, mode: 'direct' };
  }

  if (!artifact._routeError) {
    artifact._routeError = 'Could not draw a path to this destination';
  }
  return { ok: false, mode: null };
}

export function getCurrentPosition() {
  return { ...currentPos, heading: currentHeading };
}

export async function loadActiveRoute(tripId) {
  if (!tripId) {
    clearNavigation();
    return;
  }
  const { isNavigatorActive } = await import('./navigator.js');
  if (isNavigatorActive()) return;

  const { apiFetch } = await import('./api.js');
  try {
    const res = await apiFetch(`/api/trips/${tripId}/route`);
    clearRouteLayers();
    setNavigationMode(false);
    const points = res.data.map((p) => [p.lat, p.lon]);
    if (points.length > 1) {
      routeLayer = L.polyline(points, { color: '#5eead4', weight: 3, opacity: 0.5 }).addTo(map);
    }
  } catch (_) {}
}
