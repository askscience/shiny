import { DEFAULT_ACCENT } from './accent.js';

const AI_NAME_KEY = 'ai.name';
const DEFAULT_AI_NAME = 'Atlas';

export function getAiName() {
  return localStorage.getItem(AI_NAME_KEY) || DEFAULT_AI_NAME;
}

export function setAiName(name) {
  const trimmed = (name || '').trim();
  if (trimmed) localStorage.setItem(AI_NAME_KEY, trimmed);
  else localStorage.removeItem(AI_NAME_KEY);
}

export function getAccent() {
  return localStorage.getItem('ui.accent') || DEFAULT_ACCENT;
}

export function setAccent(hex) {
  localStorage.setItem('ui.accent', hex);
}
