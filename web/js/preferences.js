import { apiFetch, getTraveler } from './api.js';

const AI_NAME_KEY = 'ai.name';
const OLLAMA_MODEL_KEY = 'ai.ollama_model';
const PLUGIN_LAYOUT_KEY = 'plugin.layout';
const DESKTOP_WORKSPACES_KEY = 'desktop.workspaces';
const DESKTOP_ACTIVE_KEY = 'desktop.active';
const DESKTOP_LAYOUT_KEY = 'desktop.layout';
const DESKTOP_REMEMBER_KEY = 'session.remember';
const DEFAULT_AI_NAME = 'Shiny';

function scopedKey(base) {
  const id = getTraveler()?.id;
  return id ? `${base}.${id}` : base;
}

/* ── Server persistence ────────────────────────────────────────
 * Preferences are the user's own space in the DATABASE. localStorage is only
 * a synchronous cache; every write is also flushed (debounced) to
 * /api/preferences, which stores rows keyed by (user_id, key).
 * ─────────────────────────────────────────────────────────────── */

let dirty = new Map();
let flushTimer = null;

function persist(base, rawValue) {
  dirty.set(base, rawValue);
  if (flushTimer) clearTimeout(flushTimer);
  flushTimer = setTimeout(flushPreferences, 400);
}

async function flushPreferences() {
  flushTimer = null;
  if (!dirty.size) return;
  const payload = {};
  for (const [k, v] of dirty) payload[k] = v;
  dirty = new Map();
  try {
    await apiFetch('/api/preferences', {
      method: 'PUT',
      authRedirect: false,
      body: JSON.stringify(payload),
    });
  } catch (_) {
    // Ignore transient failures; the next change re-flushes.
  }
}

/** Force an immediate flush (e.g. before navigating away on "Done"). */
export function flushPreferencesNow() {
  if (flushTimer) {
    clearTimeout(flushTimer);
    flushTimer = null;
  }
  return flushPreferences();
}

/** Load this user's saved preferences from the database into the local cache. */
export async function loadUserPreferences() {
  const id = getTraveler()?.id;
  if (!id) return;
  try {
    const res = await apiFetch('/api/preferences', { authRedirect: false });
    const data = res?.data || {};
    for (const [base, value] of Object.entries(data)) {
      localStorage.setItem(`${base}.${id}`, value);
    }
  } catch (_) {
    // Keep the existing local cache when the server is unreachable.
  }
}

export function getAiName() {
  return localStorage.getItem(scopedKey(AI_NAME_KEY)) || DEFAULT_AI_NAME;
}

export function setAiName(name) {
  const trimmed = (name || '').trim();
  const key = scopedKey(AI_NAME_KEY);
  if (trimmed) localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
  persist(AI_NAME_KEY, trimmed);
}

export function getOllamaModel() {
  return localStorage.getItem(scopedKey(OLLAMA_MODEL_KEY)) || '';
}

export function setOllamaModel(model) {
  const trimmed = (model || '').trim();
  const key = scopedKey(OLLAMA_MODEL_KEY);
  if (trimmed) localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
  persist(OLLAMA_MODEL_KEY, trimmed);
}

/**
 * "Remember my workspace" — when off (default) a sign-in starts fresh: no
 * plugin windows, no saved desktop, a new chat. When on, everything the user
 * left behind is restored (plugins, windows, layout, chats, cards).
 */
export function getRemember() {
  return localStorage.getItem(scopedKey(DESKTOP_REMEMBER_KEY)) === 'true';
}

export function setRemember(on) {
  const key = scopedKey(DESKTOP_REMEMBER_KEY);
  if (on) localStorage.setItem(key, 'true');
  else localStorage.removeItem(key);
  persist(DESKTOP_REMEMBER_KEY, on ? 'true' : '');
}

/**
 * Per-plugin window mode: 'tile' (right-rail tile) or 'full' (overlay
 * takeover). Stored per traveler, default 'tile'.
 */
export function getPluginLayout(name) {
  return localStorage.getItem(scopedKey(`${PLUGIN_LAYOUT_KEY}.${name}`)) || 'tile';
}

export function setPluginLayout(name, mode) {
  const key = scopedKey(`${PLUGIN_LAYOUT_KEY}.${name}`);
  if (mode === 'full') localStorage.setItem(key, 'full');
  else localStorage.removeItem(key);
  persist(`${PLUGIN_LAYOUT_KEY}.${name}`, mode === 'full' ? 'full' : '');
}

/* ── Desktop manager (workspaces + tiling layout) ───────────── */

const DEFAULT_DESKTOP_LAYOUT = {
  mode: 'master',        // 'master' | 'columns'
  master_ratio: 0.6,     // master fraction (0.25–0.85)
  orientation: 'left',   // 'left' | 'right' | 'top' | 'bottom'
  gap: 12,               // px between windows
};

function readJson(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    if (raw == null) return fallback;
    return JSON.parse(raw);
  } catch (_) {
    return fallback;
  }
}

/** Ordered workspace list: [{ id, windows: [pluginName] }]. */
export function getWorkspaces() {
  if (!getRemember()) return null; // fresh mode: never restore saved windows
  const ws = readJson(scopedKey(DESKTOP_WORKSPACES_KEY), null);
  return Array.isArray(ws) ? ws : null;
}

export function setWorkspaces(workspaces) {
  if (!getRemember()) return; // fresh mode: don't persist the desktop
  const raw = JSON.stringify(workspaces);
  localStorage.setItem(scopedKey(DESKTOP_WORKSPACES_KEY), raw);
  persist(DESKTOP_WORKSPACES_KEY, raw);
}

export function getActiveWorkspaceId() {
  if (!getRemember()) return null;
  return localStorage.getItem(scopedKey(DESKTOP_ACTIVE_KEY)) || null;
}

export function setActiveWorkspaceId(id) {
  if (!getRemember()) return;
  const key = scopedKey(DESKTOP_ACTIVE_KEY);
  if (id) localStorage.setItem(key, id);
  else localStorage.removeItem(key);
  persist(DESKTOP_ACTIVE_KEY, id || '');
}

/** Tiling layout config, merged over defaults. */
export function getDesktopLayout() {
  const stored = readJson(scopedKey(DESKTOP_LAYOUT_KEY), {});
  const out = { ...DEFAULT_DESKTOP_LAYOUT, ...(stored || {}) };
  out.mode = out.mode === 'columns' ? 'columns' : 'master';
  out.master_ratio = clamp(Number(out.master_ratio) || 0.6, 0.25, 0.85);
  out.orientation = ['left', 'right', 'top', 'bottom'].includes(out.orientation)
    ? out.orientation : 'left';
  out.gap = Math.round(clamp(Number(out.gap) ?? 12, 0, 40));
  return out;
}

export function setDesktopLayout(layout) {
  const raw = JSON.stringify(layout);
  localStorage.setItem(scopedKey(DESKTOP_LAYOUT_KEY), raw);
  persist(DESKTOP_LAYOUT_KEY, raw);
}

function clamp(n, min, max) {
  return Math.min(max, Math.max(min, n));
}
