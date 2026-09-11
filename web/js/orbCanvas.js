/** Voice orb — canvas only, circular body.
 *
 *  Five selectable looks (`ORB_STYLES` in preferences.js) share one renderer.
 *  The language is deliberately restrained: few large forms, soft falloffs,
 *  slow motion, near-monochrome light taken from the user's accent + gradient.
 *  Palettes re-derive live on `appearance:change`.
 *
 *  Every style reacts to sound through one signal model:
 *
 *    intensity  0..1   how loud the audio is right now (smoothed)
 *    pan       -1..1   -1 hard left … +1 hard right
 *
 *  With a stereo source the body is squashed and pushed away from the loud
 *  side, so the orb leans away from whoever is talking — the microphone while
 *  listening, the assistant's own voice while it answers.
 *
 *  The class owns the clip circle, the outer glow and the glass highlight; a
 *  style only paints inside the body.
 */

import { getAccent, getGradient } from '../ui/index.js';
import { getOrbStyle } from './preferences.js';

const TAU = Math.PI * 2;

/** States that use the compact "square" motion instead of the orbit. */
const SQUARE_STATES = new Set(['listening', 'conversation', 'processing', 'speaking']);

function hexToRgba(hex, a) {
  const h = (hex || '#ffffff').replace('#', '');
  const n = parseInt(h.length === 3 ? h.split('').map((c) => c + c).join('') : h, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  return `rgba(${r},${g},${b},${a})`;
}

function lighten(hex, amount) {
  const h = (hex || '#ffffff').replace('#', '');
  const n = parseInt(h.length === 3 ? h.split('').map((c) => c + c).join('') : h, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  const mix = (c) => Math.min(255, Math.round(c + (255 - c) * amount));
  return `#${[mix(r), mix(g), mix(b)].map((c) => c.toString(16).padStart(2, '0')).join('')}`;
}

/** Per-state orb palettes, derived from the current accent + gradient. */
function derivedPalettes() {
  const accent = getAccent();
  const g = getGradient();
  const stops = g?.stops?.length >= 2 ? g.stops : ['#ffffff', '#8a8a8a'];
  const tail = stops[stops.length - 1];
  const light = lighten(accent, 0.22);
  const pale = lighten(accent, 0.45);
  return {
    idle: [light, accent, tail, stops[0]],
    listening: [lighten(accent, 0.34), light, accent, tail],
    conversation: [light, accent, tail, stops[0]],
    compose: [accent, light, tail, pale],
    processing: [accent, light, tail, pale],
    speaking: [lighten(accent, 0.3), light, accent, tail],
    error: ['#fca5a5', '#f87171', '#fb7185', '#ef4444'],
    downloading: [light, accent, pale, tail],
    disabled: ['#6b6b6b', '#4a4a4a', '#8a8a8a', '#333333'],
  };
}

function paletteForState(state) {
  const palettes = derivedPalettes();
  return [...(palettes[state] || palettes.idle)];
}

/** A soft radial light. `alpha` scales the whole falloff so big shapes stay
 *  translucent and never blow out to white under the screen blend. */
function blob(ctx, w, x, y, r, color, { sharp = false, alpha = 1 } = {}) {
  const g = ctx.createRadialGradient(x, y, 0, x, y, r);
  g.addColorStop(0, hexToRgba(color, alpha));
  if (sharp) {
    g.addColorStop(0.42, hexToRgba(color, 0.72 * alpha));
    g.addColorStop(0.76, hexToRgba(color, 0.22 * alpha));
  } else {
    g.addColorStop(0.38, hexToRgba(color, 0.55 * alpha));
    g.addColorStop(0.72, hexToRgba(color, 0.14 * alpha));
  }
  g.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, w, w);
}

/** A vertical, rotated veil of light — the building block of Nebula. */
function veil(ctx, w, cx, cy, r, color, angle, squash, alpha) {
  ctx.save();
  ctx.translate(cx, cy);
  ctx.rotate(angle);
  ctx.scale(1, squash);
  const g = ctx.createRadialGradient(0, 0, 0, 0, 0, r);
  g.addColorStop(0, hexToRgba(color, alpha * 0.9));
  g.addColorStop(0.5, hexToRgba(color, alpha * 0.45));
  g.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.fillStyle = g;
  ctx.beginPath();
  ctx.arc(0, 0, r, 0, TAU);
  ctx.fill();
  ctx.restore();
}

/* ── Styles ─────────────────────────────────────────────────────
   Each paints inside the clipped body. `p` carries:
   { ctx, w, cx, cy, R, dpr, t, palette, intensity, pan, squareAnim,
     speedMul, scale, glowI }
   ─────────────────────────────────────────────────────────────── */

const STYLES = {
  /** Fluid — one liquid mass of light. Three oversized soft bodies drift
   *  slowly through each other, so the silhouette breathes rather than spins. */
  fluid: {
    draw(p) {
      const { ctx, w, cx, cy, R, palette, t, intensity, squareAnim } = p;
      const energy = Math.min(1, intensity);
      const count = squareAnim ? 4 : 3;
      for (let i = 0; i < count; i++) {
        const phase = (i / count) * TAU;
        const drift = t * (0.14 + i * 0.035);
        const orbit = R * (0.15 + 0.05 * Math.sin(t * 0.35 + i)) * (1 + 0.3 * energy);
        const x = cx + Math.cos(drift + phase) * orbit;
        const y = cy + Math.sin(drift * 1.15 + phase) * orbit;
        const r = R * (0.82 + 0.06 * Math.sin(t * 0.45 + i * 2.1)) * (1 + 0.16 * energy);
        blob(ctx, w, x, y, r, palette[i % palette.length], { alpha: 0.42 });
      }
      // A quiet inner light keeps the centre from going hollow.
      blob(ctx, w, cx, cy, R * (0.7 + 0.08 * energy), palette[0], { alpha: 0.2 + 0.18 * energy });
    },
  },

  /** Ripple — light passing through water. Three soft halos leave the centre
   *  and dissolve; no hard strokes, so it stays glassy at any size. */
  ripple: {
    draw(p) {
      const { ctx, w, cx, cy, R, palette, t, intensity } = p;
      const energy = Math.min(1, intensity);
      const rings = 3;
      const travel = t * (0.15 + 0.16 * energy);
      for (let i = 0; i < rings; i++) {
        const phase = (travel + i / rings) % 1;
        const r = R * (0.3 + phase * (0.92 + 0.3 * energy));
        const a = (1 - phase) ** 1.5 * (0.1 + 0.3 * energy);
        const color = palette[i % palette.length];
        const g = ctx.createRadialGradient(cx, cy, r * 0.66, cx, cy, r);
        g.addColorStop(0, 'rgba(0,0,0,0)');
        g.addColorStop(0.7, hexToRgba(color, a * 0.5));
        g.addColorStop(0.9, hexToRgba(color, a));
        g.addColorStop(1, 'rgba(0,0,0,0)');
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.arc(cx, cy, r, 0, TAU);
        ctx.fill();
      }
      blob(ctx, w, cx, cy, R * (0.62 + 0.12 * energy), palette[0], { alpha: 0.34 + 0.22 * energy });
    },
  },

  /** Nebula — polar veils of colour, like an aurora behind frosted glass.
   *  Broad, slow and almost monochrome at rest; the voice widens them. */
  nebula: {
    draw(p) {
      const { ctx, w, cx, cy, R, palette, t, intensity, squareAnim } = p;
      const energy = Math.min(1, intensity);
      const arms = 3;
      for (let i = 0; i < arms; i++) {
        const angle = t * (0.045 + i * 0.018) * (1 + 0.6 * energy) + (i * TAU) / arms;
        const r = R * (1.16 + 0.06 * Math.sin(t * 0.25 + i));
        const squash = squareAnim ? 0.7 : 0.42 + 0.06 * Math.sin(t * 0.3 + i);
        veil(ctx, w, cx, cy, r, palette[(i + 1) % palette.length], angle, squash, 0.16 + 0.18 * energy);
      }
      blob(ctx, w, cx, cy, R * (0.66 + 0.1 * energy), palette[0], { alpha: 0.28 + 0.22 * energy });
    },
  },

  /** Pulse — a breathing core inside two tilted orbits with one satellite.
   *  Calm at rest, and the orbit tightens as the voice rises. */
  pulse: {
    draw(p) {
      const { ctx, w, cx, cy, R, palette, t, intensity, dpr } = p;
      const energy = Math.min(1, intensity);
      const breath = 0.5 + 0.5 * Math.sin(t * 1.05);

      blob(ctx, w, cx, cy, R * (0.78 + 0.1 * breath + 0.14 * energy), palette[0], {
        alpha: 0.34 + 0.2 * energy,
      });

      for (let i = 0; i < 2; i++) {
        ctx.save();
        ctx.translate(cx, cy);
        ctx.rotate(t * (0.16 + i * 0.1) * (1 + 0.5 * energy) + i * 1.15);
        ctx.scale(1, 0.36 + i * 0.14);
        ctx.strokeStyle = hexToRgba(palette[(i + 2) % palette.length], 0.16 + 0.3 * energy);
        ctx.lineWidth = (0.9 + 1.3 * energy) * dpr;
        ctx.beginPath();
        ctx.arc(0, 0, R * (0.7 + 0.06 * breath), 0, TAU);
        ctx.stroke();
        ctx.restore();
      }

      const a = t * (0.42 + 0.5 * energy);
      const orbit = R * (0.7 + 0.05 * breath);
      blob(ctx, w, cx + Math.cos(a) * orbit, cy + Math.sin(a) * orbit * 0.44, R * 0.19, palette[1], {
        sharp: true,
        alpha: 0.8,
      });
    },
  },

  /** Prism — a cut stone. Six broad facets with hairline edges and a light
   *  trapped in the middle; it turns slowly and refracts more when loud. */
  prism: {
    draw(p) {
      const { ctx, cx, cy, R, palette, t, intensity, dpr } = p;
      const energy = Math.min(1, intensity);
      const facets = 6;
      ctx.save();
      ctx.translate(cx, cy);
      ctx.rotate(t * 0.07 * (1 + 0.7 * energy));
      const spread = R * (0.82 + 0.14 * energy);
      for (let i = 0; i < facets; i++) {
        const a0 = (i / facets) * TAU;
        const a1 = ((i + 1) / facets) * TAU;
        const mid = (a0 + a1) / 2;
        const color = palette[i % palette.length];
        const g = ctx.createLinearGradient(0, 0, Math.cos(mid) * spread, Math.sin(mid) * spread);
        g.addColorStop(0, hexToRgba(color, 0.03));
        g.addColorStop(1, hexToRgba(color, 0.16 + 0.24 * energy));
        ctx.beginPath();
        ctx.moveTo(Math.cos(a0) * spread, Math.sin(a0) * spread);
        ctx.lineTo(Math.cos(a1) * spread, Math.sin(a1) * spread);
        ctx.lineTo(Math.cos(mid) * spread * 0.2, Math.sin(mid) * spread * 0.2);
        ctx.closePath();
        ctx.fillStyle = g;
        ctx.fill();
        ctx.strokeStyle = hexToRgba(color, 0.1 + 0.2 * energy);
        ctx.lineWidth = 0.7 * dpr;
        ctx.stroke();
      }
      const core = ctx.createRadialGradient(0, 0, 0, 0, 0, R * (0.5 + 0.14 * energy));
      core.addColorStop(0, hexToRgba(palette[0], 0.45 + 0.3 * energy));
      core.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.fillStyle = core;
      ctx.beginPath();
      ctx.arc(0, 0, R * (0.5 + 0.14 * energy), 0, TAU);
      ctx.fill();
      ctx.restore();
    },
  },
};

const DEFAULT_STYLE = 'fluid';

class OrbRenderer {
  constructor(canvas, { size = 84, preview = false, style = null } = {}) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d', { alpha: true });
    this.displaySize = size;
    this.preview = preview;
    this.palette = paletteForState('idle');
    this.stateKey = 'idle';
    this.animationMode = 'orbit';
    this.styleId = STYLES[style] ? style : DEFAULT_STYLE;

    // Signal — smoothed toward the audio targets every frame.
    this.intensity = 0;
    this.pan = 0;
    this.targetLevel = 0;
    this.targetPan = 0;

    this.t = 0;
    this.running = true;
    this.lastDrawAt = 0;
    // Previews are decoration; the real orb runs at full frame rate.
    this.minFrameMs = preview ? 1000 / 24 : 0;

    this.resize();
    this._onResize = () => this.resize();
    window.addEventListener('resize', this._onResize);
    this._loop = this._loop.bind(this);
    requestAnimationFrame(this._loop);
  }

  resize() {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const px = Math.round(this.displaySize * dpr);
    this.canvas.width = px;
    this.canvas.height = px;
    this.canvas.style.width = `${this.displaySize}px`;
    this.canvas.style.height = `${this.displaySize}px`;
    this.px = px;
    this.dpr = dpr;
  }

  setStyle(id) {
    this.styleId = STYLES[id] ? id : DEFAULT_STYLE;
  }

  getStyle() {
    return this.styleId;
  }

  setPalette(state) {
    this.stateKey = state;
    this.palette = paletteForState(state);
    this.animationMode = SQUARE_STATES.has(state) ? 'square' : 'orbit';
  }

  refreshPalette() {
    this.setPalette(this.stateKey);
  }

  /** Accepts a plain level (legacy) or `{ level, pan }` from voice.js. */
  setSignal(v) {
    if (typeof v === 'number') {
      this.targetLevel = Math.min(1, Math.max(0, v));
      return;
    }
    if (v && typeof v === 'object') {
      const level = Number(v.level);
      const pan = Number(v.pan);
      if (Number.isFinite(level)) this.targetLevel = Math.min(1, Math.max(0, level));
      if (Number.isFinite(pan)) this.targetPan = Math.min(1, Math.max(-1, pan));
    }
  }

  destroy() {
    this.running = false;
    window.removeEventListener('resize', this._onResize);
  }

  _loop(ts) {
    if (!this.running) return;
    if (this.minFrameMs && ts - this.lastDrawAt < this.minFrameMs) {
      requestAnimationFrame(this._loop);
      return;
    }
    this.lastDrawAt = ts;
    this.t = ts * 0.001;

    if (this.preview) {
      // Synthesize a voice so every style shows off in the settings grid.
      this.targetLevel = 0.4 + 0.36 * Math.sin(this.t * 1.15);
      this.targetPan = Math.sin(this.t * 0.42);
    }
    // Fast attack, slow release: the orb leaps on a syllable and settles.
    const k = this.targetLevel > this.intensity ? 0.45 : 0.1;
    this.intensity += (this.targetLevel - this.intensity) * k;
    this.pan += (this.targetPan - this.pan) * 0.16;

    this.draw();
    requestAnimationFrame(this._loop);
  }

  /** Move + squash the body away from the loud side of a stereo signal. */
  _applyBodyTransform(ctx, cx, cy, R) {
    const energy = this.intensity;
    const side = Math.abs(this.pan);
    const squish = Math.min(0.42, side * (0.22 + 0.5 * energy));
    const sx = 1 - squish;
    const sy = 1 + squish * 0.45;
    const shift = -this.pan * R * (0.1 + 0.2 * energy);
    ctx.translate(cx + shift, cy);
    ctx.scale(sx, sy);
    ctx.translate(-cx, -cy);
  }

  draw() {
    const { ctx, px: w } = this;
    const cx = w / 2;
    const cy = w / 2;
    const R = w * 0.44;
    const state = this.stateKey;
    const stateBoost =
      state === 'listening' || state === 'conversation' ? 0.18
        : state === 'processing' || state === 'speaking' || state === 'compose' ? 0.14
          : 0;
    const scale = 1 + Math.min(1, this.intensity + stateBoost) * 0.4;
    const glowI = Math.min(1, this.intensity + stateBoost);
    const squareAnim = this.animationMode === 'square';
    const speedMul =
      state === 'processing' ? 1.25
        : state === 'listening' || state === 'conversation' || state === 'compose' ? 1.12
          : state === 'speaking' ? 1.08
            : 1;

    ctx.clearRect(0, 0, w, w);
    ctx.save();
    this._applyBodyTransform(ctx, cx, cy, R);

    // Outer glow — a soft halo that brightens with the voice.
    ctx.save();
    ctx.shadowColor = hexToRgba(this.palette[0], 0.4 + glowI * 0.22);
    ctx.shadowBlur = (squareAnim ? 6 + glowI * 5 : 12 + glowI * 16) * this.dpr;
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, TAU);
    ctx.fillStyle = 'rgba(0,0,0,0.004)';
    ctx.fill();
    ctx.restore();

    // Body — transparent (the desktop shows through), light screen-blended.
    ctx.save();
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, TAU);
    ctx.clip();
    ctx.globalCompositeOperation = 'screen';

    const params = {
      ctx, w, cx, cy, R,
      dpr: this.dpr,
      t: this.t,
      palette: this.palette,
      intensity: this.intensity,
      pan: this.pan,
      stateKey: state,
      squareAnim,
      speedMul,
      scale,
      glowI,
    };
    (STYLES[this.styleId] || STYLES[DEFAULT_STYLE]).draw(params);

    ctx.globalCompositeOperation = 'source-over';

    // Shared glass: top-left sheen + a dark rim so the body reads as a sphere.
    const highlight = ctx.createRadialGradient(cx - R * 0.28, cy - R * 0.32, 0, cx, cy, R * 0.98);
    highlight.addColorStop(0, 'rgba(255,255,255,0.2)');
    highlight.addColorStop(0.38, 'rgba(255,255,255,0.035)');
    highlight.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.fillStyle = highlight;
    ctx.fillRect(0, 0, w, w);

    const edge = ctx.createRadialGradient(cx, cy, R * 0.58, cx, cy, R);
    edge.addColorStop(0, 'rgba(0,0,0,0)');
    edge.addColorStop(0.86, 'rgba(0,0,0,0.07)');
    edge.addColorStop(1, 'rgba(0,0,0,0.18)');
    ctx.fillStyle = edge;
    ctx.fillRect(0, 0, w, w);
    ctx.restore();

    // Escaping ripples when the voice is loud — kept faint so they read as
    // pressure in the air, not as a second animation.
    if (glowI > 0.14 && !squareAnim) {
      const rippleT = (this.t * 1.5) % 1;
      for (let i = 0; i < 2; i++) {
        const p = (rippleT + i * 0.5) % 1;
        const rr = R + p * R * 0.24;
        const g = ctx.createRadialGradient(cx, cy, rr * 0.88, cx, cy, rr);
        g.addColorStop(0, hexToRgba(this.palette[0], (1 - p) * 0.08));
        g.addColorStop(1, 'rgba(0,0,0,0)');
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.arc(cx, cy, rr, 0, TAU);
        ctx.fill();
      }
    }

    ctx.restore();
  }
}

let renderer = null;

export function initOrbCanvas(canvas) {
  if (renderer) renderer.destroy();
  renderer = new OrbRenderer(canvas, { style: getOrbStyle() });
  window.addEventListener('appearance:change', () => renderer?.refreshPalette());
  return renderer;
}

export function setOrbPalette(state) {
  renderer?.setPalette(state);
}

export function setOrbStyle(id) {
  renderer?.setStyle(id);
}

export function getOrbStyleId() {
  return renderer?.getStyle() || DEFAULT_STYLE;
}

export function setOrbIntensity(v) {
  renderer?.setSignal(v);
}

export function resetOrbIntensity() {
  renderer?.setSignal({ level: 0, pan: 0 });
}

/** A self-animating preview for the settings page (one per style card). */
export function createOrbPreview(canvas, styleId, size = 52) {
  return new OrbRenderer(canvas, { size, preview: true, style: styleId });
}
