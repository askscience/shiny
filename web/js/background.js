/**
 * background.js — the full-screen desktop background layer (#background).
 *
 * Modes (chosen in Settings → Background, stored per user in localStorage):
 *   none      — the default subtle accent/gradient mesh (layer stays blank)
 *   gradient  — a full-strength gradient, using the Appearance gradient
 *   image     — an uploaded image (served from /api/background), dimmed
 *   animated  — a CSS-animated preset ("aurora" drift or "shimmer" sweep)
 */

import { getGradient, gradientToCss } from '../ui/index.js';

const BG_KEY = 'ui.background';
const DEFAULT_BG = { mode: 'none', animation: null, url: null };

let getScope = () => null;
function scopedKey(base) {
  const id = getScope();
  return id ? `${base}.${id}` : base;
}

export function initBackground({ getScope: scopeGetter } = {}) {
  if (typeof scopeGetter === 'function') getScope = scopeGetter;
  applyBackground();
}

export function getBackground() {
  try {
    const raw = localStorage.getItem(scopedKey(BG_KEY));
    if (raw) {
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === 'object') {
        return { ...DEFAULT_BG, ...parsed };
      }
    }
  } catch (_) { /* fall through */ }
  return { ...DEFAULT_BG };
}

export function setBackground(patch) {
  const value = { ...getBackground(), ...patch };
  localStorage.setItem(scopedKey(BG_KEY), JSON.stringify(value));
  applyBackground();
  return value;
}

/** Re-apply after login / user switch (scope changed). */
export function refreshBackground() {
  applyBackground();
}

export function applyBackground() {
  const el = document.getElementById('background');
  if (!el) return;
  const bg = getBackground();

  el.classList.remove('bg-anim-aurora', 'bg-anim-shimmer');
  el.style.backgroundImage = '';
  el.style.backgroundSize = '';
  el.style.backgroundPosition = '';

  switch (bg.mode) {
    case 'gradient':
      el.style.backgroundImage = `linear-gradient(rgba(0, 0, 0, 0.42), rgba(0, 0, 0, 0.42)), ${gradientToCss(getGradient())}`;
      break;
    case 'image':
      el.style.backgroundImage = `linear-gradient(rgba(0, 0, 0, 0.5), rgba(0, 0, 0, 0.5)), url("${bg.url || '/api/background'}?v=${Date.now()}")`;
      break;
    case 'animated':
      el.classList.add(bg.animation === 'shimmer' ? 'bg-anim-shimmer' : 'bg-anim-aurora');
      break;
    default:
      break; // none — leave the layer blank so the CSS default mesh shows
  }
}
