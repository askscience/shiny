/** Voice orb — canvas only, circular body.
 *
 *  Five selectable looks (`ORB_STYLES` in preferences.js) share one renderer:
 *  palettes derive live from the user's accent + gradient, and every style
 *  reacts to the microphone. When the input is stereo the body is *pushed and
 *  squashed from the loud side*, so the orb visibly leans away from whoever is
 *  talking. All styles reuse the same signal model:
 *
 *    intensity  0..1   how loud the mic is right now (smoothed)
 *    pan       -1..1   -1 hard left … +1 hard right
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
    idle: [accent, tail, light, stops[0]],
    listening: [light, accent, pale, tail],
    conversation: [accent, tail, light, stops[0]],
    compose: [accent, light, tail, pale],
    processing: [accent, light, tail, pale],
    speaking: [light, accent, tail, pale],
    error: ['#fca5a5', '#f87171', '#fb7185', '#ef4444'],
    downloading: [light, accent, pale, tail],
    disabled: ['#6b6b6b', '#4a4a4a', '#8a8a8a', '#333333'],
  };
}

function paletteForState(state) {
  const palettes = derivedPalettes();
  return [...(palettes[state] || palettes.idle)];
}

/** One soft radial blob. `sharp` tightens the falloff for faceted styles. */
function blob(ctx, w, x, y, r, color, sharp = false) {
  const midA = sharp ? 0.88 : 0.65;
  const outerA = sharp ? 0.28 : 0.15;
  const g = ctx.createRadialGradient(x, y, 0, x, y, r);
  g.addColorStop(0, color);
  if (sharp) {
    g.addColorStop(0.45, hexToRgba(color, midA));
    g.addColorStop(0.78, hexToRgba(color, outerA));
  } else {
    g.addColorStop(0.35, hexToRgba(color, midA));
    g.addColorStop(0.7, hexToRgba(color, outerA));
  }
  g.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, w, w);
}

/* ── Styles ─────────────────────────────────────────────────────
   Each style paints inside the clipped body. `p` carries:
   { ctx, w, cx, cy, R, dpr, t, palette, intensity, pan, squareAnim,
     speedMul, scale, glowI, blobs }
   ─────────────────────────────────────────────────────────────── */

const STYLES = {
  /** The classic: four orbiting blobs of light. */
  fluid: {
    draw(p) {
      const { ctx, w, cx, cy, palette, t, blobs, squareAnim, speedMul, scale } = p;
      if (squareAnim) {
        const d = p.R * 0.24 * scale;
        const rot = t * 0.65 * speedMul;
        const corners = [[-1, -1], [1, -1], [1, 1], [-1, 1]];
        for (let i = 0; i < blobs.length; i++) {
          const [sx, sy] = corners[i];
          const cos = Math.cos(rot);
          const sin = Math.sin(rot);
          const x = cx + (sx * cos - sy * sin) * d;
          const y = cy + (sx * sin + sy * cos) * d;
          const r = blobs[i].radius * w * scale * 0.42;
          blob(ctx, w, x, y, r, palette[i % palette.length], true);
        }
        return;
      }
      for (let i = 0; i < blobs.length; i++) {
        const b = blobs[i];
        const angle = t * b.speed * speedMul + b.phase;
        const wobble = Math.sin(t * 1.7 + b.phase * 2) * 0.06 * w;
        const orbit = b.orbit * w * scale;
        const x = cx + Math.cos(angle) * orbit + wobble;
        const y = cy + Math.sin(angle * 1.25 + b.phase) * orbit - wobble * 0.5;
        const r = b.radius * w * scale * (0.92 + Math.sin(t * 2 + i) * 0.08);
        blob(ctx, w, x, y, r, palette[i % palette.length]);
      }
      // Warm swirl that ties the blobs into one body.
      ctx.globalAlpha = 0.55;
      const swirl = ctx.createRadialGradient(
        cx + Math.cos(t * 0.4) * p.R * 0.35,
        cy + Math.sin(t * 0.35) * p.R * 0.35,
        0,
        cx,
        cy,
        p.R * 1.1,
      );
      swirl.addColorStop(0, hexToRgba(palette[2], 0.5));
      swirl.addColorStop(0.5, hexToRgba(palette[1], 0.2));
      swirl.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.fillStyle = swirl;
      ctx.fillRect(0, 0, w, w);
      ctx.globalAlpha = 1;
    },
  },

  /** Concentric waves that leave the body as the voice rises. */
  ripple: {
    draw(p) {
      const { ctx, cx, cy, R, palette, t, intensity, squareAnim, dpr } = p;
      const energy = Math.min(1, intensity);
      const rings = squareAnim ? 4 : 6;
      const emit = t * (0.28 + 0.55 * energy);
      for (let i = 0; i < rings; i++) {
        const phase = (emit + i / rings) % 1;
        const r = R * (0.16 + phase * (0.95 + 0.35 * energy));
        const fade = (1 - phase) * (0.16 + 0.5 * energy);
        const color = palette[i % palette.length];
        ctx.strokeStyle = hexToRgba(color, Math.min(0.85, fade));
        ctx.lineWidth = (1 + 2.6 * energy) * dpr;
        ctx.beginPath();
        ctx.arc(cx, cy, r, 0, TAU);
        ctx.stroke();
      }
      // Bright core that swells with the voice.
      const coreR = R * (0.34 + 0.22 * energy);
      const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, coreR * 1.9);
      g.addColorStop(0, hexToRgba(palette[0], 0.95));
      g.addColorStop(0.35, hexToRgba(palette[1], 0.45 + 0.35 * energy));
      g.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.fillStyle = g;
      ctx.fillRect(0, 0, p.w, p.w);
    },
  },

  /** Slow drifting clouds of colour, like a nebula seen through glass. */
  nebula: {
    draw(p) {
      const { ctx, w, cx, cy, R, palette, t, intensity, squareAnim } = p;
      const energy = Math.min(1, intensity);
      const clouds = 6;
      for (let i = 0; i < clouds; i++) {
        const a = t * (0.1 + i * 0.028) * (1 + 0.8 * energy) + i * 1.7;
        const orbit = R * (0.3 + 0.22 * Math.sin(t * 0.3 + i)) * (1 + 0.35 * energy);
        const x = cx + Math.cos(a) * orbit;
        const y = cy + Math.sin(a * 0.8 + i) * orbit * (squareAnim ? 0.9 : 1);
        const r = R * (0.55 + 0.16 * Math.sin(t * 0.5 + i * 2)) * (1 + 0.4 * energy);
        blob(ctx, w, x, y, r, palette[i % palette.length], true);
      }
      // Dust: a handful of tiny bright motes drifting the other way.
      for (let i = 0; i < 10; i++) {
        const a = -t * (0.2 + i * 0.01) + i * 2.4;
        const orbit = R * (0.35 + 0.4 * ((i * 37) % 100) / 100);
        const x = cx + Math.cos(a) * orbit;
        const y = cy + Math.sin(a * 1.3 + i) * orbit;
        const s = (0.6 + 1.4 * energy) * p.dpr;
        ctx.fillStyle = hexToRgba(palette[(i + 1) % palette.length], 0.25 + 0.4 * energy);
        ctx.beginPath();
        ctx.arc(x, y, s, 0, TAU);
        ctx.fill();
      }
    },
  },

  /** A heartbeat core with satellites — doubles up when you speak. */
  pulse: {
    draw(p) {
      const { ctx, w, cx, cy, R, palette, t, intensity, squareAnim, dpr } = p;
      const energy = Math.min(1, intensity);
      const period = 1.15;
      const ph = (t % period) / period;
      const thump =
        Math.exp(-(((ph - 0.06) / 0.05) ** 2)) + 0.55 * Math.exp(-(((ph - 0.2) / 0.07) ** 2));
      const beat = thump * (0.3 + 0.7 * energy);
      const coreR = R * (0.36 + 0.2 * beat + 0.12 * energy);

      const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, coreR * 2.4);
      g.addColorStop(0, hexToRgba(palette[0], 0.95));
      g.addColorStop(0.3, hexToRgba(palette[1], 0.45 + 0.35 * beat));
      g.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.fillStyle = g;
      ctx.fillRect(0, 0, w, w);

      const n = squareAnim ? 4 : 3;
      for (let i = 0; i < n; i++) {
        const a = t * (0.9 + 0.7 * energy) + (i * TAU) / n;
        const orbit = R * (0.6 + 0.12 * Math.sin(t * 1.3 + i)) * (1 + 0.3 * energy);
        const x = cx + Math.cos(a) * orbit;
        const y = cy + Math.sin(a) * orbit * (squareAnim ? 0.75 : 0.62);
        blob(ctx, w, x, y, R * 0.2, palette[(i + 2) % palette.length], true);
      }
      // ECG-ish tick that sharpens with the voice.
      ctx.strokeStyle = hexToRgba(palette[0], 0.18 + 0.4 * energy);
      ctx.lineWidth = 1.2 * dpr;
      ctx.beginPath();
      const span = R * 1.5;
      const trace = (u) => cy + Math.sin(u * TAU * 3 + t * 2) * R * 0.06 * (0.4 + energy);
      for (let i = 0; i <= 48; i++) {
        const u = i / 48;
        const x = cx - span / 2 + u * span;
        if (i === 0) ctx.moveTo(x, trace(u));
        else ctx.lineTo(x, trace(u));
      }
      ctx.stroke();
    },
  },

  /** Rotating facets — light bends through the orb in shards. */
  prism: {
    draw(p) {
      const { ctx, cx, cy, R, palette, t, intensity, squareAnim, dpr } = p;
      const energy = Math.min(1, intensity);
      const facets = squareAnim ? 4 : 7;
      ctx.save();
      ctx.translate(cx, cy);
      ctx.rotate(t * 0.22 * (1 + 0.7 * energy));
      for (let i = 0; i < facets; i++) {
        const a0 = (i / facets) * TAU;
        const a1 = ((i + 1) / facets) * TAU;
        const spread = R * (0.42 + 0.34 * energy + 0.06 * Math.sin(t * 2 + i));
        const color = palette[i % palette.length];
        const g = ctx.createLinearGradient(0, 0, Math.cos(a0) * spread, Math.sin(a0) * spread);
        g.addColorStop(0, hexToRgba(color, 0.04));
        g.addColorStop(0.6, hexToRgba(color, 0.3 + 0.3 * energy));
        g.addColorStop(1, hexToRgba(color, 0.62 + 0.28 * energy));
        const mid = (a0 + a1) / 2;
        ctx.beginPath();
        ctx.moveTo(Math.cos(a0) * spread, Math.sin(a0) * spread);
        ctx.lineTo(Math.cos(a1) * spread, Math.sin(a1) * spread);
        ctx.lineTo(Math.cos(mid) * spread * 0.14, Math.sin(mid) * spread * 0.14);
        ctx.closePath();
        ctx.fillStyle = g;
        ctx.fill();
        ctx.strokeStyle = hexToRgba(color, 0.2 + 0.25 * energy);
        ctx.lineWidth = 1 * dpr;
        ctx.stroke();
      }
      // Hot centre where the facets meet.
      const core = ctx.createRadialGradient(0, 0, 0, 0, 0, R * (0.3 + 0.25 * energy));
      core.addColorStop(0, hexToRgba(palette[0], 0.85));
      core.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.fillStyle = core;
      ctx.fillRect(-R * 2, -R * 2, R * 4, R * 4);
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

    // Signal — smoothed toward the microphone targets every frame.
    this.intensity = 0;
    this.pan = 0;
    this.targetLevel = 0;
    this.targetPan = 0;

    this.t = 0;
    this.running = true;
    this.lastDrawAt = 0;
    // Previews are decoration; the main orb runs at full frame rate.
    this.minFrameMs = preview ? 1000 / 24 : 0;

    this.blobs = [
      { phase: 0, speed: 0.9, orbit: 0.22, radius: 0.55 },
      { phase: 2.1, speed: 1.15, orbit: 0.18, radius: 0.48 },
      { phase: 4.2, speed: 0.75, orbit: 0.26, radius: 0.52 },
      { phase: 1.3, speed: 1.05, orbit: 0.2, radius: 0.44 },
    ];

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
      this.targetLevel = 0.42 + 0.38 * Math.sin(this.t * 1.35);
      this.targetPan = Math.sin(this.t * 0.5);
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
      state === 'listening' || state === 'conversation' ? 0.22
        : state === 'processing' || state === 'speaking' || state === 'compose' ? 0.18
          : 0;
    const scale = 1 + Math.min(1, this.intensity + stateBoost) * 0.45;
    const glowI = Math.min(1, this.intensity + stateBoost);
    const squareAnim = this.animationMode === 'square';
    const speedMul =
      state === 'processing' ? 1.35
        : state === 'listening' || state === 'conversation' || state === 'compose' ? 1.2
          : state === 'speaking' ? 1.15
            : 1;

    ctx.clearRect(0, 0, w, w);
    ctx.save();
    this._applyBodyTransform(ctx, cx, cy, R);

    // Outer glow — a soft halo that brightens with the voice.
    ctx.save();
    ctx.shadowColor = hexToRgba(this.palette[0], 0.55 + glowI * 0.25);
    ctx.shadowBlur = (squareAnim ? 6 + glowI * 6 : 14 + glowI * 18) * this.dpr;
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, TAU);
    ctx.fillStyle = 'rgba(0,0,0,0.004)';
    ctx.fill();
    ctx.restore();

    // Body — transparent (the desktop shows through), blobs screen-blended.
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
      blobs: this.blobs,
    };
    (STYLES[this.styleId] || STYLES[DEFAULT_STYLE]).draw(params);

    ctx.globalCompositeOperation = 'source-over';

    // Shared glass: top-left sheen + a dark rim so the body reads as a sphere.
    const highlight = ctx.createRadialGradient(cx - R * 0.25, cy - R * 0.3, 0, cx, cy, R * 0.95);
    highlight.addColorStop(0, 'rgba(255,255,255,0.22)');
    highlight.addColorStop(0.35, 'rgba(255,255,255,0.04)');
    highlight.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.fillStyle = highlight;
    ctx.fillRect(0, 0, w, w);

    const edge = ctx.createRadialGradient(cx, cy, R * 0.55, cx, cy, R);
    edge.addColorStop(0, 'rgba(0,0,0,0)');
    edge.addColorStop(0.85, 'rgba(0,0,0,0.08)');
    edge.addColorStop(1, 'rgba(0,0,0,0.2)');
    ctx.fillStyle = edge;
    ctx.fillRect(0, 0, w, w);
    ctx.restore();

    // Escaping ripples when the voice is loud.
    if (glowI > 0.12 && !squareAnim) {
      const rippleT = (this.t * 1.8) % 1;
      for (let i = 0; i < 2; i++) {
        const p = (rippleT + i * 0.5) % 1;
        const rr = R + p * R * 0.28;
        const g = ctx.createRadialGradient(cx, cy, rr * 0.85, cx, cy, rr);
        g.addColorStop(0, hexToRgba(this.palette[0], (1 - p) * 0.12));
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
