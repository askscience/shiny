/**
 * glow — the shared ambient blurred-mirror layer for plugin windows.
 *
 * Two tiers (see AMBIENT_GLOW_PLAN.md):
 *   Tier 0 — every window gets a seeded "global accent + partner hue" color
 *            glow for free. Core installs it with the window chrome
 *            (tiles.js → ensureWindowChrome), so no plugin CSS is needed.
 *   Tier 1 — a plugin overrides that with its real subject (photo, PDF page,
 *            artwork, track colour, map tile…) via setTileGlow().
 *
 * The layer paints behind all content: .tile is an isolated stacking context
 * and .tile-glow sits at z-index -1, which is above the window surface and
 * below every child — so it works in every plugin untouched.
 */
import { cssVar, hexToRgb } from '../appearance.js';
import { getThemeManifest } from '../theme-loader.js';

/** True when the active theme is a light one (glow must brighten, not darken). */
function isLightTheme() {
  return getThemeManifest()?.modes?.[0] === 'light';
}

/* ── colour math ────────────────────────────────────────────── */

function rgbToHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
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
  let r; let g; let b;
  if (s === 0) {
    r = g = b = l;
  } else {
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    r = hue2rgb(p, q, hh + 1 / 3);
    g = hue2rgb(p, q, hh);
    b = hue2rgb(p, q, hh - 1 / 3);
  }
  return `#${[r, g, b].map((v) => Math.round(v * 255).toString(16).padStart(2, '0')).join('')}`;
}

/** Stable, well-spread pseudo-random angle for a window, from its name.
 *  Deterministic on purpose: a window keeps its colour personality across
 *  reloads, while the set of windows still looks varied. */
function seedAngle(name) {
  let h = 2166136261;
  const s = String(name || 'plugin');
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return Math.abs(h) % 360;
}

/** Tier 0 partner colour: the accent's hue rotated per window, with a
 *  saturation floor so a white/gray accent still blooms into real colour. */
export function partnerColor(hex, angle) {
  const [r, g, b] = hexToRgb(hex || '#ffffff');
  let [h, s, l] = rgbToHsl(r, g, b);
  if (s < 0.18) { s = 0.55; l = 0.55; }
  return hslToHex(h + angle, Math.min(s + 0.05, 0.9), Math.min(Math.max(l, 0.45), 0.62));
}

function paintVars(host, name) {
  if (!host) return;
  const angle = seedAngle(name);
  const accent = cssVar('--accent') || '#ffffff';
  host.style.setProperty('--glow-a', accent);
  host.style.setProperty('--glow-b', partnerColor(accent, angle));
  host.style.setProperty('--glow-x', `${12 + (angle % 56)}%`);
  host.style.setProperty('--glow-y', `${(angle * 3) % 34}%`);
  host.style.setProperty('--glow-x2', `${58 + (angle % 36)}%`);
  host.style.setProperty('--glow-y2', `${68 + (angle % 30)}%`);
  // Light themes: a dark blur turns white pages to gray, so brighten instead.
  // Intensity itself lives in CSS (`.tile` / `.tile[data-glow-light]`) so each
  // theme can tune or override it from its own app.css.
  host.toggleAttribute('data-glow-light', isLightTheme());
}

/* ── tier 0: the universal colour glow ──────────────────────── */

/** Create + show a window's glow layer. Idempotent. */
export function installGlow(tileEl, name) {
  if (!tileEl) return null;
  paintVars(tileEl, name || tileEl.dataset?.plugin || '');
  let el = tileEl.querySelector(':scope > .tile-glow:not(.tile-glow--rim)');
  if (!el) {
    el = document.createElement('div');
    el.className = 'tile-glow';
    el.setAttribute('aria-hidden', 'true');
    tileEl.prepend(el);
  }
  el.classList.add('tile-glow--default', 'tile-glow--on');
  return el;
}

/** A window's glow layer, installing it if the window mounted before the
 *  chrome did. Plugins use this instead of caching a reference. */
export function glowFor(tileEl) {
  return tileEl?.querySelector(':scope > .tile-glow:not(.tile-glow--rim)')
    || installGlow(tileEl, tileEl?.dataset?.plugin || '');
}

/** Repaint Tier 0 colours (theme / accent changed). */
export function refreshGlow(tileEl, name) {
  if (!tileEl) return;
  paintVars(tileEl, name || tileEl.dataset?.plugin || '');
  const el = tileEl.querySelector(':scope > .tile-glow.tile-glow--default');
  if (el && !el.classList.contains('tile-glow--subject')) {
    el.classList.add('tile-glow--on');
  }
}

/** Rim layer for opaque, full-bleed windows (the map). Inherits the tile's
 *  Tier 0 colour variables; returns the rim element so callers can override
 *  its background-image with a sampled subject. */
export function createRim(tileEl) {
  if (!tileEl) return null;
  let el = tileEl.querySelector(':scope > .tile-glow--rim');
  if (!el) {
    el = document.createElement('div');
    el.className = 'tile-glow tile-glow--default tile-glow--rim';
    el.setAttribute('aria-hidden', 'true');
    tileEl.appendChild(el);
  }
  el.classList.add('tile-glow--on');
  return el;
}

/* ── tier 1: the per-plugin subject override ────────────────── */

/** Override with any CSS <image> — url("…") or a gradient. Passing null
 *  restores the window's colour glow; the glow is never fully off.
 *
 *  The image goes into --glow-image (not background-image) so the stylesheet
 *  can compose a semi-transparent wash of the window colour on top of it,
 *  which keeps the light as an ambient tint instead of a saturated band. */
export function setGlow(glowEl, imageCss) {
  if (!glowEl) return;
  if (imageCss) {
    glowEl.style.setProperty('--glow-image', imageCss);
    glowEl.classList.add('tile-glow--subject', 'tile-glow--on');
  } else {
    glowEl.style.removeProperty('--glow-image');
    glowEl.classList.remove('tile-glow--subject');
    glowEl.classList.add('tile-glow--on');
  }
}

/** Convenience for plugins: find/create the window glow, then set/clear it.
 *  `tileEl` is the plugin's `.tile` element; `imageCss` any CSS image. */
export function setTileGlow(tileEl, imageCss) {
  if (!tileEl) return;
  setGlow(glowFor(tileEl), imageCss);
}

/* ── source helpers ─────────────────────────────────────────── */

export const glowUrl = (url) => (url ? `url("${url}")` : null);

/** Build a soft two-blob radial gradient that reads as ambient light. The
 *  multi-stop falloff approximates a Gaussian, so no CSS blur is needed. */
export function glowGradient(colorA, colorB, {
  x = 20, y = 0, x2 = 82, y2 = 100,
} = {}) {
  if (!colorA) return null;
  const b = colorB || colorA;
  const soft = (c, p) => `color-mix(in srgb, ${c} ${p}%, transparent)`;
  const blob = (c, bx, by) =>
    `radial-gradient(85% 100% at ${bx}% ${by}%, ${c} 0%, ${soft(c, 78)} 18%, `
    + `${soft(c, 48)} 40%, ${soft(c, 20)} 62%, transparent 86%)`;
  return `${blob(colorA, x, y)}, ${blob(b, x2, y2)}`;
}

/** Downscale a canvas/img/video frame to a tiny, already-blurred JPEG data
 *  URL. The blur is baked in here, at thumbnail size, because a CSS blur at
 *  window size is the one thing that re-rasterizes on every resize frame:
 *  a ~2.5px blur at 64px reads as ~30px once scaled up to a window. */
export function glowFromDrawable(source, size = 64) {
  if (!source) return null;
  const w = source.naturalWidth || source.videoWidth || source.width;
  const h = source.naturalHeight || source.videoHeight || source.height;
  if (!w || !h) return null;
  const scale = size / Math.max(w, h);
  const c = document.createElement('canvas');
  c.width = Math.max(1, Math.round(w * scale));
  c.height = Math.max(1, Math.round(h * scale));
  const ctx = c.getContext('2d');
  if (!ctx) return null;
  try {
    // Overscan so the blur's soft (transparent) edges are cropped away.
    const pad = 8;
    if (typeof ctx.filter === 'string') ctx.filter = 'blur(2.5px)';
    ctx.drawImage(source, -pad, -pad, c.width + pad * 2, c.height + pad * 2);
    return `url("${c.toDataURL('image/jpeg', 0.6)}")`;
  } catch (_) {
    return null; // tainted canvas (cross-origin without CORS)
  }
}

/** Cached lightly-blurred backgrounds built from an image URL. */
const imageGlowCache = new Map();

/** Load an image URL, pre-blur it once at a modest size and return a glow CSS
 *  value. Unlike glowFromDrawable this keeps the picture recognisable ("not
 *  too much" blur) while still baking the blur in, so the window pays nothing
 *  per frame. Resolves null when the image can't be used — callers can then
 *  fall back to the raw URL. */
export async function glowFromImageUrl(url, { size = 320, blur = 7 } = {}) {
  if (!url) return null;
  const key = `${url}|${size}|${blur}`;
  if (imageGlowCache.has(key)) return imageGlowCache.get(key);

  const css = await new Promise((resolve) => {
    const img = new Image();
    img.crossOrigin = 'anonymous';
    img.onload = () => {
      const w = img.naturalWidth || 0;
      const h = img.naturalHeight || 0;
      if (!w || !h) return resolve(null);
      const scale = size / Math.max(w, h);
      const c = document.createElement('canvas');
      c.width = Math.max(1, Math.round(w * scale));
      c.height = Math.max(1, Math.round(h * scale));
      const ctx = c.getContext('2d');
      if (!ctx) return resolve(null);
      try {
        const pad = 12; // crop the blur's soft edges
        if (typeof ctx.filter === 'string') ctx.filter = `blur(${blur}px)`;
        ctx.drawImage(img, -pad, -pad, c.width + pad * 2, c.height + pad * 2);
        resolve(`url("${c.toDataURL('image/jpeg', 0.7)}")`);
      } catch (_) {
        resolve(null); // tainted canvas (image without CORS)
      }
    };
    img.onerror = () => resolve(null);
    img.src = url;
  });

  imageGlowCache.set(key, css);
  return css;
}
