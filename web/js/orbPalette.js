/**
 * orbPalette — shared colour source for the voice orb (2D fallback + WebGL).
 *
 * The reference look is chromatic: cyan → violet → magenta light on black.
 * A neutral (white/grey) accent would normally give a monochrome orb, so a
 * neutral accent is anchored to that reference split, while a genuinely
 * colourful accent keeps its own hue family (rotated into cool / deep / warm
 * partners). The accent still drives lightness, so the orb stays "yours".
 */

import { getAccent, getGradient, themeMode } from '../ui/index.js';

/** True when the orb sits on paper rather than on night. */
export function isLightCanvas() {
  return themeMode() === 'light';
}

/**
 * Deepen one colour for a light canvas.
 *
 * On black the orb is light — additive, luminous, bright. On paper that same
 * fan washes out to nothing, so the light canvas gets the ink version of it:
 * same hue, more saturation, a fraction of the lightness. Dark themes keep the
 * bright colour untouched.
 */
export function inkFor(hex) {
  if (!isLightCanvas()) return hex;
  const [r, g, b] = hexToRgb(hex);
  const [h, s, l] = rgbToHsl(r, g, b);
  return hslToHex(h, Math.min(0.95, s + 0.12), Math.min(0.4, l * 0.6));
}

function hexToRgb(hex) {
  const h = (hex || '#ffffff').replace('#', '');
  const n = parseInt(h.length === 3 ? h.split('').map((c) => c + c).join('') : h, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function rgbToHsl(r, g, b) {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h;
  if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [h * 60, s, l];
}

function hue2rgb(p, q, t) {
  let x = t;
  if (x < 0) x += 1;
  if (x > 1) x -= 1;
  if (x < 1 / 6) return p + (q - p) * 6 * x;
  if (x < 1 / 2) return q;
  if (x < 2 / 3) return p + (q - p) * (2 / 3 - x) * 6;
  return p;
}

function hslToHex(h, s, l) {
  const hh = (((h % 360) + 360) % 360) / 360;
  let r;
  let g;
  let b;
  if (s === 0) {
    r = g = b = l;
  } else {
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    r = hue2rgb(p, q, hh + 1 / 3);
    g = hue2rgb(p, q, hh);
    b = hue2rgb(p, q, hh - 1 / 3);
  }
  const to = (v) => Math.round(v * 255).toString(16).padStart(2, '0');
  return `#${to(r)}${to(g)}${to(b)}`;
}

export function lighten(hex, amount) {
  const [r, g, b] = hexToRgb(hex);
  const mix = (c) => Math.min(255, Math.round(c + (255 - c) * amount));
  const to = (c) => mix(c).toString(16).padStart(2, '0');
  return `#${to(r)}${to(g)}${to(b)}`;
}

/** Cool / deep / warm partners for the accent: the reference chromatic split. */
function chromaticPartners(accent) {
  const [r, g, b] = hexToRgb(accent);
  const [h, s, l] = rgbToHsl(r, g, b);
  const L = Math.min(0.78, Math.max(0.54, l));
  if (s < 0.2) {
    // Neutral accent → the cyan/violet/magenta split from the references.
    return {
      cool: hslToHex(196, 0.92, L),
      deep: hslToHex(262, 0.74, Math.min(0.68, L * 0.9)),
      warm: hslToHex(301, 0.86, L),
    };
  }
  return {
    cool: hslToHex(h - 40, Math.min(s + 0.12, 0.92), L),
    deep: hslToHex(h + 4, Math.min(s + 0.06, 0.86), Math.min(0.68, L * 0.9)),
    warm: hslToHex(h + 64, Math.min(s + 0.12, 0.92), L),
  };
}

/**
 * The spectral fan used by the two "rainbow" looks — the prism flare's spikes
 * and the eclipse corona. Hue order runs cyan → blue → violet → magenta →
 * pink → amber → green and wraps, so sampling it around a ring never seams.
 * It is anchored on the accent's hue (a colourful accent rotates the whole
 * fan) and falls back to the reference chromatic spread for a neutral accent.
 */
const SPECTRUM_HUES = [196, 232, 268, 300, 336, 20, 64];

/** Shortest-path hue interpolation, so 336° → 20° travels through 0°. */
function hueLerp(a, b, t) {
  const d = ((((b - a) % 360) + 540) % 360) - 180;
  return a + d * t;
}

/** `count` colours sampled evenly (and cyclically) from the accent's fan. */
export function spectrumForState(state, count = SPECTRUM_HUES.length) {
  const palette = paletteForState(state);
  // Error and disabled carry their own (red / grey) signal, so the two
  // spectral looks keep that signal instead of painting a rainbow.
  if (state === 'error' || state === 'disabled') {
    return Array.from({ length: count }, (_, i) => palette[i % 3]);
  }
  const accent = getAccent();
  const [r, g, b] = hexToRgb(accent);
  const [h, s, l] = rgbToHsl(r, g, b);
  const neutral = s < 0.2;
  const base = neutral ? 0 : h - SPECTRUM_HUES[0];
  const sat = neutral ? 0.9 : Math.min(0.92, s + 0.15);
  const bright = neutral ? 0.62 : Math.min(0.72, Math.max(0.5, l));
  // Light canvas: the same fan, mixed as ink instead of as light.
  const light = isLightCanvas() ? Math.min(0.4, bright * 0.6) : bright;
  const n = SPECTRUM_HUES.length;
  const out = [];
  for (let i = 0; i < count; i++) {
    const pos = (i / count) * n;
    const i0 = Math.floor(pos) % n;
    const i1 = (i0 + 1) % n;
    const hue = hueLerp(SPECTRUM_HUES[i0], SPECTRUM_HUES[i1], pos - Math.floor(pos));
    out.push(hslToHex(base + hue, sat, light));
  }
  return out;
}

/** Palette for one orb state, as [cool, deep, warm, light] by convention. */
export function paletteForState(state) {
  const accent = getAccent();
  const grad = getGradient();
  const stops = grad?.stops?.length >= 2 ? grad.stops : ['#ffffff', '#8a8a8a'];
  const light = lighten(accent, 0.26);
  const pale = lighten(accent, 0.5);
  const { cool, deep, warm } = chromaticPartners(accent);
  const byState = {
    idle: [cool, deep, warm, light],
    conversation: [cool, deep, warm, light],
    listening: [pale, cool, warm, light],
    speaking: [light, cool, deep, warm],
    compose: [cool, light, warm, deep],
    processing: [cool, light, warm, deep],
    downloading: [light, cool, deep, warm],
    error: ['#fca5a5', '#f87171', '#fb7185', '#ef4444'],
    disabled: ['#6b6b6b', '#4a4a4a', '#8a8a8a', '#333333'],
  };
  const palette = byState[state] || byState.idle;
  // `stops` is only read to keep the gradient part of the palette family when
  // a theme supplies one; the chromatic partners carry the look.
  void stops;
  // Every entry is ink on a light canvas — including the error reds, which are
  // deliberately pale and would otherwise vanish on paper.
  return palette.map(inkFor);
}
