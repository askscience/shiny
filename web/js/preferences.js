import { getTraveler } from './api.js';

const AI_NAME_KEY = 'ai.name';
const OLLAMA_MODEL_KEY = 'ai.ollama_model';
const PLUGIN_LAYOUT_KEY = 'plugin.layout';
const DESKTOP_WORKSPACES_KEY = 'desktop.workspaces';
const DESKTOP_ACTIVE_KEY = 'desktop.active';
const DESKTOP_LAYOUT_KEY = 'desktop.layout';
const DEFAULT_AI_NAME = 'Shiny';

function scopedKey(base) {
  const id = getTraveler()?.id;
  return id ? `${base}.${id}` : base;
}

export function getAiName() {
  return localStorage.getItem(scopedKey(AI_NAME_KEY)) || DEFAULT_AI_NAME;
}

export function setAiName(name) {
  const trimmed = (name || '').trim();
  const key = scopedKey(AI_NAME_KEY);
  if (trimmed) localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
}

export function getOllamaModel() {
  return localStorage.getItem(scopedKey(OLLAMA_MODEL_KEY)) || '';
}

export function setOllamaModel(model) {
  const trimmed = (model || '').trim();
  const key = scopedKey(OLLAMA_MODEL_KEY);
  if (trimmed) localStorage.setItem(key, trimmed);
  else localStorage.removeItem(key);
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
  const ws = readJson(scopedKey(DESKTOP_WORKSPACES_KEY), null);
  return Array.isArray(ws) ? ws : null;
}

export function setWorkspaces(workspaces) {
  localStorage.setItem(scopedKey(DESKTOP_WORKSPACES_KEY), JSON.stringify(workspaces));
}

export function getActiveWorkspaceId() {
  return localStorage.getItem(scopedKey(DESKTOP_ACTIVE_KEY)) || null;
}

export function setActiveWorkspaceId(id) {
  localStorage.setItem(scopedKey(DESKTOP_ACTIVE_KEY), id);
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
  localStorage.setItem(scopedKey(DESKTOP_LAYOUT_KEY), JSON.stringify(layout));
}

function clamp(n, min, max) {
  return Math.min(max, Math.max(min, n));
}
