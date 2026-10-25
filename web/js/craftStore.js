/**
 * craftStore.js — the Craft App Suite catalog and its background updater.
 *
 * The catalog lives in the core (`/api/plugins/store`): it resolves each app's
 * latest GitHub release, downloads the app's own web build, verifies its
 * SHA-256 and installs it as a web-app plugin. This module is the single place
 * the shell talks to it, so the Plugins window's Store tab and the always-on
 * updater never disagree about what is installed.
 *
 * Updating is front-end driven on purpose — the same design the Updates plugin
 * uses: the core answers cheaply from its cache and does the work when asked,
 * and the shell asks on a timer while it runs. Installed apps update
 * themselves by default (`store.autoUpdate`, Settings-independent toggle in the
 * store tab); installing a *new* app is always the user's move.
 *
 * A live image can boot before its network is up. The core then reports
 * `waiting_for_network` instead of caching a GitHub failure as a real answer,
 * and this module retries on a short backoff — plus immediately on the browser's
 * `online` event — so the catalog appears on its own once the connection lands.
 */
import { apiFetch } from './api.js';
import { notify } from '../ui/index.js';
import { t } from './i18n.js';
import { getStoreAutoUpdate } from './preferences.js';

/** How often the running shell re-checks the catalog. */
const CHECK_MS = 30 * 60 * 1000;
/** Delay before the first check, so boot work comes first. */
const FIRST_CHECK_MS = 30 * 1000;
/** Retry backoff while the catalog is waiting for the network. */
const RETRY_MIN_MS = 4 * 1000;
const RETRY_MAX_MS = 15 * 1000;

let snapshot = null;
let timer = null;
let working = false;
let retryTimer = null;
let retryDelay = RETRY_MIN_MS;
/** True when the catalog request itself failed (the core, not just GitHub). */
let unreachable = false;
/** The last permanent read failure, for a "store unavailable" state. */
let lastError = null;
/** Coalesces simultaneous reads (tab opening, retry timer, online event). */
let inFlight = null;

/** The last catalog the core returned (null before the first load). */
export function storeSnapshot() {
  return snapshot;
}

/** True while the catalog cannot be resolved yet: the shell should wait and
 *  retry instead of presenting a GitHub/network error. */
export function storeWaitingForNetwork() {
  return unreachable || snapshot?.waiting_for_network === true;
}

/** True when the last read failed for a reason retrying cannot fix (an expired
 *  session, a missing endpoint) rather than a network wait. */
export function storeLoadFailed() {
  return lastError !== null;
}

/** True while an app is mid-install (its state comes from the core). */
export function craftAppBusy(app) {
  return app?.state === 'downloading' || app?.state === 'installing';
}

/** Tell the Plugins window, the updater and any open surface that the catalog
 *  changed. Rendering stays with the surfaces; this module owns the data. */
function announceStore() {
  if (typeof window === 'undefined' || typeof window.dispatchEvent !== 'function') return;
  try {
    window.dispatchEvent(new CustomEvent('craft-store:changed'));
  } catch (_) {
    // A webview without CustomEvent still gets the next poll's render.
  }
}

/** Connectivity failures and server hiccups are worth retrying; a 401/404 is
 *  a real answer. `ApiError` carries the HTTP status; fetch failures do not. */
function transientError(error) {
  const status = Number(error?.status);
  if (!status) return true;
  return status === 403 || status === 429 || status >= 500;
}

function scheduleRetry() {
  if (retryTimer) return; // keep the earliest pending attempt
  const delay = retryDelay;
  retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
  retryTimer = setTimeout(() => {
    retryTimer = null;
    void loadStore().catch(() => { /* loadStore schedules the next attempt */ });
  }, delay);
  // Under Node's test runner a pending background retry must not keep the
  // process alive. Browsers' numeric timer ids have no unref().
  retryTimer?.unref?.();
}

function clearRetry() {
  if (retryTimer) clearTimeout(retryTimer);
  retryTimer = null;
}

/** Record a failed read and schedule the next one. Returns false for errors
 *  that retrying cannot fix (an expired session, a missing endpoint). */
function noteFailure(error) {
  lastError = error;
  if (!transientError(error)) {
    unreachable = false;
    return false;
  }
  unreachable = true;
  scheduleRetry();
  return true;
}

/** Read the catalog. Cheap: the core schedules its own refresh when stale.
 *  While the catalog waits for the network, retries happen on their own. */
export function loadStore() {
  if (inFlight) return inFlight;
  inFlight = (async () => {
    try {
      const res = await apiFetch('/api/plugins/store');
      if (res?.data) snapshot = res.data;
      unreachable = false;
      lastError = null;
      if (snapshot?.waiting_for_network || snapshot?.refreshing) {
        // The core reached us but GitHub did not (or a refresh is still
        // running): keep polling until the next snapshot has the answers.
        scheduleRetry();
      } else {
        retryDelay = RETRY_MIN_MS;
        clearRetry();
      }
      announceStore();
      return snapshot;
    } catch (error) {
      noteFailure(error);
      announceStore();
      throw error;
    } finally {
      inFlight = null;
    }
  })();
  return inFlight;
}

/** Ask the core to re-check every app's latest release, ignoring its cache. */
export async function refreshStore() {
  try {
    await apiFetch('/api/plugins/store/refresh', { method: 'POST' });
  } catch (error) {
    noteFailure(error);
    announceStore();
    throw error;
  }
  return loadStore();
}

/** Retry now: used by the browser's `online` event (and tests). Forces the
 *  core's retry window open, then reads whatever the refresh has so far. */
export async function retryStoreNow() {
  clearRetry();
  retryDelay = RETRY_MIN_MS;
  try {
    if (storeWaitingForNetwork()) {
      await apiFetch('/api/plugins/store/refresh', { method: 'POST' });
    }
    return await loadStore();
  } catch (error) {
    noteFailure(error);
    announceStore();
    throw error;
  }
}

/** Install or update one app. The core downloads, verifies and installs it in
 *  the background; the caller follows `loadStore()` for progress. */
export async function installCraftApp(id) {
  await apiFetch('/api/plugins/store/install', { method: 'POST', body: JSON.stringify({ id }) });
  return loadStore();
}

/** Turn an app's AI control (its MCP server) on or off. Once on, the agent can
 *  drive every app whose window is open, and focus mode narrows that to the
 *  focused apps — a rule the core enforces, not a prompt instruction. */
export async function setCraftMcp(id, enable) {
  await apiFetch('/api/plugins/store/mcp', {
    method: 'POST',
    body: JSON.stringify({ id, enable }),
  });
  return loadStore();
}

/**
 * One background pass: refresh the catalog and queue every available update
 * when the user allows automatic updates. Returns the app ids it queued.
 */
export async function craftUpdateTick({ force = false } = {}) {
  if (working) return [];
  working = true;
  try {
    if (force || !snapshot) await refreshStore();
    else await loadStore();
    if (!getStoreAutoUpdate()) return [];
    // Outdated apps update themselves — and so does an install that is missing
    // its AI control (the app's CLI), which is what lets an app that was already
    // on disk pick its CLI up without the user hunting for a second download.
    const outdated = (snapshot?.apps || []).filter(
      (app) => app.installed_version && (app.update_available || app.mcp_pending) && !craftAppBusy(app),
    );
    for (const app of outdated) await installCraftApp(app.id);
    if (outdated.length) {
      notify({
        app: t('plugins.storeApp', null, 'Craft store'),
        title: outdated.length === 1
          ? t('plugins.storeUpdatingOne', { name: outdated[0].name }, `Updating ${outdated[0].name}`)
          : t('plugins.storeUpdatingMany', { count: outdated.length }, `Updating ${outdated.length} apps`),
        body: t('plugins.storeUpdatingBody', null, 'They keep working while they update.'),
        icon: 'ui/download',
      });
    }
    return outdated.map((app) => app.id);
  } finally {
    working = false;
  }
}

/** Start the periodic check. Idempotent: called once from the shell boot. */
export function initCraftStore() {
  if (timer) return;
  const tick = () => { void craftUpdateTick().catch(() => { /* offline: try later */ }); };
  setTimeout(tick, FIRST_CHECK_MS);
  timer = setInterval(tick, CHECK_MS);
  // A live image usually gets its network after the shell is already up: retry
  // immediately instead of waiting for the backoff (or the 30-minute check).
  if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
    window.addEventListener('online', () => {
      retryDelay = RETRY_MIN_MS;
      void retryStoreNow().catch(() => { /* a new retry is scheduled */ });
    });
  }
}
