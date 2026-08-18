// Tiny shared helper for "which plugins are active for the current user".
import { apiFetch } from './api.js';

let lastActive = new Set();

export async function refreshActivePlugins() {
  try {
    const res = await apiFetch('/api/plugins/active');
    lastActive = new Set((res?.data) || []);
  } catch (_) {
    // Keep the last known set on transient errors.
  }
  return new Set(lastActive);
}

/** Sync check against the last refreshed set (boot order: refresh first). */
export function isPluginActive(name) {
  return lastActive.has(name);
}
