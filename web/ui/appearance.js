/**
 * appearance — the accent / gradient engine.
 *
 * The UI is monochrome; the user picks ONE accent color and ONE gradient
 * (presets from the active theme manifest, or custom). This module derives
 * every accent-related token (--accent-soft, --gradient-mesh, …) and
 * broadcasts `appearance:change` so JS-rendered surfaces (map, orb canvas)
 * can follow. A legacy `accent:change` event is also dispatched.
 *
 * Storage is per user; the app injects its scope via initAppearance:
 *   initAppearance({ getScope: () => getTraveler()?.id })
 */

import { getThemeManifest } from './theme-loader.js';

const ACCENT_KEY = 'ui.accent';
const GRADIENT_KEY = 'ui.gradient';
const DEFAULT_ACCENT = '#ffffff';
const DEFAULT_GRADIENT = { id: 'mono', angle: 135, stops: ['#ffffff', '#8a8a8a'] };

let getScope = () => null;

function scopedKey(base) {
  const id = getScope();
  return id ? `${base}.${id}` : base;
}

/* ── color math ─────────────────────────────────────────────── */

export function hexToRgb(hex) {
  const h = String(hex || '').replace('#', '');
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h;
  const n = parseInt(full, 16);
  if (Number.isNaN(n)) return [255, 255, 255];
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function rgba(hex, alpha) {
  const [r, g, b] = hexToRgb(hex);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Readable text color (near-black or near-white) for a given background. */
export function contrastFor(hex) {
  const [r, g, b] = hexToRgb(hex);
  const luminance = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  return luminance > 0.6 ? '#0a0a0a' : '#fafafa';
}

/** Current computed value of a CSS variable on :root (for canvas/JS rendering). */
export function cssVar(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/* ── stored state ───────────────────────────────────────────── */

export function getAccent() {
  return localStorage.getItem(scopedKey(ACCENT_KEY)) || DEFAULT_ACCENT;
}

export function setAccent(hex) {
  localStorage.setItem(scopedKey(ACCENT_KEY), hex);
}

export function getGradient() {
  try {
    const raw = localStorage.getItem(scopedKey(GRADIENT_KEY));
    if (raw) {
      const parsed = JSON.parse(raw);
      if (parsed && Array.isArray(parsed.stops) && parsed.stops.length >= 2) return parsed;
    }
  } catch (_) { /* fall through */ }
  return DEFAULT_GRADIENT;
}

export function setGradient(gradient) {
  localStorage.setItem(scopedKey(GRADIENT_KEY), JSON.stringify(gradient));
}

/** Gradient presets declared by the active theme. */
export function gradientPresets() {
  return getThemeManifest()?.gradients || [];
}

export function accentPresets() {
  return getThemeManifest()?.accents || [];
}

/* ── application ────────────────────────────────────────────── */

export function gradientToCss(gradient) {
  const angle = Number.isFinite(gradient?.angle) ? gradient.angle : 135;
  const stops = gradient?.stops?.length >= 2 ? gradient.stops : DEFAULT_GRADIENT.stops;
  return `linear-gradient(${angle}deg, ${stops.join(', ')})`;
}

export function applyAppearance({ accent = getAccent(), gradient = getGradient() } = {}) {
  const root = document.documentElement;
  const stops = gradient?.stops?.length >= 2 ? gradient.stops : DEFAULT_GRADIENT.stops;
  const tail = stops[stops.length - 1];

  root.style.setProperty('--accent', accent);
  root.style.setProperty('--accent-2', tail);
  root.style.setProperty('--accent-soft', rgba(accent, 0.1));
  root.style.setProperty('--accent-glow', rgba(accent, 0.24));
  root.style.setProperty('--accent-contrast', contrastFor(accent));
  root.style.setProperty('--gradient-accent', gradientToCss(gradient));
  root.style.setProperty('--gradient-text',
    `linear-gradient(120deg, var(--text) 0%, ${stops[0]} 45%, ${tail} 100%)`);
  // Ambient background mesh — an accent presence, still quiet, but now
  // visible enough to give the surface a soft glow (not just a whisper).
  root.style.setProperty('--gradient-mesh',
    `radial-gradient(ellipse 80% 60% at 20% 10%, ${rgba(stops[0], 0.13)} 0%, transparent 55%),` +
    `radial-gradient(ellipse 70% 50% at 80% 90%, ${rgba(tail, 0.1)} 0%, transparent 50%)`);

  const detail = { accent, gradient };
  window.dispatchEvent(new CustomEvent('appearance:change', { detail }));
  window.dispatchEvent(new CustomEvent('accent:change', { detail: { accent } })); // legacy
}

/** Boot: wire per-user scoping and apply the stored appearance. */
export function initAppearance({ getScope: scopeGetter } = {}) {
  if (typeof scopeGetter === 'function') getScope = scopeGetter;
  applyAppearance();
}

/** Re-apply after login / user switch (scope changed). */
export function refreshAppearance() {
  applyAppearance();
}
