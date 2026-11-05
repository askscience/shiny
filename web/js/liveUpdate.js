/**
 * liveUpdate.js — the page follows the server through an update.
 *
 * Nothing restarts for an update to show: the server loads changed plugins
 * again on its own (`/api/plugins/rescan`, the plugin watcher) and can swap
 * itself for a new binary without closing the port (`/api/system/reexec`).
 * What the page has to do is notice, and this module is that:
 *
 * - every `POLL_MS` it reads `/api/system/version` — the build fingerprint,
 *   the core web tree's fingerprint and one per loaded plugin — and compares
 *   with what it saw last;
 * - a plugin whose fingerprint moved gets its catalog entry refreshed and a
 *   `plugins:changed` event, so `tiles.js` reloads that window's module (a
 *   window that can carry its state over is rebuilt in place);
 * - a changed core web tree means the modules this page runs are stale: the
 *   desktop is stashed and the page reloads itself once, restoring every
 *   window — also with "Remember my workspace" off, because this is the same
 *   session continuing, not a new sign-in;
 * - when a request fails to reach the server (`server:unreachable`, raised by
 *   `api.js`) it says so once and polls fast until the server answers again,
 *   then dispatches `server:reconnected` for the streams that need to
 *   reconnect, and compares versions right away.
 */
import { apiFetch } from './api.js';
import { invalidatePluginCatalog } from './activePlugins.js';
import { t } from './i18n.js';
import { stashDesktopForReload } from './preferences.js';

const POLL_MS = 20000;
const RETRY_MS = 1000;

let known = null;        // the last version answer: { build, web, plugins }
let timer = null;
let outage = false;
let notice = null;       // the "reconnecting" toast, dismissed on return
let reloading = false;
let started = false;

const hooks = {
  reload: () => window.location.reload(),
  // The desktop arrangement to carry across the reload (fresh mode has no
  // persisted copy). Loaded late: desktop.js is heavy and not needed sooner.
  snapshot: async () => {
    const desktop = await import('./desktop.js');
    return { workspaces: desktop.getWorkspacesList(), active: desktop.activeWorkspaceIndex() };
  },
  // Toasts need the UI library and the DOM; tests pass their own.
  toast: async (message, options) => {
    const { toast } = await import('../ui/components/feedback.js');
    return toast(message, options);
  },
};

export function initLiveUpdate(options = {}) {
  Object.assign(hooks, options);
  if (started) return;
  started = true;
  window.addEventListener('server:unreachable', onUnreachable);
  document.addEventListener?.('visibilitychange', () => {
    if (!document.hidden) schedule(0);
  });
  schedule(0);
}

/** What the page currently believes is running (tests and diagnostics). */
export function knownVersion() {
  return known;
}

function schedule(ms) {
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => void check(), ms);
}

function onUnreachable() {
  if (outage || reloading) return;
  outage = true;
  void Promise.resolve(hooks.toast(
    t('system.update.reconnecting', null, 'Reconnecting to the server…'),
    { type: 'info', duration: 10 * 60 * 1000 },
  )).then((el) => { notice = el; }).catch(() => {});
  schedule(RETRY_MS);
}

function dismissNotice() {
  notice?.remove?.();
  notice = null;
}

/** One comparison. Exposed for tests; the schedule calls it on its own. */
export async function check() {
  if (reloading) return null;
  let data;
  try {
    data = (await apiFetch('/api/system/version', { authRedirect: false }))?.data;
  } catch (err) {
    // Not signed in yet, or the server is away: try again at the right pace.
    const away = !err?.status || err.status >= 502;
    if (away && !outage) onUnreachable();
    schedule(away ? RETRY_MS : POLL_MS);
    return null;
  }
  if (!data) {
    schedule(POLL_MS);
    return null;
  }
  if (outage) {
    outage = false;
    dismissNotice();
    window.dispatchEvent(new CustomEvent('server:reconnected'));
  }
  const result = await apply(data);
  if (!reloading) schedule(POLL_MS);
  return result;
}

async function apply(data) {
  const plugins = data.plugins && typeof data.plugins === 'object' ? data.plugins : {};
  if (!known) {
    known = { build: data.build, web: data.web, plugins: { ...plugins } };
    return { first: true };
  }
  if (data.web && data.web !== known.web) {
    await reload();
    return { reload: true };
  }
  const updated = Object.keys(plugins).filter((name) => plugins[name] !== known.plugins[name]);
  known = { build: data.build, web: data.web, plugins: { ...plugins } };
  if (updated.length) {
    invalidatePluginCatalog();
    window.dispatchEvent(new CustomEvent('plugins:changed', { detail: { updated } }));
    void Promise.resolve(hooks.toast(
      t('system.update.pluginsReloaded', { names: updated.join(', ') }, `Updated: ${updated.join(', ')}`),
      { type: 'ok' },
    )).catch(() => {});
  }
  return { updated };
}

async function reload() {
  reloading = true;
  dismissNotice();
  try {
    stashDesktopForReload(await hooks.snapshot());
  } catch (err) {
    console.warn('live update: the desktop could not be stashed before the reload', err);
  }
  void Promise.resolve(hooks.toast(
    t('system.update.reloading', null, 'Updating the interface…'),
    { type: 'info', duration: 8000 },
  )).catch(() => {});
  hooks.reload();
}
