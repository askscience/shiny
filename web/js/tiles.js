/**
 * tiles.js — plugin window manager (Android Auto-style tiling).
 *
 * Every plugin that has an interface lives inside its own tile window in
 * #tile-grid. The traveler plugin's interface is the map. The HUD header and
 * the AI sphere/dock are fixed chrome — always visible, never tiled.
 *
 * - Settings → Plugin windows chooses Tile (grid) or Full screen (focused
 *   takeover between HUD and chrome-bottom) per plugin.
 * - The AI surfaces a window with the show_plugin tool (→ `plugin:focus`).
 * - Artifact cards are NOT tiles — they use the dock + travel panel as before.
 */
import { isPluginActive, refreshActivePlugins } from './activePlugins.js';
import { getPluginLayout } from './preferences.js';
import { apiFetch } from './api.js';
import { navigateToDestination } from './map.js';
import { artifactPanel } from '../ui/index.js';

const MAP_TILE_PLUGIN = 'traveler';

let grid = null;
let overlay = null;
let overlayBody = null;
let overlayTitle = null;

let pluginCatalog = new Map(); // name -> { description }
let focusedPlugin = null;      // plugin currently in full screen
let mapTileEl = null;          // the tile hosting the map DOM

function pluginLabel(name) {
  return name.charAt(0).toUpperCase() + name.slice(1);
}

function pluginIconName(name) {
  if (name === 'traveler') return 'artifacts/plan';
  return 'ui/puzzle';
}

/** Which plugins currently have a window surface? */
function surfacePlugins() {
  const out = [];
  if (isPluginActive(MAP_TILE_PLUGIN)) out.push(MAP_TILE_PLUGIN);
  return out;
}

async function refreshCatalog() {
  try {
    const res = await apiFetch('/api/plugins');
    pluginCatalog = new Map(
      (res?.data || []).map((p) => [p.name, { description: p.description || p.summary || '' }]),
    );
  } catch (_) { /* keep last catalog */ }
}

/* ── Map tile (traveler plugin window) ─────────────────────── */

/** Detach the map DOM into its tile, keeping a placeholder to restore it. */
function mountMapTile() {
  if (mapTileEl) return;

  const stage = document.createElement('div');
  stage.id = 'map-stage';
  const mapDiv = document.createElement('div');
  mapDiv.id = 'map';
  stage.appendChild(mapDiv);

  mapTileEl = document.createElement('section');
  mapTileEl.className = 'tile tile--map';
  mapTileEl.dataset.plugin = MAP_TILE_PLUGIN;
  mapTileEl.appendChild(stage);
}

/** Remove the map tile from the grid (traveler deactivated). */
function unmountMapTile() {
  mapTileEl?.remove();
}

function resizeMapSoon() {
  window.dispatchEvent(new Event('map:resize'));
  requestAnimationFrame(() => window.dispatchEvent(new Event('map:resize')));
  setTimeout(() => window.dispatchEvent(new Event('map:resize')), 200);
}

/* ── Layout ─────────────────────────────────────────────────── */

function renderTiles() {
  if (!grid) return;
  const names = surfacePlugins();

  // Mount the map tile the first time the traveler window appears (it must
  // exist in the DOM before initMap() looks for #map).
  if (names.includes(MAP_TILE_PLUGIN) && !mapTileEl) mountMapTile();

  // Full-screen plugin takes over the grid area exclusively.
  if (focusedPlugin && !names.includes(focusedPlugin)) focusedPlugin = null;
  const focused = focusedPlugin;

  grid.innerHTML = '';
  for (const name of names) {
    if (name === MAP_TILE_PLUGIN && mapTileEl) {
      // Full when focused, when the user picked Full screen, or when it is
      // the ONLY plugin window on screen — the rounded frame stays either way.
      const isFull = focused === name
        || names.length === 1
        || (!focused && getPluginLayout(name) === 'full');
      mapTileEl.classList.toggle('tile--full', isFull);
      grid.appendChild(mapTileEl);
      continue;
    }
    // Plugins without a surface yet don't get a window.
  }
  grid.classList.toggle('hidden', names.length === 0);
  grid.classList.toggle('tile-grid--single', names.length === 1);
  document.body.classList.toggle('tiles-active', names.length > 0);
  resizeMapSoon();
}

/* ── Full-screen focus ─────────────────────────────────────── */

function focusPlugin(name) {
  if (document.body.classList.contains('navigator-active')) return;
  if (!surfacePlugins().includes(name)) return;

  if (getPluginLayout(name) === 'full' || focusedPlugin) {
    focusedPlugin = name;
    renderTiles();
    return;
  }
  // Tile mode: pulse the window so the user sees where the plugin lives.
  const tile = grid?.querySelector(`[data-plugin="${CSS.escape(name)}"]`);
  if (tile) {
    tile.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    tile.classList.add('tile--focused');
    setTimeout(() => tile.classList.remove('tile--focused'), 1800);
  }
}

function unfocus() {
  focusedPlugin = null;
  renderTiles();
}

/* ── Full-screen overlay (generic plugin content) ──────────── */
/* Kept for plugins whose window content is artifact-like detail. The map tile
 * uses grid focus instead. */

async function onTileNavigate(artifact) {
  const result = await navigateToDestination(artifact);
  if (result?.ok) {
    const msg = result.mode === 'direct'
      ? 'Straight line from your location — full driving route could not be loaded'
      : 'Driving route from your location — pinch or drag the map to explore';
    window.dispatchEvent(new CustomEvent('app:toast', { detail: { message: msg, type: 'info' } }));
  } else if (artifact.coordinates || artifact.actions?.some((a) => a.tool === 'map_route')) {
    const msg = artifact._routeError || 'Could not load driving route — try again in a moment';
    window.dispatchEvent(new CustomEvent('app:toast', { detail: { message: msg, type: 'error' } }));
  } else {
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: 'No destination coordinates for this plan', type: 'error' },
    }));
  }
}

function onTileAction(action, artifact) {
  if (action?.tool === 'map_route') {
    void onTileNavigate(artifact);
    return;
  }
  window.dispatchEvent(new CustomEvent('artifact:action', { detail: { action, artifact } }));
}

/** Open an artifact card in the overlay (used when a plugin surface wants a
 * large card view). The default artifact flow stays on the travel panel. */
export async function openArtifactOverlay(pluginName, artifactId) {
  if (!overlay) return;
  const { getArtifact } = await import('./artifactStore.js');
  focusedPlugin = pluginName;
  overlayTitle.textContent = pluginLabel(pluginName);
  overlayBody.innerHTML = '';
  try {
    const artifact = await getArtifact(artifactId);
    overlayBody.appendChild(artifactPanel(artifact, {
      onClose: closeOverlay,
      onNavigate: (a) => { closeOverlay(); void onTileNavigate(a); },
      onAction: onTileAction,
    }));
  } catch (_) {
    const empty = document.createElement('p');
    empty.className = 'tile-empty';
    empty.textContent = 'Card unavailable.';
    overlayBody.appendChild(empty);
  }
  overlay.classList.remove('hidden');
  requestAnimationFrame(() => overlay.classList.add('visible'));
}

function closeOverlay() {
  if (!overlay) return;
  overlay.classList.remove('visible');
  setTimeout(() => overlay.classList.add('hidden'), 250);
}

/* ── In-tile artifact sheet (plugin output stays inside its window) ── */

function tileForPlugin(name) {
  if (name === MAP_TILE_PLUGIN) return mapTileEl;
  return grid?.querySelector(`[data-plugin="${CSS.escape(name)}"]`) || null;
}

/**
 * Open an artifact card INSIDE its plugin's tile window (absolute sheet over
 * the plugin's own UI). Falls back to the generic overlay when the window
 * isn't visible (e.g. plugin deactivated mid-conversation).
 */
export async function openArtifactInTile(pluginName, artifact) {
  const tile = tileForPlugin(pluginName);
  if (!tile || !tile.isConnected) {
    await openArtifactOverlay(pluginName, artifact.id);
    return;
  }
  tile.querySelector('.tile-sheet')?.remove();
  const sheet = document.createElement('div');
  sheet.className = 'tile-sheet';
  sheet.appendChild(artifactPanel(artifact, {
    onClose: () => sheet.remove(),
    onNavigate: (a) => { sheet.remove(); void onTileNavigate(a); },
    onAction: onTileAction,
  }));
  tile.appendChild(sheet);
}

/* ── Public API ─────────────────────────────────────────────── */

export function initTileManager() {
  if (grid) return; // idempotent — applyTravelerActivation re-calls this
  grid = document.getElementById('tile-grid');
  overlay = document.getElementById('tile-overlay');
  overlayBody = document.getElementById('tile-overlay-body');
  overlayTitle = document.getElementById('tile-overlay-title');
  document.getElementById('tile-overlay-back')?.addEventListener('click', () => {
    closeOverlay();
    unfocus();
  });

  mountMapTile();
  void refreshCatalog().then(renderTiles);
  renderTiles();

  window.addEventListener('plugin:focus', (e) => {
    const name = e.detail?.name;
    if (name) focusPlugin(name);
  });
  window.addEventListener('plugins:changed', async () => {
    await refreshActivePlugins();
    await refreshCatalog();
    if (focusedPlugin && !surfacePlugins().includes(focusedPlugin)) focusedPlugin = null;
    if (!isPluginActive(MAP_TILE_PLUGIN)) unmountMapTile();
    renderTiles();
  });
}

/** Re-evaluate which plugin windows exist (called after activation checks). */
export function refreshTiles() {
  renderTiles();
}

/** The map tile element (or null when the traveler window is not shown). */
export function getMapTileElement() {
  return isPluginActive(MAP_TILE_PLUGIN) ? mapTileEl : null;
}

export { mountMapTile, unmountMapTile };
