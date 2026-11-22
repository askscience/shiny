/**
 * Report the errors of plugin windows to the server.
 *
 * A plugin's window runs in this page. When its module fails to load or its
 * code throws, the assistant that built it has no way to see the browser's
 * console: this module sends the error to `/api/plugins/dev/console`, where
 * the plugin builder's `plugin_dev_console` reads it.
 *
 * Only errors that come from a file under `/plugins/<name>/` are reported,
 * and nothing else about the page.
 */
import { apiFetch } from './api.js';

const PLUGIN_PATH = /\/plugins\/([A-Za-z0-9][A-Za-z0-9._-]*)\/([^?#\s:)]+)/;
let sent = 0;
const MAX_PER_PAGE = 200;

/** The plugin and file a URL or a stack trace points into, if any. */
export function pluginOf(text) {
  const match = PLUGIN_PATH.exec(String(text || ''));
  return match ? { plugin: match[1], source: match[2] } : null;
}

/** Report one error of a plugin's window. Never throws. */
export function reportPluginError(plugin, message, source = '', line = 0) {
  if (!plugin || !message || sent >= MAX_PER_PAGE) return;
  sent += 1;
  apiFetch('/api/plugins/dev/console', {
    method: 'POST',
    body: JSON.stringify({ plugin, message: String(message).slice(0, 600), source: String(source).slice(0, 200), line: Number(line) || 0 }),
  }).catch(() => { /* a report that cannot be sent is dropped */ });
}

export function initPluginConsole() {
  window.addEventListener('error', (event) => {
    const where = pluginOf(event.filename) || pluginOf(event.error?.stack);
    if (where) reportPluginError(where.plugin, event.message || String(event.error), where.source, event.lineno);
  });
  window.addEventListener('unhandledrejection', (event) => {
    const reason = event.reason;
    const where = pluginOf(reason?.stack);
    if (where) reportPluginError(where.plugin, reason?.message || String(reason), where.source, 0);
  });
}
