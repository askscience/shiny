let map = null;
let baseTileLayer = null;
let userMarker = null;

const MAP_TILES = {
  dark: 'https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png',
  light: 'https://{s}.basemaps.cartocdn.com/light_all/{z}/{x}/{y}{r}.png',
};
let routeLayer = null;
let routeCasingLayer = null;
let destinationMarker = null;
let artifactMarker = null;
let navigationActive = false;
let currentHeading = 0;
let currentPos = { lat: 48.8566, lon: 2.3522 };
let gpsAccurate = false;

export function initMap() {
  map = L.map('map', {
    zoomControl: false,
    attributionControl: false,
    dragging: false,
    touchZoom: false,
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
    radius: 6,
    fillColor: '#7dd3fc',
    fillOpacity: 0.95,
    color: '#fff',
    weight: 2,
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
    // Keep the route visible while navigating — closing the panel should not erase it
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
    if (navigationActive && routeLayer) {
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

export function updatePosition({ lat, lon, heading }) {
  currentPos = { lat, lon };
  gpsAccurate = true;
  if (heading != null && !isNaN(heading)) {
    currentHeading = heading;
  }
  userMarker.setLatLng([lat, lon]);
  if (!navigationActive) {
    map.setView([lat, lon], map.getZoom(), { animate: true });
  }

  const arrow = document.getElementById('nav-arrow');
  if (arrow) {
    arrow.style.transform = `translate(-50%, -90px) rotate(${currentHeading}deg)`;
  }
}

function clearRouteLayers() {
  if (routeLayer) {
    map.removeLayer(routeLayer);
    routeLayer = null;
  }
  if (routeCasingLayer) {
    map.removeLayer(routeCasingLayer);
    routeCasingLayer = null;
  }
}

function getPanelFitPadding() {
  const panel = document.getElementById('travel-panel');
  const panelOpen = panel?.classList.contains('visible') && window.innerWidth >= 768;
  const panelW = panelOpen ? (panel.offsetWidth || 420) + 32 : 32;
  return {
    paddingTopLeft: [panelW, 72],
    paddingBottomRight: [32, 140],
  };
}

function setNavigationMode(active) {
  navigationActive = active;
  document.body.classList.toggle('nav-active', active);
  if (!map) return;
  const enable = active;
  if (enable) {
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
  setNavigationMode(false);
}

function showDestinationMarker(coords) {
  if (destinationMarker) map.removeLayer(destinationMarker);
  destinationMarker = L.circleMarker([coords.lat, coords.lon], {
    radius: 9,
    fillColor: '#fbbf24',
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
  map.fitBounds(bounds, { ...getPanelFitPadding(), maxZoom: 13 });
}

function normalizeGeometry(geometry) {
  return (geometry || []).map((pt) => {
    if (Array.isArray(pt)) return [pt[0], pt[1]];
    if (pt.lat != null) return [pt.lat, pt.lon];
    return null;
  }).filter(Boolean);
}

export function drawRouteGeometry(geometry, { fit = false, navigate = false, dest = null, dashed = false } = {}) {
  if (!geometry?.length || !map) return false;
  clearRouteLayers();

  const points = normalizeGeometry(geometry);
  if (points.length < 2) return false;

  const lineOpts = {
    lineCap: 'round',
    lineJoin: 'round',
    ...(dashed ? { dashArray: '10 14', opacity: 0.75 } : {}),
  };

  if (!dashed) {
    routeCasingLayer = L.polyline(points, {
      color: '#0c4a6e',
      weight: 10,
      opacity: 0.55,
      ...lineOpts,
    }).addTo(map);
  }

  routeLayer = L.polyline(points, {
    color: dashed ? '#fbbf24' : '#7dd3fc',
    weight: dashed ? 4 : 5,
    opacity: dashed ? 0.85 : 0.95,
    ...lineOpts,
  }).addTo(map);

  if (navigate) {
    setNavigationMode(true);
    fitRouteView(points, dest);
  } else if (fit) {
    fitRouteView(points, dest);
  }
  map.invalidateSize();
  return true;
}

async function fetchOsrmRouteDirect(fromLat, fromLon, toLat, toLon) {
  const url = `https://router.project-osrm.org/route/v1/car/${fromLon},${fromLat};${toLon},${toLat}?overview=full&geometries=geojson`;
  const res = await fetch(url);
  if (!res.ok) return null;
  const data = await res.json();
  if (data.code !== 'Ok' || !data.routes?.[0]?.geometry?.coordinates?.length) return null;
  return data.routes[0].geometry.coordinates.map(([lon, lat]) => [lat, lon]);
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
  const fromLat = currentPos.lat;
  const fromLon = currentPos.lon;

  const { apiFetch } = await import('./api.js');
  try {
    const res = await apiFetch(
      `/api/map/route?from_lat=${fromLat}&from_lon=${fromLon}&to_lat=${toLat}&to_lon=${toLon}&profile=car`
    );
    const geom = res.data?.geometry;
    if (geom?.length > 1) {
      return drawRouteGeometry(geom, { navigate, fit: navigate, dest });
    }
  } catch (e) {
    console.warn('Backend route failed, trying OSRM from browser:', e);
  }

  try {
    const geom = await fetchOsrmRouteDirect(fromLat, fromLon, toLat, toLon);
    if (geom?.length > 1) {
      return drawRouteGeometry(geom, { navigate, fit: navigate, dest });
    }
  } catch (e) {
    console.warn('Browser OSRM failed:', e);
  }

  if (artifactRef) {
    artifactRef._routeError = 'Driving directions could not be loaded';
  }
  return false;
}

/** Show destination on map without drawing a route (panel open). */
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

/**
 * Show route A (GPS) → B (artifact coordinates). No AI — only map math + optional OSRM.
 * Returns { ok, mode: 'driving' | 'saved' | 'direct' }
 */
export async function navigateToDestination(artifact) {
  if (!map) return { ok: false, mode: null };

  map.invalidateSize();
  delete artifact._routeError;
  delete artifact._routeFallback;

  const dest = resolveDestination(artifact);
  if (!dest) return { ok: false, mode: null };

  await refreshGpsPosition();
  showDestinationMarker(dest);

  // Always route from live GPS → destination (never reuse old plan geometry)
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
  const { apiFetch } = await import('./api.js');
  try {
    const res = await apiFetch(`/api/trips/${tripId}/route`);
    clearRouteLayers();
    setNavigationMode(false);
    const points = res.data.map((p) => [p.lat, p.lon]);
    if (points.length > 1) {
      routeLayer = L.polyline(points, { color: '#7dd3fc', weight: 3, opacity: 0.5 }).addTo(map);
    }
  } catch (_) {}
}
