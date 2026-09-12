/** Voice orb — canvas only, circular body.
 *
 *  The fallback for `orbCanvas3d.js`, and the same five looks: every one is a
 *  variant of the reference ring in `web/orbs/eclipse.jpg` — a thin ring of
 *  spectral light on black.
 *
 *    corona — the hairline ring throwing fine sparks outward     (default)
 *    halo   — the ring itself rippling as a smooth radial wave
 *    ripple — echoes that spread outward from the ring
 *    flare  — a crown of long rays growing out of the ring
 *    aura   — two rings riding a wave in and out of the screen
 *
 *  Nothing rotates and the ring always faces you. Sound is what moves the
 *  light, through one signal model shared with the WebGL renderer:
 *
 *    intensity  0..1   how loud the audio is right now (smoothed)
 *    pan       -1..1   -1 hard left … +1 hard right
 *
 *  With a stereo source the body is squashed and pushed away from the loud
 *  side, so the orb leans away from whoever is talking. Ring colours come from
 *  `spectrumForState()`, so the fan follows the user's accent (and falls back
 *  to red / grey on error and disabled).
 */

import { spectrumForState, isLightCanvas } from './orbPalette.js';
import { DEFAULT_ORB_STYLE } from './preferences.js';

const TAU = Math.PI * 2;

function hexToRgba(hex, a) {
  const h = (hex || '#ffffff').replace('#', '');
  const n = parseInt(h.length === 3 ? h.split('').map((c) => c + c).join('') : h, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  return `rgba(${r},${g},${b},${a})`;
}

/* ── ring painting ─────────────────────────────────────────────── */

/**
 * Stroke the coloured ring: the spectrum runs once around it, and an optional
 * wave modulates the radius so the ring can move with the voice.
 *
 * `width` is in canvas pixels, like every other length here — the canvas is
 * already backed at the device pixel ratio, so scaling a width by `dpr` again
 * would make the ring's weight depend on the display.
 */
function coloredRing(ctx, cx, cy, radius, width, spectrum, opts = {}) {
  const {
    alpha = 1, waves = 0, amp = 0, phase = 0, ribbon = 0,
    // A flat ring needs fewer segments than a waved one to stay smooth.
    steps = waves ? 180 : 110,
  } = opts;
  ctx.lineCap = 'butt';
  ctx.lineWidth = Math.max(0.5, width);
  const span = (TAU / steps) * 1.7; // overlap, so the ring never shows gaps
  for (let i = 0; i < steps; i++) {
    const a = (i / steps) * TAU;
    const mid = a + span * 0.5;
    const r = radius * (1 + amp * Math.sin(waves * mid + phase));
    // `ribbon` thins and dims the parts of the ring that have twisted away
    // from you, which is what sells the out-of-plane wave in 2D.
    const shade = ribbon ? 1 - ribbon * (1 - Math.cos(waves * mid + phase)) * 0.5 : 1;
    ctx.lineWidth = Math.max(0.5, ribbon ? width * (0.4 + 0.6 * shade) : width);
    ctx.strokeStyle = hexToRgba(spectrum[i % spectrum.length], Math.min(1, alpha * shade));
    ctx.beginPath();
    ctx.arc(cx, cy, r, a, a + span);
    ctx.stroke();
  }
}

/** The ring body: a soft halo pass behind a bright hairline at one radius. */
function ringBody(ctx, cx, cy, radius, spectrum, opts = {}) {
  const { alpha = 1, waves = 0, amp = 0, phase = 0, ribbon = 0, soft = 0.055, core = 0.017 } = opts;
  coloredRing(ctx, cx, cy, radius, radius * soft, spectrum, {
    alpha: alpha * 0.22, waves, amp, phase, ribbon,
  });
  coloredRing(ctx, cx, cy, radius, radius * core, spectrum, {
    alpha, waves, amp, phase, ribbon,
  });
}

/** One straight ray of light running outward from `radius`. */
function ray(ctx, cx, cy, th, radius, len, width, color, alpha) {
  ctx.strokeStyle = hexToRgba(color, Math.min(1, alpha));
  ctx.lineWidth = Math.max(0.5, width);
  ctx.beginPath();
  ctx.moveTo(cx + Math.cos(th) * radius, cy + Math.sin(th) * radius);
  ctx.lineTo(cx + Math.cos(th) * (radius + len), cy + Math.sin(th) * (radius + len));
  ctx.stroke();
}

/* ── Styles ─────────────────────────────────────────────────────
   Each paints inside the clipped body. `p` carries:
   { ctx, w, cx, cy, R, t, intensity, pan, stateKey }
   ─────────────────────────────────────────────────────────────── */

const STYLES = {
  /** Corona — the reference ring, throwing fine sparks outward on sound. */
  corona: {
    draw(p) {
      const { ctx, cx, cy, R, t, intensity, stateKey } = p;
      const energy = Math.min(1, intensity);
      const spectrum = spectrumForState(stateKey, 12);
      const radius = R * 0.84;

      ringBody(ctx, cx, cy, radius, spectrum);
      if (energy < 0.02) return;

      ctx.lineCap = 'round';
      const count = 64;
      for (let i = 0; i < count; i++) {
        const th = (i / count) * TAU;
        const flicker = 0.5 + 0.5 * Math.sin(t * 2.6 + i * 1.9);
        ray(
          ctx, cx, cy, th, radius,
          R * 0.24 * energy * flicker,
          R * 0.014,
          spectrum[i % spectrum.length],
          energy * 1.6 * flicker,
        );
      }
    },
  },

  /** Halo — the ring itself rippling as a smooth radial wave. */
  halo: {
    draw(p) {
      const { ctx, cx, cy, R, t, intensity, stateKey } = p;
      const energy = Math.min(1, intensity);
      const spectrum = spectrumForState(stateKey, 12);
      const radius = R * 0.82;
      const amp = 0.06 * energy;
      const phase = t * 1.7;

      ringBody(ctx, cx, cy, radius, spectrum, {
        waves: 9, amp, phase, soft: 0.075, core: 0.019,
      });
    },
  },

  /** Ripple — echoes that spread outward from the ring while you speak. */
  ripple: {
    draw(p) {
      const { ctx, cx, cy, R, t, intensity, stateKey } = p;
      const energy = Math.min(1, intensity);
      const spectrum = spectrumForState(stateKey, 12);
      const radius = R * 0.82;

      ringBody(ctx, cx, cy, radius, spectrum);
      if (energy < 0.02) return;

      for (let i = 0; i < 3; i++) {
        const f = (t * 0.5 + i / 3) % 1;
        coloredRing(ctx, cx, cy, radius * (1 + f * 0.45), R * 0.013, spectrum, {
          alpha: energy * (1 - f) * 0.85,
        });
      }
    },
  },

  /** Flare — a crown of long rays growing out of the ring. */
  flare: {
    draw(p) {
      const { ctx, cx, cy, R, intensity, stateKey } = p;
      const energy = Math.min(1, intensity);
      const spectrum = spectrumForState(stateKey, 12);
      const radius = R * 0.8;

      ringBody(ctx, cx, cy, radius, spectrum, { soft: 0.06, core: 0.018 });
      if (energy < 0.02) return;

      ctx.lineCap = 'round';
      const count = 14;
      for (let i = 0; i < count; i++) {
        ray(
          ctx, cx, cy, (i / count) * TAU, radius,
          R * 0.55 * energy,
          R * 0.055 * (0.3 + 0.7 * energy),
          spectrum[i % spectrum.length],
          energy * 1.2,
        );
      }
    },
  },

  /** Aura — two rings riding a wave in and out of the screen, counter-phase. */
  aura: {
    draw(p) {
      const { ctx, cx, cy, R, t, intensity, stateKey } = p;
      const energy = Math.min(1, intensity);
      const spectrum = spectrumForState(stateKey, 12);
      const phase = t * 1.2;

      for (const [radius, offset] of [[R * 0.72, 0], [R * 0.93, Math.PI]]) {
        ringBody(ctx, cx, cy, radius, spectrum, {
          waves: 3, amp: 0.045 * energy, phase: phase + offset, ribbon: 1,
          soft: 0.05, core: 0.019,
        });
      }
    },
  },
};

const DEFAULT_STYLE = DEFAULT_ORB_STYLE;

class OrbRenderer {
  constructor(canvas, { size = 84, preview = false, style = null } = {}) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d', { alpha: true });
    this.displaySize = size;
    this.preview = preview;
    this.stateKey = 'idle';
    this.styleId = STYLES[style] ? style : DEFAULT_STYLE;
    // Screen blending adds light, which disappears on paper; a light canvas
    // composites normally so the ink stays ink.
    this.screen = !isLightCanvas();

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

  /** The state drives the palette family (including the red / grey error
   *  signal); the accent itself is read live through `spectrumForState`. */
  setPalette(state) {
    this.stateKey = state;
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
      // A gentle synthetic voice, always centred — oscillating pan made the
      // settings cards look like they were being squeezed.
      this.targetLevel = 0.36 + 0.22 * Math.sin(this.t * 0.9);
      this.targetPan = 0;
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

    // Waiting for the assistant: there is no voice to react to, so the orb runs
    // its own slow rhythm (the ring breathes and each style's own gesture keeps
    // moving) and pulses keep leaving it. Shared by every look.
    const thinking = this.stateKey === 'processing';
    const rhythm = Math.sin(this.t * 2.4);
    const drive = thinking ? Math.max(this.intensity, 0.34 + 0.16 * rhythm) : this.intensity;

    ctx.clearRect(0, 0, w, w);
    ctx.save();
    this._applyBodyTransform(ctx, cx, cy, R);

    // The body stays transparent (the desktop shows through); on a dark canvas
    // the ring is screen-blended so overlapping passes add up to light.
    ctx.save();
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, TAU);
    ctx.clip();
    ctx.globalCompositeOperation = this.screen ? 'screen' : 'source-over';

    // The whole orb breathes while it thinks — one alpha for every pass.
    ctx.globalAlpha = thinking ? 0.86 + 0.14 * rhythm : 1;

    (STYLES[this.styleId] || STYLES[DEFAULT_STYLE]).draw({
      ctx, w, cx, cy, R,
      t: this.t,
      intensity: drive,
      pan: this.pan,
      stateKey: this.stateKey,
    });

    ctx.globalAlpha = 1;
    if (thinking) this._drawPulses(ctx, cx, cy, R);

    ctx.globalCompositeOperation = 'source-over';
    ctx.restore();
    ctx.restore();
  }

  /** The "thinking" pulses: soft rings that leave the orb and fade, so the orb
   *  reads as busy while the assistant works. Silent at every other state. */
  _drawPulses(ctx, cx, cy, R) {
    const spectrum = spectrumForState(this.stateKey, 12);
    for (let i = 0; i < 2; i++) {
      const f = (this.t * 0.5 + i / 2) % 1;
      coloredRing(ctx, cx, cy, R * 0.52 * (1 + f * 0.88), R * 0.012, spectrum, {
        alpha: (1 - f) * (1 - f) * 0.55,
      });
    }
  }
}

let renderer = null;

/** Generic factory used by the orb facade (this module is the fallback). */
export function createRenderer(canvas, opts = {}) {
  return new OrbRenderer(canvas, opts);
}

export function initOrbCanvas(canvas) {
  if (renderer) renderer.destroy();
  renderer = new OrbRenderer(canvas, {});
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
