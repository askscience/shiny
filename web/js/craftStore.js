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
 */
import { apiFetch } from './api.js';
import { notify } from '../ui/index.js';
import { t } from './i18n.js';
import { getStoreAutoUpdate } from './preferences.js';

/** How often the running shell re-checks the catalog. */
const CHECK_MS = 30 * 60 * 1000;
/** Delay before the first check, so boot work comes first. */
const FIRST_CHECK_MS = 30 * 1000;

let snapshot = null;
let timer = null;
let working = false;

/** The last catalog the core returned (null before the first load). */
export function storeSnapshot() {
  return snapshot;
}

/** True while an app is mid-install (its state comes from the core). */
export function craftAppBusy(app) {
  return app?.state === 'downloading' || app?.state === 'installing';
}

/** Read the catalog. Cheap: the core schedules its own refresh when stale. */
export async function loadStore() {
  const res = await apiFetch('/api/plugins/store');
  if (res?.data) snapshot = res.data;
  return snapshot;
}

/** Ask the core to re-check every app's latest release, ignoring its cache. */
export async function refreshStore() {
  await apiFetch('/api/plugins/store/refresh', { method: 'POST' });
  return loadStore();
}

/** Install or update one app. The core downloads, verifies and installs it in
 *  the background; the caller follows `loadStore()` for progress. */
export async function installCraftApp(id) {
  await apiFetch('/api/plugins/store/install', { method: 'POST', body: JSON.stringify({ id }) });
  return loadStore();
}

/** Turn an app's AI control (its MCP server) on or off. Once on, the agent can
 *  drive the app only while that app's window is in focus mode — a rule the
 *  core enforces, not a prompt instruction. */
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
    const outdated = (snapshot?.apps || []).filter(
      (app) => app.installed_version && app.update_available && !craftAppBusy(app),
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
}
