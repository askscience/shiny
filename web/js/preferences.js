import { getTraveler } from './api.js';

const AI_NAME_KEY = 'ai.name';
const OLLAMA_MODEL_KEY = 'ai.ollama_model';
const PLUGIN_LAYOUT_KEY = 'plugin.layout';
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
