import { controlButton } from '../ui/components/button.js';
/**
 * tiles.js — plugin window surfaces (Hyprland-style desktop).
 *
 * Every plugin that has an interface lives inside its own tile window in
 * #tile-grid. The traveler plugin's interface is the map. The HUD header and
 * the voice bar/dock are fixed chrome — always visible, never tiled.
 *
 * The layout engine (workspaces, master/stack/windows, focus, fullscreen)
 * lives in desktop.js; this module owns the plugin-window DOM: mounting the
 * map/radio/word/youtube/calc tiles, the per-window title bar (close/fullscreen/
 * focus controls), and the in-tile artifact sheets/overlay.
 *
 * - Settings → Desktop sets the tiling layout; Settings → Plugin windows
 *   still chooses Tile vs Full screen per plugin.
 * - The AI surfaces a window with show_plugin (→ `plugin:focus`) and controls
 *   the desktop with desktop_fullscreen / workspace_* tools (→ `agent:actions`).
 * - Artifact cards are NOT tiles — they use the dock + travel panel as before.
 */
import { isPluginActive, refreshActivePlugins, refreshPluginCatalog } from './activePlugins.js';
import { CORE_WINDOWS, isCoreWindow, coreWindowTitle } from './coreWindows.js';
import { getPluginLayout } from './preferences.js';
import {
  initDesktop, ensureWindows, activeWindowNames, activeWorkspaceIndex,
  applyLayout, renderWorkspaceBar, getLayout,
  focusWindow, clearFullscreen, clearFocus, getFullscreen,
  setSurfaceNamesProvider, isFocusOnlyChange, bumpZ, getWorkspacesList,
  isPluginFocused, togglePluginFocus, reconcilePluginFocus, revealWindow,
  closeWindow as closeDesktopWindow,
} from './desktop.js';
import {
  toggleWindowFullscreen, enterWindowFullscreen, exitWindowFullscreen,
  setFullscreenSurfaceProvider, fullscreenWindow,
} from './fullscreen.js';
import { apiFetch } from './api.js';
import { navigateToDestination } from './map.js';
import {
  artifactPanel, icon, installGlow, refreshGlow, createRim, setGlow, glowFromDrawable,
  windowEnter, windowExit, layoutShift,
} from '../ui/index.js';
import { getDockSummaries } from './artifactStore.js';
import { loadPluginCatalog } from './i18n.js';
import { webAppSurface, forgetWebAppSurface } from './webAppSurface.js';

import { PHONE_QUERY, MOBILE_PORTRAIT_QUERY } from './viewport.js';
import { createWorkspaceSlide } from './desktop/workspaceSlide.js';

const MAP_TILE_PLUGIN = 'traveler';

/* Built-in windows (settings, plugins) live in the shared registry so the
 * tray, workspace bar and context menu can recognise them too. */
const coreOpen = new Set();          // built-in windows the user has open
const coreLoading = new Map();       // name -> in-flight import promise
/** Plugin windows open in THIS session. A plugin being enabled is not a
 *  window: this set (seeded from the saved workspaces when "Remember my
 *  workspace" is on, empty in a fresh session) is what puts windows on the
 *  desktop. Closing a window removes it; deactivating a plugin does too. */
const openWindows = new Set();
const webApps = new Set();           // open windows whose surface is an iframe
let catalogReady = false;            // first active-set + surface pass finished

let grid = null;
let overlay = null;
let overlayBody = null;
let overlayTitle = null;

let pluginCatalog = new Map(); // name -> { description, surface, web_app, version }
let surfaceModules = new Map(); // name -> plugin.js surface module
const surfaceUrls = new Map(); // name -> the module's URL (for catalog reloads)
const surfaceVersions = new Map(); // name -> manifest version the module was loaded for
const surfaceLoads = new Map(); // name -> in-flight surface import

// Switching the interface language reloads every mounted plugin's catalog, so
// the next render of its window is already translated (open windows pick it up
// when they are rebuilt).
window.addEventListener('locale:change', () => {
  for (const url of surfaceUrls.values()) void loadPluginCatalog(url);
});
let mapTileEl = null;          // the tile hosting the map DOM
let mapRimEl = null;           // the map's on-top ambient rim glow
let mapGlowTimer = null;       // debounce for the map glow sample
let activePhonePlugin = null;  // the window shown on a phone (one at a time)
let lastWsIndex = null;        // last workspace index seen (for slide direction)
let slide = null;              // the workspace carousel (desktop/workspaceSlide.js)

function pluginLabel(name) {
  return isCoreWindow(name) ? coreWindowTitle(name) : name.charAt(0).toUpperCase() + name.slice(1);
}

/** Which windows belong on the desktop right now: the open set, filtered to
 *  the plugins that are active and have a surface. An enabled plugin with no
 *  window is not on the desktop. */
function surfacePlugins() {
  const out = [];
  if (openWindows.has(MAP_TILE_PLUGIN) && isPluginActive(MAP_TILE_PLUGIN)) {
    out.push(MAP_TILE_PLUGIN);
  }
  // Built-in windows are listed even while their module is still importing, so
  // a restored window is not pruned from its saved workspace before it mounts.
  for (const name of coreOpen) out.push(name);
  for (const name of openWindows) {
    if (name === MAP_TILE_PLUGIN || isCoreWindow(name)) continue;
    if (isPluginActive(name) && surfaceModules.has(name)) out.push(name);
  }
  return out;
}

/** Whether a window surface that `pluginCatalog` says is a web app exists,
 *  recreating it after a close (closing destroys the iframe on purpose). */
function ensureWebAppSurface(name) {
  if (surfaceModules.has(name)) return true;
  if (!pluginCatalog.get(name)?.web_app) return false;
  surfaceModules.set(name, webAppSurface(name));
  webApps.add(name);
  return true;
}

/**
 * Load one plugin's window surface on demand. A plugin being active is not
 * enough: a surface is imported when its window opens (or a saved window is
 * restored), so a session with fifteen active plugins no longer evaluates
 * fifteen modules at boot.
 */
async function ensureSurface(name) {
  if (name === MAP_TILE_PLUGIN) return true;
  const meta = pluginCatalog.get(name);
  if (!meta || (!meta.surface && !meta.web_app)) {
    // The plugin went away while its surface was loaded: drop the stale module.
    surfaceModules.get(name)?.unmount?.();
    surfaceModules.delete(name);
    surfaceVersions.delete(name);
    return false;
  }
  if (surfaceModules.has(name)) {
    // Web apps keep their iframe until the window is closed on purpose.
    if (meta.web_app || surfaceVersions.get(name) === meta.version) return true;
    // A newer manifest is installed: drop the old module so the import below
    // fetches the new one instead of reusing the ES module cache.
    surfaceModules.get(name)?.unmount?.();
    surfaceModules.delete(name);
    surfaceVersions.delete(name);
  }
  if (surfaceLoads.has(name)) return surfaceLoads.get(name);

  const load = (async () => {
    if (meta.web_app) return ensureWebAppSurface(name);
    try {
      // Relative on purpose: a module specifier resolves against the module's
      // URL, and the app can be served through the filtering proxy (the
      // kiosk), where the module URL lives under `/p/…`.
      const moduleUrl = new URL(`../plugins/${name}/plugin.js`, import.meta.url);
      // The version is part of the URL: a plugin update produces a distinct
      // module specifier, so reopening its window loads the new code even
      // though the page itself was never reloaded.
      if (meta.version) moduleUrl.searchParams.set('v', meta.version);
      await loadPluginCatalog(moduleUrl);
      const mod = await import(moduleUrl.href);
      surfaceModules.set(name, mod.default || mod);
      surfaceVersions.set(name, meta.version);
      surfaceUrls.set(name, moduleUrl);
      surfaceModules.get(name)?.wireEvents?.();
      return true;
    } catch (err) {
      console.warn(`plugin surface '${name}' failed to load`, err);
      return false;
    }
  })();
  surfaceLoads.set(name, load);
  try {
    return await load;
  } finally {
    surfaceLoads.delete(name);
  }
}

/** Load (once) the module backing a built-in window. */
function ensureCoreSurface(name) {
  if (surfaceModules.has(name)) return Promise.resolve();
  if (!coreLoading.has(name)) {
    const def = CORE_WINDOWS[name];
    const p = def.load()
      .then((mod) => {
        // A close that raced ahead of the import must not leave a stray mount.
        if (!coreOpen.has(name)) return;
        surfaceModules.set(name, mod.default || mod);
        surfaceModules.get(name)?.wireEvents?.();
      })
      .catch((err) => {
        console.warn(`core window '${name}' failed to load`, err);
      })
      .finally(() => coreLoading.delete(name));
    coreLoading.set(name, p);
  }
  return coreLoading.get(name);
}

/** Open (or bring forward) a built-in window. Opening one reveals it like any
 *  other window; an explicit gesture on an already-open one brings it forward. */
export function openCoreWindow(name) {
  if (!isCoreWindow(name)) return;
  const alreadyOpen = coreOpen.has(name);
  coreOpen.add(name);
  void ensureCoreSurface(name).then(() => {
    renderTiles();
    if (alreadyOpen) focusWindow(name);
  });
}

/**
 * Open (or bring forward) a built-in window and ask it to show one of its
 * sections. Used by the battery quick menu's "Power settings" shortcut; a
 * surface that has no sections ignores the extra hint.
 */
export async function openCoreWindowSection(name, section) {
  if (!isCoreWindow(name)) return;
  const alreadyOpen = coreOpen.has(name);
  coreOpen.add(name);
  await ensureCoreSurface(name);
  renderTiles();
  if (alreadyOpen) focusWindow(name);
  surfaceModules.get(name)?.selectSection?.(section);
}

/** Close a built-in window — its own gesture, not a plugin deactivation. */
export function closeCoreWindow(name) {
  if (!isCoreWindow(name) || !coreOpen.has(name)) return;
  animateClose(name, () => finishCloseCoreWindow(name));
}

function finishCloseCoreWindow(name) {
  // A fullscreen window that is closed must hand back its workspace and the
  // browser fullscreen. Plugin windows do this through `plugins:changed`;
  // built-in windows have no such signal, so do it here.
  if (fullscreenWindow() === name) exitWindowFullscreen();
  coreOpen.delete(name);
  surfaceModules.get(name)?.unmount?.();
  surfaceModules.delete(name);
  mountedNames.delete(name);
  renderTiles();
}

/**
 * Refresh the plugin catalog AND dynamically load/unload each enabled
 * plugin's window surface (its `web/plugin.js`, served at /plugins/<name>/).
 */
async function refreshCatalog(force = false) {
  // Which plugins are active THIS session (fresh mode → empty). This, not the
  // persisted `enabled` flag from /api/plugins, decides which window surfaces
  // get loaded — so a fresh sign-in never mounts plugin windows.
  const activeSet = await refreshActivePlugins({ force });
  const plugins = await refreshPluginCatalog({ force });
  const previousCatalog = pluginCatalog;
  pluginCatalog = new Map(
    plugins.map((p) => [p.name, {
      description: p.description || p.summary || '',
      surface: !!p.surface,
      web_app: !!p.web_app,
      version: p.version || '',
    }]),
  );

  const wanted = new Set(
    plugins
      .filter((p) => activeSet.has(p.name) && (p.surface || p.web_app))
      .map((p) => p.name),
  );
  let wiredAny = false;
  for (const p of plugins) {
    if (!activeSet.has(p.name) || surfaceModules.has(p.name)) continue;
    // Only windows that exist (open now or restored from a saved workspace)
    // need a surface. Active-but-closed plugins stay as catalog metadata.
    if (!openWindows.has(p.name)) continue;
    if (!p.surface && !p.web_app) continue;
    if (await ensureSurface(p.name)) wiredAny = true;
  }
  // A surface activated in the same agent response as its tool call would
  // otherwise miss the `agent:actions` dispatch (wired after it fired) —
  // re-deliver the last actions so e.g. a freshly-activated Studio sees the
  // track the AI just created.
  if (wiredAny && window.__lastAgentActions?.length) {
    window.dispatchEvent(new CustomEvent('agent:actions', { detail: window.__lastAgentActions }));
  }
  for (const name of [...surfaceModules.keys()]) {
    if (isCoreWindow(name)) continue;
    if (!wanted.has(name)) {
      surfaceModules.get(name)?.unmount?.();
      // A web app's unmount is a no-op: its iframe and control stream live in
      // webAppSurface's cache, so dropping the surface without forgetting it
      // would leave the app running off screen (and a later activation would
      // resurrect the stale frame).
      if (webApps.has(name)) forgetWebAppSurface(name);
      surfaceModules.delete(name);
      surfaceVersions.delete(name);
      webApps.delete(name);
    }
  }
  // Native surfaces follow their manifest version. A window that is not on
  // screen (another workspace, or hidden) is rebuilt in place right away; a
  // visible window keeps running and loads the new module when it is closed
  // and reopened, so nothing the user is looking at disappears underneath.
  for (const [name, meta] of pluginCatalog) {
    const before = previousCatalog.get(name);
    if (!before || !meta.surface || meta.web_app) continue;
    if (before.version === meta.version) continue;
    if (!openWindows.has(name) || !surfaceModules.has(name)) continue;
    const el = tileForPlugin(name);
    if (el && !el.classList.contains('hidden')) continue;
    if (surfaceBusy(name)) continue;
    surfaceModules.get(name)?.unmount?.();
    surfaceModules.delete(name);
    surfaceVersions.delete(name);
    await ensureSurface(name);
  }
  // A window only lives while its plugin is active and has a surface. A
  // deactivation (or an uninstalled plugin) takes its saved window with it.
  for (const name of [...openWindows]) {
    if (name === MAP_TILE_PLUGIN) {
      if (!isPluginActive(name)) openWindows.delete(name);
    } else if (!isCoreWindow(name)
      && !surfaceModules.has(name)
      && !surfaceLoads.has(name)) {
      openWindows.delete(name);
    }
  }
  // Built-in windows behave like plugin surfaces once open.
  await Promise.all([...coreOpen].map((name) => ensureCoreSurface(name)));
  catalogReady = true;
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
  // map) instead of above the voice bar. artifacts.js renders the buttons
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

/** Remove the map tile from the grid (traveler deactivated or uninstalled). */
function unmountMapTile() {
  openWindows.delete(MAP_TILE_PLUGIN);
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

/** Deactivate a plugin (Plugins page, context/tray menus, AI). This is not the
 *  window's close button: it turns the plugin off for the session and its
 *  window goes with it. Mirrors the Plugins page so the desktop, map and
 *  surfaces all refresh together. */
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

/** Put a plugin's window on the desktop, if its plugin is active and has a
 *  surface. Opening a window is not activation: a plugin that was activated
 *  earlier (or is active by default with "remember" on) can be opened and
 *  closed without touching its enabled state. `focus` makes it the active
 *  window; without it the window is raised but the accent stays put. */
export async function openPluginWindow(name, { focus = false } = {}) {
  if (!name) return false;
  if (isCoreWindow(name)) {
    openCoreWindow(name);
    return true;
  }
  if (!isPluginActive(name)) return false;

  if (name !== MAP_TILE_PLUGIN) {
    if (!pluginCatalog.has(name)) await refreshCatalog();
    const meta = pluginCatalog.get(name);
    if (!meta || (!meta.surface && !meta.web_app)) return false;
    openWindows.add(name);
    if (!await ensureSurface(name)) {
      openWindows.delete(name);
      return false;
    }
  } else {
    openWindows.add(name);
  }

  renderTiles();
  if (focus) focusPlugin(name);
  else revealPlugin(name);
  return true;
}

/** Close a plugin's window — a real close, not a deactivation. The plugin stays
 *  enabled, so its tools keep working and the tray can reopen it; the window's
 *  surface is torn down (playback stops, a web app's iframe and control stream
 *  are destroyed) and the server is told to release whatever the window kept
 *  alive (a Craft app's MCP process). */
export function closePluginWindow(name) {
  if (!name) return;
  if (isCoreWindow(name)) {
    closeCoreWindow(name);
    return;
  }
  if (!openWindows.has(name)) return;
  animateClose(name, () => finishClosePluginWindow(name));
}

function finishClosePluginWindow(name) {
  if (!openWindows.has(name)) return;
  openWindows.delete(name);
  if (fullscreenWindow() === name) exitWindowFullscreen();
  if (name !== MAP_TILE_PLUGIN) {
    if (webApps.has(name)) {
      // Drop the frame entirely: a closed app must not keep running off
      // screen. Reopening builds a fresh surface.
      forgetWebAppSurface(name);
      surfaceModules.delete(name);
      webApps.delete(name);
    } else {
      // Native surfaces tear their tile down; the module itself stays cached
      // (ES modules are singletons) unless the manifest changed while the
      // window was open — then the reopen must import the new code.
      surfaceModules.get(name)?.unmount?.();
      if (surfaceVersions.get(name) !== pluginCatalog.get(name)?.version) {
        surfaceModules.delete(name);
        surfaceVersions.delete(name);
      }
    }
  }
  // Take it out of the saved arrangement too (a no-op in fresh mode, where
  // nothing is persisted). That render also re-tiles the remaining windows.
  if (!closeDesktopWindow(name)) renderTiles();
  // Craft apps: stop the MCP engine now and gracefully instead of waiting for
  // the next agent turn. No-op for every other plugin.
  void apiFetch('/api/plugins/window/closed', {
    method: 'POST',
    authRedirect: false,
    body: JSON.stringify({ name }),
  }).catch(() => { /* the next agent turn reconciles the process either way */ });
}

/* ── Window motion ─────────────────────────────────────────── */

/** Windows playing their exit; a second close while one plays is a no-op. */
const closing = new Set();

/** The tray button a window belongs to, as the box it grows out of / falls into. */
function trayBox(name) {
  const btn = document.querySelector(`#hud-plugins [data-plugin="${CSS.escape(name)}"]`);
  const box = btn?.getBoundingClientRect();
  return box && box.width > 0 ? box : null;
}

/**
 * Close with the user's motion language (web/ui/motionStyle.js): the window
 * plays its exit, then `finish` tears it down exactly as before. Nothing to
 * animate — not on screen, the overview owns the tiles, the phone layout —
 * means the close is immediate.
 */
function animateClose(name, finish) {
  if (closing.has(name)) return;
  const el = getPluginTile(name);
  if (!el?.isConnected || el.classList.contains('hidden')
    || document.documentElement.dataset.overview || PHONE_QUERY.matches) {
    finish();
    return;
  }
  closing.add(name);
  void windowExit(el, {
    to: trayBox(name),
    done: () => {
      closing.delete(name);
      finish();
    },
  });
}

/**
 * Windows opened since boot that have not played their entrance yet. A name
 * can appear in a render before its surface has an element (the module is
 * still loading), so the entrance waits for the first render that lays the
 * window out.
 */
const awaitingEntrance = new Set();

/** Play the entrance of windows that were just opened (not the boot restore). */
function animateOpenings(opening, items, names) {
  for (const name of awaitingEntrance) if (!names.includes(name)) awaitingEntrance.delete(name);
  if (!animateOpenWindows || PHONE_QUERY.matches || document.documentElement.dataset.overview) return;
  for (const name of opening) awaitingEntrance.add(name);
  for (const item of items) {
    if (!awaitingEntrance.has(item.name) || item.el.classList.contains('hidden')) continue;
    awaitingEntrance.delete(item.name);
    const others = items.filter((it) => it !== item).map((it) => it.el);
    void windowEnter(item.el, { from: trayBox(item.name), others });
  }
}

/** Boxes of the windows currently on screen, by element (none while still booting). */
function shiftableBoxes(ordered) {
  const boxes = new Map();
  if (!animateOpenWindows || document.documentElement.dataset.overview) return boxes;
  for (const { el } of ordered) {
    if (!el.isConnected || el.classList.contains('hidden')) continue;
    const box = el.getBoundingClientRect();
    if (box.width && box.height) boxes.set(el, box);
  }
  return boxes;
}

/**
 * Re-tiling moves the windows that stay: the one that takes the space of a
 * closed window grows into it, the others slide over (layoutShift). Windows
 * being opened play their own entrance instead.
 */
function animateLayoutShift(items, before, opening) {
  if (!before.size) return;
  for (const { name, el } of items) {
    if (opening.includes(name) || awaitingEntrance.has(name)) continue;
    const from = before.get(el);
    if (from) void layoutShift(el, from);
  }
}

/** Off until the restored desktop has been laid out once: boot does not animate. */
let animateOpenWindows = false;

/** The loaded window-surface module for a plugin (or undefined). Used by the
 *  context menu to collect the app's own `contextMenu()` entries. */
export function getPluginSurface(name) {
  return surfaceModules.get(name);
}

/** The mounted window element for a plugin (the map counts as traveler's), or
 *  null when the plugin has no window / is not mounted yet. An element that is
 *  merely cached after a close does not count: the window is not on the
 *  desktop. */
export function getPluginTile(name) {
  if (isCoreWindow(name)) {
    if (!coreOpen.has(name)) return null;
  } else if (!openWindows.has(name)) {
    return null;
  }
  if (name === MAP_TILE_PLUGIN) return mapTileEl;
  return surfaceModules.get(name)?.getElement?.() || null;
}

/** Add the shared window chrome (title bar + close/fullscreen/focus controls) to
 *  a plugin window, once. The controls sit on the left as requested; the title
 *  is centered and the header doubles as the drag handle in Windows layout. */
function ensureWindowChrome(el, name) {
  if (!el) return;

  // Ambient glow — Tier 0 colour layer, universal for every window (including
  // the map). Idempotent, so it also survives re-tiling. The map keeps the
  // behind-content glow (it shows in the window chrome above the opaque map).
  installGlow(el, name);

  // The marker identifies injected chrome — not the styling class. A plugin
  // bar that happens to reuse `tile-header` must not be able to suppress the
  // close/fullscreen injection.
  if (el.querySelector(':scope > [data-window-chrome]')) return;

  const header = document.createElement('header');
  header.className = 'tile-header';
  header.dataset.windowChrome = '';

  const controls = document.createElement('span');
  controls.className = 'tile-header-controls';

  const core = isCoreWindow(name);
  const closeBtn = controlButton();
  closeBtn.type = 'button';
  closeBtn.className = 'tile-header-btn tile-header-btn--close';
  closeBtn.title = 'Close';
  closeBtn.setAttribute('aria-label', `Close ${pluginLabel(name)}`);
  closeBtn.appendChild(icon('ui/close', { size: 14 }));
  closeBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    if (core) closeCoreWindow(name);
    else closePluginWindow(name);
  });

  const fullBtn = controlButton();
  fullBtn.type = 'button';
  fullBtn.className = 'tile-header-btn tile-header-btn--full';
  fullBtn.title = 'Full screen';
  fullBtn.setAttribute('aria-label', `Full screen ${pluginLabel(name)}`);
  fullBtn.appendChild(icon('ui/expand', { size: 14 }));
  fullBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    // Real fullscreen: its own workspace + the browser Fullscreen API, with
    // the in-app fullscreen as the fallback (fullscreen.js).
    toggleWindowFullscreen(name);
  });

  const focusBtn = controlButton();
  focusBtn.type = 'button';
  focusBtn.className = 'tile-header-btn tile-header-btn--focus';
  focusBtn.title = 'Focus';
  focusBtn.setAttribute('aria-label', `Focus ${pluginLabel(name)}`);
  focusBtn.setAttribute('aria-pressed', 'false');
  focusBtn.appendChild(icon('ui/eye', { size: 14 }));
  focusBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    // Focus mode: the AI's skills narrow to the focused plugins and the window
    // ring, the tray glyph and the workspace dot wear the accent. Click again
    // to unfocus. Never set automatically — see desktop.js.
    togglePluginFocus(name);
  });

  controls.append(closeBtn, fullBtn, focusBtn);

  // A plugin window can nominate its top bar (`data-window-bar`) to merge with
  // the window controls, so the window shows one slim row — close, fullscreen,
  // then the plugin's own tools — instead of a title bar stacked above the
  // plugin toolbar. Windows whose top row is content rather than tools (map,
  // browser, files, an idle YouTube player) keep the centered title.
  const pluginBar = el.querySelector(':scope > [data-window-bar]');
  if (pluginBar) {
    header.classList.add('tile-header--merged');
    header.append(controls, pluginBar);
  } else {
    const title = document.createElement('span');
    title.className = 'tile-header-title';
    title.textContent = pluginLabel(name);

    const spacer = document.createElement('span');
    spacer.className = 'tile-header-spacer';
    spacer.setAttribute('aria-hidden', 'true');

    header.append(controls, title, spacer);
  }

  el.prepend(header);

  // Bottom-right resize grip (visible in Windows layout mode only).
  const resize = document.createElement('span');
  resize.className = 'tile-resize';
  resize.title = 'Resize';
  resize.setAttribute('aria-hidden', 'true');
  resize.appendChild(icon('ui/grip', { size: 14 }));
  el.appendChild(resize);
}

/** Reflect the current fullscreen and focus state on a window's controls. */
function syncWindowChrome(el, name) {
  if (!el) return;
  const full = el.querySelector(':scope > .tile-header .tile-header-btn--full');
  if (full) {
    const isFull = getFullscreen() === name;
    full.classList.toggle('is-active', isFull);
    full.title = isFull ? 'Exit full screen' : 'Full screen';
    full.setAttribute('aria-label', isFull
      ? `Exit full screen ${pluginLabel(name)}`
      : `Full screen ${pluginLabel(name)}`);
  }

  const focused = isPluginFocused(name);
  el.classList.toggle('tile--plugin-focus', focused);
  const focusBtn = el.querySelector(':scope > .tile-header .tile-header-btn--focus');
  if (focusBtn) {
    focusBtn.classList.toggle('is-active', focused);
    focusBtn.setAttribute('aria-pressed', String(focused));
    focusBtn.title = focused ? 'Unfocus' : 'Focus';
    focusBtn.setAttribute('aria-label', `${focused ? 'Unfocus' : 'Focus'} ${pluginLabel(name)}`);
  }
}

/** Re-apply the focus-mode chrome without re-tiling — the desktop hands us the
 *  focus-only changes, which deliberately skip the layout pass. */
function repaintWindowFocus() {
  if (!grid) return;
  grid.querySelectorAll(':scope > .tile').forEach((el) => {
    syncWindowChrome(el, el.dataset.plugin || '');
  });
}

/** Plugin names that already had a window at the previous render. A name that
 *  appears for the first time is a window the user just opened, and must come
 *  up above the existing ones rather than at whatever stacking order it
 *  happened to be left with. */
let mountedNames = new Set();

function renderTiles() {
  if (!grid) return;
  // A transition that owns the tiles (the overview, a workspace slide) has
  // pinned them with its own inline styles; a layout pass now would tear them
  // out of its hands. Both re-render when they let go.
  if (document.documentElement.dataset.overview) return;
  if (slide?.holdsRender()) return;
  const names = surfacePlugins();

  // Names appearing for the first time are windows being opened now.
  const opening = names.filter((n) => !mountedNames.has(n));
  // Before the active set and the surface modules have loaded, the saved
  // window list is not authoritative: pruning against a half-loaded set would
  // wipe the restored workspaces. The first catalog pass turns this on.
  if (catalogReady) ensureWindows(names);
  // A focused plugin whose window is gone (closed, deactivated, or a stale
  // saved entry) leaves focus mode here, where the window list is authoritative.
  if (catalogReady) reconcilePluginFocus(names);

  // Give a window being opened the top of the stacking order *before* the
  // layout applies it, so one pass is enough.
  //
  // `geomFor` cannot do this itself: it only auto-assigns an order when none is
  // stored (`g.z <= 0`), so a plugin used earlier in the session keeps its old,
  // stale z and reopens *underneath* windows that have been focused since —
  // exactly what a reopened calculator did, at z=1294 below mail at z=1778.
  for (const name of opening) bumpZ(name);

  mountedNames = new Set(names);

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
    // Where each window on screen sits before this layout pass, so the
    // windows that keep their place can move into the new one.
    const before = shiftableBoxes(ordered);
    for (const { name, el } of ordered) {
      const inActiveWs = visible.includes(name);
      el.classList.toggle('hidden', !inActiveWs);
      if (inActiveWs) items.push({ name, el });
    }
    applyLayout(grid, items);
    animateLayoutShift(items, before, opening);
  }
  if (catalogReady && !animateOpenWindows) requestAnimationFrame(() => { animateOpenWindows = true; });

  grid.classList.toggle('hidden', names.length === 0);
  document.body.classList.toggle('tiles-active', names.length > 0);
  // After the grid is shown: a first window opens into a grid that was hidden.
  if (!phone) animateOpenings(opening, items, names);
  renderWorkspaceBar();
  // The dock moved into the traveler window — re-render it whenever tiles
  // change so it follows the tile (activation toggles, layout switches).
  window.dispatchEvent(new CustomEvent('artifact:dock', { detail: getDockSummaries() }));
  resizeMapSoon();

  // A window being opened is raised (above, before the layout) but deliberately
  // NOT focused. Focus mode is the user's explicit choice — the eye button, the
  // tray menu, or the AI — so a new window never enters it, never takes the
  // accent from the window being worked on, and never narrows the AI's skills.

  // The window set changed (opened, closed, activated, deactivated). Chrome
  // that follows a window rather than its plugin — the map's vignette, puck,
  // trips HUD and GPS — listens to this, after the tiles have been re-laid out.
  window.dispatchEvent(new CustomEvent('windows:changed', { detail: { names: [...names] } }));
}

/** Start a finger-driven workspace slide towards `dir` (+1 next, −1
 *  previous): a transition for gestures.js to drive, or null when there is
 *  nowhere to slide (phone, workspaces off, one workspace). */
export function beginWorkspaceSlide(dir) {
  if (document.documentElement.dataset.overview) return null;
  return slide?.begin(dir) || null;
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

  // The per-plugin "Full screen" preference still means "open fullscreen" — the
  // real one, with the window's own workspace. The in-app pointer alone would
  // leave a workspace sealed around a window that no longer owns it.
  const fs = getFullscreen();
  if (getPluginLayout(name) === 'full') {
    enterWindowFullscreen(name);
  } else if (fs && fs !== name) {
    // Focusing another window clears this workspace's fullscreen so we return to
    // the tiled layout — never the fullscreen window itself: a click on it must
    // not throw the screen away.
    exitWindowFullscreen();
  }
}

/** Show a window that was just opened, without focusing it: put it on screen and
 *  honour its "open full screen" preference. Neither the active window nor focus
 *  mode is set here — both belong to the user (or the AI, explicitly). */
function revealPlugin(name) {
  if (!name || !surfacePlugins().includes(name)) return;

  // Phone: one window at a time — switching is the whole gesture.
  if (PHONE_QUERY.matches) {
    activePhonePlugin = name;
    renderTiles();
    return;
  }

  revealWindow(name);
  // The per-plugin "Full screen" preference still means "open fullscreen".
  if (getPluginLayout(name) === 'full') enterWindowFullscreen(name);
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

/** Whether a mounted plugin window is doing something the user can still hear
 *  or watch. Those are never rebuilt by an update, even when off screen. */
function surfaceBusy(name) {
  const el = tileForPlugin(name);
  if (!el) return false;
  for (const media of el.querySelectorAll('audio, video')) {
    if (!media.paused && !media.ended) return true;
  }
  return false;
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
  slide = createWorkspaceSlide({
    grid,
    render: () => renderTiles(),
    applyLayout,
    isPhone: () => PHONE_QUERY.matches,
  });
  setSurfaceNamesProvider(() => surfacePlugins());
  setFullscreenSurfaceProvider(() => surfacePlugins());

  // Restore the windows the user left open. In fresh mode there is no saved
  // workspace (preferences.js returns nothing when "remember" is off), so the
  // set stays empty and the session starts with a clean desktop. Being
  // active/enabled alone never opens a window.
  for (const ws of getWorkspacesList()) {
    for (const name of ws.windows || []) {
      if (isCoreWindow(name)) coreOpen.add(name);
      else openWindows.add(name);
    }
  }

  void refreshCatalog().then(renderTiles);
  renderTiles();

  // Desktop state changes (workspace/focus/fullscreen) re-tile the windows.
  window.addEventListener('desktop:changed', (e) => {
    // Focus is geometry in Tiling and Windows layouts: the focused window is
    // the master pane, and in the floating layout it is the topmost one. A
    // focus-only repaint there would leave the old master in place (or the old
    // window in front) until some unrelated render happened to apply it.
    // Only the Columns layout paints focus without moving anything.
    if (isFocusOnlyChange(e.detail) && getLayout().mode === 'columns') {
      repaintWindowFocus();
      return;
    }

    const idx = activeWorkspaceIndex();
    // A workspace switch slides the old desktop out and the new one in
    // (desktop.js hints the true direction for wrap-around; otherwise infer
    // from the index delta). Focus/fullscreen changes don't animate, and a
    // slide committing its own switch has already shown the motion.
    const switching = lastWsIndex !== null && idx !== lastWsIndex
      && !slide?.isCommitting() && !document.documentElement.dataset.overview
      && !!slide?.available() && !window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    let outgoing = null;
    if (switching) {
      // The state has switched but the DOM still shows the old workspace:
      // capture it before the re-render hides it.
      if (!slide.finishNow()) outgoing = slide.capture();
    }
    renderTiles();
    if (outgoing) {
      const hinted = Number(e.detail?.slide);
      const dir = (Number.isFinite(hinted) && hinted !== 0) ? hinted : (idx > lastWsIndex ? 1 : -1);
      slide.play(outgoing, dir);
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
    if (name) void openPluginWindow(name, { focus: true });
  });
  window.addEventListener('plugins:changed', async (e) => {
    await refreshCatalog(true);
    if (!isPluginActive(MAP_TILE_PLUGIN)) unmountMapTile();
    if (!surfacePlugins().includes(activePhonePlugin)) activePhonePlugin = null;
    renderTiles();
    // A plugin just opened from the tray, launcher, Plugins page or the AI —
    // show its window. It is raised but not focused: neither the active window
    // nor focus mode changes on an open.
    const opened = e.detail?.opened;
    for (const name of (Array.isArray(opened) ? opened : (opened ? [opened] : []))) {
      void openPluginWindow(name);
    }
  });
}

/** Re-evaluate which plugin windows exist (called after activation checks). */
export function refreshTiles() {
  renderTiles();
}

export { mountMapTile, unmountMapTile };
export { isCoreWindow } from './coreWindows.js';
