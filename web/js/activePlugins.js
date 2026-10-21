// Tiny shared helper for "which plugins are active for the current user" and
// the installed-plugin catalog. One in-flight request and one short cache keep
// the tray, launcher, tiles, settings and link router from each fetching the
// same JSON during boot.
import { apiFetch } from './api.js';

const CATALOG_TTL_MS = 30_000;

let lastActive = new Set();
let activeAt = 0;
let activePromise = null;

let lastCatalog = [];
let catalogAt = 0;
let catalogPromise = null;

function fresh(at) {
  return at !== 0 && Date.now() - at < CATALOG_TTL_MS;
}

function invalidate() {
  activeAt = 0;
  catalogAt = 0;
}

if (typeof window !== 'undefined') {
  window.addEventListener('plugins:changed', invalidate);
  window.addEventListener('auth:success', invalidate);
  // `/api/plugins` localizes description/summary/category through
  // Accept-Language, so a language switch invalidates the cached catalog too.
  window.addEventListener('locale:change', invalidate);
  window.addEventListener('storage', (e) => {
    if (e.key === 'plugins.changed') invalidate();
  });
}

export async function refreshActivePlugins({ force = false } = {}) {
  if (!force && fresh(activeAt)) return new Set(lastActive);
  if (!force && activePromise) return activePromise;
  activePromise = apiFetch('/api/plugins/active')
    .then((res) => {
      lastActive = new Set((res?.data) || []);
      activeAt = Date.now();
      return new Set(lastActive);
    })
    .catch(() => new Set(lastActive))
    .finally(() => { activePromise = null; });
  return activePromise;
}

/** Installed-plugin catalog for the current user. */
export async function refreshPluginCatalog({ force = false } = {}) {
  if (!force && fresh(catalogAt)) return lastCatalog;
  if (!force && catalogPromise) return catalogPromise;
  catalogPromise = apiFetch('/api/plugins')
    .then((res) => {
      lastCatalog = res?.data || [];
      catalogAt = Date.now();
      return lastCatalog;
    })
    .catch(() => lastCatalog)
    .finally(() => { catalogPromise = null; });
  return catalogPromise;
}

/** Drop both caches after an install/activation/locale change. */
export function invalidatePluginCatalog() {
  invalidate();
}

/** Sync check against the last refreshed set (boot order: refresh first). */
export function isPluginActive(name) {
  return lastActive.has(name);
}

/**
 * Start a clean plugin session ("Remember my workspace" off): drop the
 * activations a previous sign-in left behind. The server also stops any Craft
 * app MCP process that outlived its window. Best effort — the desktop still
 * starts empty even if the call fails, and the next sign-in retries.
 */
export async function resetSessionPlugins() {
  lastActive = new Set();
  invalidatePluginCatalog();
  try {
    await apiFetch('/api/plugins/session/reset', {
      method: 'POST',
      authRedirect: false,
    });
  } catch (_) {
    // Keep going: no windows are restored in fresh mode either way.
  }
}
