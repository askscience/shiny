// Tiny shared helper for "which plugins are active for the current user".
import { apiFetch } from './api.js';

export async function refreshActivePlugins() {
  try {
    const res = await apiFetch('/api/plugins/active');
    return new Set((res?.data) || []);
  } catch (_) {
    return new Set();
  }
}