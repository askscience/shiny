/**
 * linkRouter.js — every link in the kiosk opens in a browser plugin.
 *
 * The shell refuses page popups on purpose (a link must never leave the kiosk),
 * which used to make app-drawn links dead: the popup request arrived as a
 * `new-window` event for the `main` view and matched no browser tab. This
 * module intercepts those clicks before the shell ever sees them and hands the
 * URL to the plugin the user picked as their link handler — the `browser`
 * plugin by default, or any installed plugin that declares the `open-urls`
 * capability (Settings → Browser → Links).
 *
 * The hand-off mirrors `openWithPlugin`/`onOpenFromFiles` in files.js: the
 * target is activated, a `url:open` event carries it the URL, and
 * `window.__pendingUrlOpen` covers the race where its window mounts a moment
 * later.
 */
import { apiFetch } from './api.js';
import { getLinkPlugin } from './preferences.js';
import { toast } from '../ui/index.js';
import { t } from './i18n.js';

/** `plugin.toml` capability that makes a plugin a link-handler candidate. */
export const LINK_CAPABILITY = 'open-urls';
/** The plugin links go to when the preference names no other candidate. */
export const DEFAULT_LINK_PLUGIN = 'browser';

const LINK_EVENT = 'url:open';

/**
 * Whether we are inside a kiosk shell — both shells expose `window.ipc`. In a
 * normal browser the default anchor behaviour is left alone: the in-app
 * browser has no native views to render pages outside the shell.
 */
export function isKiosk() {
  return !!window.ipc?.postMessage;
}

/**
 * The absolute http(s) URL a click should be routed for, or null when the link
 * must keep its default behaviour (`#`, relative, same-origin, `mailto:`,
 * `blob:`, `javascript:`). `href` is read from `a.href`, already absolutized.
 */
export function externalLinkUrl(href, origin = location.origin) {
  let url;
  try {
    url = new URL(href);
  } catch (_) {
    return null;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
  if (url.origin === origin) return null;
  return url.href;
}

/** The installed plugins that can open links: declared `open-urls` and ship a
 *  window surface. */
export function linkPlugins(catalog) {
  return (Array.isArray(catalog) ? catalog : []).filter(
    (p) => p && p.surface && Array.isArray(p.capabilities) && p.capabilities.includes(LINK_CAPABILITY),
  );
}

/**
 * The plugin a link click goes to: the user's choice while it is still
 * installed, otherwise the default, otherwise any candidate. Null when no
 * installed plugin can open links.
 */
export function resolveLinkPlugin(catalog, preferred = getLinkPlugin()) {
  const candidates = linkPlugins(catalog);
  const pick = (name) => (name && candidates.some((p) => p.name === name) ? name : null);
  return pick(preferred) || pick(DEFAULT_LINK_PLUGIN) || candidates[0]?.name || null;
}

let catalogCache = null;

/** The plugin catalog, cached for the clicks that follow a successful fetch. */
export async function linkCatalog() {
  try {
    const res = await apiFetch('/api/plugins');
    catalogCache = res?.data || [];
  } catch (_) {
    // Keep the last known catalog — a transient failure must not kill the click.
  }
  return catalogCache || [];
}

/**
 * Open `url` in the user's link-handler plugin: activate it, reveal its window
 * and hand it the URL. Resolves true when the hand-off was made.
 */
export async function openUrl(url) {
  if (!isKiosk() || !url) return false;
  const plugin = resolveLinkPlugin(await linkCatalog());
  if (!plugin) {
    toast(t('link.noBrowserPlugin', null, 'No browser plugin is installed'), { type: 'error' });
    return false;
  }
  try {
    await apiFetch('/api/plugins/activate', {
      method: 'POST',
      body: JSON.stringify({ name: plugin }),
    });
  } catch (err) {
    toast(err?.message || t('link.openFailed', { plugin }, `Could not open ${plugin}`),
      { type: 'error' });
    return false;
  }
  window.__pendingUrlOpen = { plugin, url, ts: Date.now() };
  window.dispatchEvent(new CustomEvent('plugins:changed', { detail: { opened: plugin } }));
  window.dispatchEvent(new CustomEvent(LINK_EVENT, { detail: { plugin, url } }));
  return true;
}

/** The one listener, at module scope: it survives windows coming and going. */
const linkHandlers = new Map();

window.addEventListener(LINK_EVENT, (e) => dispatchLink(e.detail));

function dispatchLink(detail) {
  if (!detail || typeof detail.url !== 'string' || !detail.url) return;
  const handler = linkHandlers.get(detail.plugin);
  if (!handler) return;
  const pending = window.__pendingUrlOpen;
  if (pending && pending.plugin === detail.plugin) window.__pendingUrlOpen = null;
  try {
    Promise.resolve(handler(detail.url)).catch(() => {});
  } catch (_) { /* an opener must never break the caller */ }
}

/**
 * Register as a link handler. Call once from the surface's `wireEvents` —
 * drains an open that was queued before the window existed, and handles later
 * ones. Re-registering (a window that unmounted and came back) replaces the
 * handler instead of stacking a second one.
 */
export function onOpenUrl(plugin, handler) {
  linkHandlers.set(plugin, handler);
  const pending = window.__pendingUrlOpen;
  if (pending && pending.plugin === plugin) dispatchLink(pending);
}

/** The one delegated click listener; installed by app.js, kiosk-only. */
export function initLinkRouter() {
  if (!isKiosk()) return;
  // Capture phase: a window's own click handling must not be able to stop a
  // link from reaching the router.
  document.addEventListener('click', onDocumentClick, true);
}

function onDocumentClick(event) {
  if (event.defaultPrevented || event.button !== 0) return;
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  const link = event.target instanceof Element ? event.target.closest('a[href]') : null;
  if (!link) return;
  const url = externalLinkUrl(link.href);
  if (!url) return;
  event.preventDefault();
  void openUrl(url);
}
