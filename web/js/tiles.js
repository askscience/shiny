/**
 * tiles.js — plugin window surfaces (Hyprland-style desktop).
 *
 * Every plugin that has an interface lives inside its own tile window in
 * #tile-grid. The traveler plugin's interface is the map. The HUD header and
 * the AI sphere/dock are fixed chrome — always visible, never tiled.
 *
 * The layout engine (workspaces, master/stack/windows, focus, fullscreen)
 * lives in desktop.js; this module owns the plugin-window DOM: mounting the
 * map/radio/word/youtube/calc tiles, the per-window title bar (close/fullscreen
 * controls), and the in-tile artifact sheets/overlay.
 *
 * - Settings → Desktop sets the tiling layout; Settings → Plugin windows
 *   still chooses Tile vs Full screen per plugin.
 * - The AI surfaces a window with show_plugin (→ `plugin:focus`) and controls
 *   the desktop with desktop_fullscreen / workspace_* tools (→ `agent:actions`).
 * - Artifact cards are NOT tiles — they use the dock + travel panel as before.
 */
import { isPluginActive, refreshActivePlugins } from './activePlugins.js';
import { getPluginLayout } from './preferences.js';
import {
  initDesktop, ensureWindows, activeWindowNames, activeWorkspaceIndex,
  applyLayout, renderWorkspaceBar,
  focusWindow, toggleFullscreen, clearFullscreen, clearFocus, getFullscreen,
  setSurfaceNamesProvider,
} from './desktop.js';
import { apiFetch } from './api.js';
import { navigateToDestination } from './map.js';
import {
  artifactPanel, icon, installGlow, refreshGlow, createRim, setGlow, glowFromDrawable,
} from '../ui/index.js';
import { getDockSummaries } from './artifactStore.js';

import { PHONE_QUERY, MOBILE_PORTRAIT_QUERY } from './viewport.js';

const MAP_TILE_PLUGIN = 'traveler';

let grid = null;
let overlay = null;
let overlayBody = null;
let overlayTitle = null;

let pluginCatalog = new Map(); // name -> { description }
let surfaceModules = new Map(); // name -> plugin.js surface module
let mapTileEl = null;          // the tile hosting the map DOM
let mapRimEl = null;           // the map's on-top ambient rim glow
let mapGlowTimer = null;       // debounce for the map glow sample
let activePhonePlugin = null;  // the window shown on a phone (one at a time)
let lastWsIndex = null;        // last workspace index seen (for slide direction)

function pluginLabel(name) {
  return name.charAt(0).toUpperCase() + name.slice(1);
}

/** Which plugins currently have a window surface? */
function surfacePlugins() {
  const out = [];
  if (isPluginActive(MAP_TILE_PLUGIN)) out.push(MAP_TILE_PLUGIN);
  for (const name of surfaceModules.keys()) out.push(name);
  return out;
}

/**
 * Refresh the plugin catalog AND dynamically load/unload each enabled
 * plugin's window surface (its `web/plugin.js`, served at /plugins/<name>/).
 */
async function refreshCatalog() {
  // Which plugins are active THIS session (fresh mode → empty). This, not the
  // persisted `enabled` flag from /api/plugins, decides which window surfaces
  // get loaded — so a fresh sign-in never mounts plugin windows.
  const activeSet = await refreshActivePlugins();

  let plugins = [];
  try {
    const res = await apiFetch('/api/plugins');
    plugins = res?.data || [];
    pluginCatalog = new Map(
      plugins.map((p) => [p.name, { description: p.description || p.summary || '' }]),
    );
  } catch (_) { /* keep last catalog */ }

  const wanted = new Set(
    plugins.filter((p) => activeSet.has(p.name) && p.surface).map((p) => p.name),
  );
  let wiredAny = false;
  for (const p of plugins) {
    if (activeSet.has(p.name) && p.surface && !surfaceModules.has(p.name)) {
      try {
        const mod = await import(`/plugins/${p.name}/plugin.js`);
        surfaceModules.set(p.name, mod.default || mod);
        surfaceModules.get(p.name)?.wireEvents?.();
        wiredAny = true;
      } catch (err) {
        console.warn(`plugin surface '${p.name}' failed to load`, err);
      }
    }
  }
  // A surface activated in the same agent response as its tool call would
  // otherwise miss the `agent:actions` dispatch (wired after it fired) —
  // re-deliver the last actions so e.g. a freshly-activated Studio sees the
  // track the AI just created.
  if (wiredAny && window.__lastAgentActions?.length) {
    window.dispatchEvent(new CustomEvent('agent:actions', { detail: window.__lastAgentActions }));
  }
  for (const name of [...surfaceModules.keys()]) {
    if (!wanted.has(name)) {
      surfaceModules.get(name)?.unmount?.();
      surfaceModules.delete(name);
    }
  }
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

  // The map is opaque and full-bleed, so a behind-content glow would never
  // show. The map window uses the on-top rim variant instead; it inherits the
  // Tier 0 colours from the tile and is upgraded with a sampled map tile.
  mapRimEl = createRim(mapTileEl);

  // Saved-cards dock lives INSIDE the traveler window (top-right, over the
  // map) instead of under the AI sphere. artifacts.js renders the buttons
  // into #map-tile-dock-icons and hides the old chrome-bottom dock.
  const tileDock = document.createElement('div');
  tileDock.id = 'map-tile-dock';
  tileDock.className = 'map-tile-dock hidden';
  tileDock.setAttribute('aria-label', 'Saved cards');
  const tileDockIcons = document.createElement('div');
  tileDockIcons.id = 'map-tile-dock-icons';
  tileDock.appendChild(tileDockIcons);
  mapTileEl.appendChild(tileDock);
}

/* Map → ambient rim: sample the centre Carto tile and mirror it, falling back
   to the Tier 0 colour rim whenever the fetch/decode fails (offline, tainted
   canvas, blocked host). */
function lonLatToTile(lon, lat, z) {
  const n = 2 ** z;
  const latRad = (lat * Math.PI) / 180;
  return {
    x: Math.floor(((lon + 180) / 360) * n),
    y: Math.floor((1 - Math.log(Math.tan(latRad) + 1 / Math.cos(latRad)) / Math.PI) / 2 * n),
  };
}

async function sampleMapGlow(view) {
  if (!mapRimEl || !view) return;
  const z = Math.max(3, Math.min(10, Math.round(view.zoom || 8)));
  const { x, y } = lonLatToTile(view.lon, view.lat, z);
  const sub = 'abc'[Math.abs(x + y) % 3];
  const mode = view.mode === 'light' ? 'light' : 'dark';
  const url = `https://${sub}.basemaps.cartocdn.com/${mode}_all/${z}/${x}/${y}.png`;
  const img = new Image();
  img.crossOrigin = 'anonymous';
  await new Promise((resolve) => {
    img.onload = resolve;
    img.onerror = resolve;
    img.src = url;
  });
  const css = glowFromDrawable(img, 96);
  if (css) setGlow(mapRimEl, css);
}

function scheduleMapGlow(view) {
  window.clearTimeout(mapGlowTimer);
  mapGlowTimer = window.setTimeout(() => void sampleMapGlow(view), 250);
}

/** Remove the map tile from the grid (traveler deactivated). */
function unmountMapTile() {
  mapTileEl?.remove();
  mapTileEl = null;
  mapRimEl = null;
}

function resizeMapSoon() {
  window.dispatchEvent(new Event('map:resize'));
  requestAnimationFrame(() => window.dispatchEvent(new Event('map:resize')));
  setTimeout(() => window.dispatchEvent(new Event('map:resize')), 200);
}

/* ── Layout ─────────────────────────────────────────────────── */

/** Get (lazily mounting) the tile element for a plugin's window. */
function elementForTile(name) {
  if (name === MAP_TILE_PLUGIN) {
    // Mount the map tile the first time the traveler window appears (it must
    // exist in the DOM before initMap() looks for #map).
    if (!mapTileEl) mountMapTile();
    return mapTileEl;
  }
  return surfaceModules.get(name)?.mount?.() || null;
}

/** Deactivate a plugin — the close icon on its window. Mirrors the Plugins
 *  page so the desktop, map and surfaces all refresh together. */
export async function deactivatePlugin(name) {
  try {
    await apiFetch('/api/plugins/deactivate', {
      method: 'POST',
      body: JSON.stringify({ name }),
    });
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: err.message || `Could not deactivate ${pluginLabel(name)}`, type: 'error' },
    }));
    return;
  }
  localStorage.setItem('plugins.changed', String(Date.now()));
  window.dispatchEvent(new CustomEvent('plugins:changed'));
}

/** Activate a plugin for this session — mirrors the tray / Plugins page. The
 *  `opened` hint makes the desktop focus (raise) the new window. */
export async function activatePlugin(name) {
  try {
    await apiFetch('/api/plugins/activate', {
      method: 'POST',
      body: JSON.stringify({ name }),
    });
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: err.message || `Could not activate ${pluginLabel(name)}`, type: 'error' },
    }));
    return;
  }
  localStorage.setItem('plugins.changed', String(Date.now()));
  window.dispatchEvent(new CustomEvent('plugins:changed', { detail: { opened: name } }));
}

/** The loaded window-surface module for a plugin (or undefined). Used by the
 *  context menu to collect the app's own `contextMenu()` entries. */
export function getPluginSurface(name) {
  return surfaceModules.get(name);
}

/** The mounted window element for a plugin (the map counts as traveler's), or
 *  null when the plugin has no window / is not mounted yet. */
export function getPluginTile(name) {
  if (name === MAP_TILE_PLUGIN) return mapTileEl;
  return surfaceModules.get(name)?.getElement?.() || null;
}

/** Add the shared window chrome (title bar + close/fullscreen controls) to a
 *  plugin window, once. The controls sit on the left as requested; the title
 *  is centered and the header doubles as the drag handle in Windows layout. */
function ensureWindowChrome(el, name) {
  if (!el) return;

  // Ambient glow — Tier 0 colour layer, universal for every window (including
  // the map). Idempotent, so it also survives re-tiling. The map keeps the
  // behind-content glow (it shows in the window chrome above the opaque map).
  installGlow(el, name);

  if (el.querySelector(':scope > .tile-header')) return;

  const header = document.createElement('header');
  header.className = 'tile-header';

  const controls = document.createElement('span');
  controls.className = 'tile-header-controls';

  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'tile-header-btn tile-header-btn--close';
  closeBtn.title = 'Deactivate plugin';
  closeBtn.setAttribute('aria-label', `Deactivate ${pluginLabel(name)}`);
  closeBtn.appendChild(icon('ui/close', { size: 14 }));
  closeBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    void deactivatePlugin(name);
  });

  const fullBtn = document.createElement('button');
  fullBtn.type = 'button';
  fullBtn.className = 'tile-header-btn tile-header-btn--full';
  fullBtn.title = 'Full screen';
  fullBtn.setAttribute('aria-label', `Full screen ${pluginLabel(name)}`);
  fullBtn.appendChild(icon('ui/expand', { size: 14 }));
  fullBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    toggleFullscreen(name);
  });

  controls.append(closeBtn, fullBtn);

  const title = document.createElement('span');
  title.className = 'tile-header-title';
  title.textContent = pluginLabel(name);

  const spacer = document.createElement('span');
  spacer.className = 'tile-header-spacer';
  spacer.setAttribute('aria-hidden', 'true');

  header.append(controls, title, spacer);
  el.prepend(header);

  // Bottom-right resize grip (visible in Windows layout mode only).
  const resize = document.createElement('span');
  resize.className = 'tile-resize';
  resize.title = 'Resize';
  resize.setAttribute('aria-hidden', 'true');
  resize.appendChild(icon('ui/grip', { size: 14 }));
  el.appendChild(resize);
}

/** Reflect the current fullscreen state on a window's fullscreen button. */
function syncWindowChrome(el, name) {
  const full = el?.querySelector(':scope > .tile-header .tile-header-btn--full');
  if (!full) return;
  const isFull = getFullscreen() === name;
  full.classList.toggle('is-active', isFull);
  full.title = isFull ? 'Exit full screen' : 'Full screen';
  full.setAttribute('aria-label', isFull
    ? `Exit full screen ${pluginLabel(name)}`
    : `Full screen ${pluginLabel(name)}`);
}

function renderTiles() {
  if (!grid) return;
  const names = surfacePlugins();
  ensureWindows(names);

  const phone = PHONE_QUERY.matches && names.length > 0;
  const visible = activeWindowNames(names);

  grid.classList.toggle('tile-grid--phone', phone);
  grid.classList.toggle('tile-grid--single', !phone && visible.length === 1);

  // Build the window list. NOTE: do not wipe and re-append these elements —
  // detaching and re-attaching a tile reloads any <iframe> inside it, which
  // restarts the YouTube player. The DOM is reconciled in place below.
  const ordered = [];
  for (const name of names) {
    const el = elementForTile(name);
    if (!el) continue;
    ensureWindowChrome(el, name);
    syncWindowChrome(el, name);
    ordered.push({ name, el });
  }

  const want = ordered.map((o) => o.el);
  const have = [...grid.children].filter((c) => c.classList.contains('tile'));
  const sameOrder = have.length === want.length && want.every((el, i) => have[i] === el);
  if (!sameOrder) {
    for (const el of have) if (!want.includes(el)) el.remove();
    want.forEach((el, i) => {
      const at = grid.children[i] || null;
      if (at !== el) grid.insertBefore(el, at);
    });
  }

  const items = [];
  if (phone) {
    // Navigation always lives on the map — surface it if a route is live.
    if (document.body.classList.contains('navigator-active')) activePhonePlugin = MAP_TILE_PLUGIN;
    if (!visible.includes(activePhonePlugin)) activePhonePlugin = visible[0] || null;

    // Keep every window mounted (hidden, not detached) so their chrome — the
    // traveler dock, tile sheets — stays inside its own window.
    for (const { name, el } of ordered) {
      const inActiveWs = visible.includes(name);
      const shown = inActiveWs && name === activePhonePlugin;
      el.classList.remove('tile--full', 'tile--master', 'tile--stack', 'tile--window');
      el.style.gridColumn = '';
      el.style.gridRow = '';
      el.style.left = '';
      el.style.top = '';
      el.style.width = '';
      el.style.height = '';
      el.style.zIndex = '';
      el.classList.toggle('hidden', !shown);
    }
  } else {
    for (const { name, el } of ordered) {
      const inActiveWs = visible.includes(name);
      el.classList.toggle('hidden', !inActiveWs);
      if (inActiveWs) items.push({ name, el });
    }
    applyLayout(grid, items);
  }

  grid.classList.toggle('hidden', names.length === 0);
  document.body.classList.toggle('tiles-active', names.length > 0);
  renderWorkspaceBar();
  // The dock moved into the traveler window — re-render it whenever tiles
  // change so it follows the tile (activation toggles, layout switches).
  window.dispatchEvent(new CustomEvent('artifact:dock', { detail: getDockSummaries() }));
  resizeMapSoon();
}

/** Slide the visible windows into place after a workspace switch, coming from
 *  the side the switch moved away from (next → from the right, prev → from the
 *  left). Uses the Web Animations API so it never fights the focus pulse. */
function animateWorkspaceSlide(dir) {
  if (!grid || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const tiles = [...grid.querySelectorAll('.tile')].filter((t) => !t.classList.contains('hidden'));
  if (!tiles.length) return;
  const offset = (dir >= 0 ? 1 : -1) * 64;
  tiles.forEach((t, i) => {
    t.animate(
      [
        { transform: `translateX(${offset}px)`, opacity: 0.25 },
        { transform: 'translateX(0px)', opacity: 1 },
      ],
      { duration: 340, easing: 'cubic-bezier(0.16, 1, 0.3, 1)', delay: Math.min(i * 24, 160) },
    );
  });
}

/* ── Full-screen focus ─────────────────────────────────────── */

function focusPlugin(name) {
  if (document.body.classList.contains('navigator-active')) return;
  if (!surfacePlugins().includes(name)) return;

  // Phone: one window at a time — switching is the whole gesture.
  if (PHONE_QUERY.matches) {
    activePhonePlugin = name;
    renderTiles();
    return;
  }

  focusWindow(name);

  // The per-plugin "Full screen" preference still means "open fullscreen".
  // Otherwise clear any fullscreen so we return to the tiled layout.
  if (getPluginLayout(name) === 'full') {
    toggleFullscreen(name, true);
  } else if (getFullscreen()) {
    clearFullscreen();
  }
}

function unfocus() {
  clearFullscreen();
  clearFocus();
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
  focusWindow(pluginName);
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
  return surfaceModules.get(name)?.getElement?.()
    || grid?.querySelector(`[data-plugin="${CSS.escape(name)}"]`)
    || null;
}

/**
 * Open an artifact card INSIDE its plugin's tile window (absolute sheet over
 * the plugin's own UI). Falls back to the generic overlay when the window
 * isn't visible (e.g. plugin deactivated mid-conversation).
 */
export async function openArtifactInTile(pluginName, artifact) {
  // Phone: bring the plugin's window forward so its sheet is visible.
  if (PHONE_QUERY.matches
    && surfacePlugins().includes(pluginName)
    && activePhonePlugin !== pluginName) {
    activePhonePlugin = pluginName;
    renderTiles();
  }
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

  initDesktop();
  setSurfaceNamesProvider(() => surfacePlugins());

  mountMapTile();
  void refreshCatalog().then(renderTiles);
  renderTiles();

  // Desktop state changes (workspace/focus/fullscreen) re-tile the windows.
  window.addEventListener('desktop:changed', (e) => {
    const idx = activeWorkspaceIndex();
    renderTiles();
    // A workspace switch slides the incoming windows into place (desktop.js
    // hints the true direction for wrap-around; otherwise infer from the
    // index delta). Focus/fullscreen changes don't animate.
    if (lastWsIndex !== null && idx !== lastWsIndex) {
      const hinted = Number(e.detail?.slide);
      const dir = (Number.isFinite(hinted) && hinted !== 0) ? hinted : (idx > lastWsIndex ? 1 : -1);
      animateWorkspaceSlide(dir);
    }
    lastWsIndex = idx;
  });

  // Crossing the phone breakpoint re-tiles the windows.
  PHONE_QUERY.addEventListener('change', () => renderTiles());
  // So does turning the screen (which switches the workspace system off/on).
  MOBILE_PORTRAIT_QUERY.addEventListener('change', () => renderTiles());

  // The map broadcasts its view so the traveler window's rim can mirror it.
  window.addEventListener('map:view', (e) => scheduleMapGlow(e.detail));

  // Tier 0 glow colours are seeded from the accent + a per-window partner hue;
  // repaint them when the theme or accent changes.
  for (const evt of ['theme:change', 'appearance:change']) {
    window.addEventListener(evt, () => {
      grid?.querySelectorAll(':scope > .tile').forEach((el) => {
        refreshGlow(el, el.dataset.plugin || '');
      });
    });
  }

  // Starting a route on a phone must surface the map window.
  const bodyObserver = new MutationObserver(() => {
    if (PHONE_QUERY.matches
      && document.body.classList.contains('navigator-active')
      && activePhonePlugin !== MAP_TILE_PLUGIN) {
      activePhonePlugin = MAP_TILE_PLUGIN;
      renderTiles();
    }
  });
  bodyObserver.observe(document.body, { attributes: true, attributeFilter: ['class'] });

  window.addEventListener('plugin:focus', (e) => {
    const name = e.detail?.name;
    if (name) focusPlugin(name);
  });
  window.addEventListener('plugins:changed', async (e) => {
    await refreshActivePlugins();
    await refreshCatalog();
    if (!isPluginActive(MAP_TILE_PLUGIN)) unmountMapTile();
    if (!surfacePlugins().includes(activePhonePlugin)) activePhonePlugin = null;
    renderTiles();
    // A plugin just opened from the tray — focus it so its window comes to
    // the front (raises it in the floating Windows layout).
    const opened = e.detail?.opened;
    if (opened) focusPlugin(opened);
  });
}

/** Re-evaluate which plugin windows exist (called after activation checks). */
export function refreshTiles() {
  renderTiles();
}

export { mountMapTile, unmountMapTile };
