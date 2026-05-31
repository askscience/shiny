import { DEFAULT_ACCENT } from './accent.js';
import { getTraveler } from './api.js';

const AI_NAME_KEY = 'ai.name';
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

export function getAccent() {
  return localStorage.getItem(scopedKey('ui.accent')) || DEFAULT_ACCENT;
}

export function setAccent(hex) {
  localStorage.setItem(scopedKey('ui.accent'), hex);
}
