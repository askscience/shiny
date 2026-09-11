/** Voice orb — canvas only, circular body.
 *
 *  Five selectable looks (`ORB_STYLES` in preferences.js) share one renderer.
 *  The language comes from the reference images: light structures suspended in
 *  a glass sphere — filament trails, a glowing shell, swirls caught in glass,
 *  orbiting rings and a wireframe of light. Palettes derive from the user's
 *  accent + gradient and re-derive on `appearance:change`.
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

import { paletteForState } from './orbPalette.js';

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

/* ── Tiny 3D helpers — everything is orthographic-projected onto the orb ── */

function rotY(p, a) {
  const c = Math.cos(a);
  const s = Math.sin(a);
  return [p[0] * c + p[2] * s, p[1], -p[0] * s + p[2] * c];
}

function rotX(p, a) {
  const c = Math.cos(a);
  const s = Math.sin(a);
  return [p[0], p[1] * c - p[2] * s, p[1] * s + p[2] * c];
}

function normalize(v) {
  const m = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / m, v[1] / m, v[2] / m];
}

function cross(a, b) {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
}

/** An orthonormal basis (u, v) for the plane whose normal is `n`. */
function basisFor(n) {
  const helper = Math.abs(n[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
  const u = normalize(cross(n, helper));
  return { u, v: cross(n, u) };
}

/** Great circles, spread evenly like a ball of yarn (golden-angle normals). */
const STRANDS = (() => {
  const n = 11;
  const out = [];
  for (let i = 0; i < n; i++) {
    const y = 1 - (i / (n - 1)) * 2;
    const r = Math.sqrt(Math.max(0, 1 - y * y));
    const phi = i * 2.399963229728653;
    const normal = [Math.cos(phi) * r, y, Math.sin(phi) * r];
    out.push({ normal, ...basisFor(normal) });
  }
  return out;
})();

/** Point on a strand at angle `th`, rotated and projected to the canvas. */
function strandPoint(strand, th, spin, tilt, cx, cy, r) {
  const c = Math.cos(th);
  const s = Math.sin(th);
  const p = [
    strand.u[0] * c + strand.v[0] * s,
    strand.u[1] * c + strand.v[1] * s,
    strand.u[2] * c + strand.v[2] * s,
  ];
  const q = rotX(rotY(p, spin), tilt);
  return [cx + q[0] * r, cy + q[1] * r, q[2]];
}

/**
 * Stroke one great circle as a filament. Split into arcs so the alpha can
 * follow depth: the far side of the sphere fades, the near side blazes. Each
 * arc is stroked twice on the same path — a wide bloom, then a thin core.
 */
function drawFilament(ctx, strand, o) {
  const { cx, cy, r, spin, tilt, color, alpha, width, dpr, seg = 6, steps = 6 } = o;
  ctx.lineJoin = 'round';
  ctx.lineCap = 'round';
  for (let a = 0; a < seg; a++) {
    const t0 = (a / seg) * TAU;
    const t1 = ((a + 1) / seg) * TAU;
    let z = 0;
    ctx.beginPath();
    for (let i = 0; i <= steps; i++) {
      const [x, y, pz] = strandPoint(strand, t0 + (t1 - t0) * (i / steps), spin, tilt, cx, cy, r);
      z += pz;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    const depth = 0.3 + 0.7 * ((z / (steps + 1) + 1) / 2);
    ctx.strokeStyle = hexToRgba(color, Math.min(1, alpha * depth * 0.3));
    ctx.lineWidth = width * 4.5 * dpr;
    ctx.stroke();
    ctx.strokeStyle = hexToRgba(color, Math.min(1, alpha * depth * 1.4));
    ctx.lineWidth = width * dpr;
    ctx.stroke();
  }
}

/** A soft radial light. */
function blob(ctx, w, x, y, r, color, { alpha = 1 } = {}) {
  if (r <= 0) return;
  const g = ctx.createRadialGradient(x, y, 0, x, y, r);
  g.addColorStop(0, hexToRgba(color, alpha));
  g.addColorStop(0.4, hexToRgba(color, 0.5 * alpha));
  g.addColorStop(0.74, hexToRgba(color, 0.12 * alpha));
  g.addColorStop(1, 'rgba(0,0,0,0)');
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, w, w);
}

/* ── Styles ─────────────────────────────────────────────────────
   Each paints inside the clipped body. `p` carries:
   { ctx, w, cx, cy, R, dpr, t, palette, intensity, pan, squareAnim,
     speedMul, scale, glowI }
   ─────────────────────────────────────────────────────────────── */

const STYLES = {
  /** Filament — a ball of light trails wrapped around a glass sphere, with
   *  one thick ribbon band. The reference image, made of maths. */
  filament: {
    draw(p) {
      const { ctx, w, cx, cy, R, palette, t, intensity, dpr } = p;
      const energy = Math.min(1, intensity);
      const spin = t * (0.16 + 0.14 * energy);
      const tilt = 0.42 + 0.05 * Math.sin(t * 0.22);
      const breathe = 0.94 + 0.06 * Math.sin(t * 0.9) + 0.05 * energy;

      // Interior light so the sphere is not hollow.
      blob(ctx, w, cx, cy, R * 1.02, palette[3], { alpha: 0.1 + 0.12 * energy });

      for (let i = 0; i < STRANDS.length; i++) {
        drawFilament(ctx, STRANDS[i], {
          cx, cy, r: R * breathe,
          spin, tilt,
          color: palette[i % palette.length],
          alpha: 0.5 + 0.5 * energy,
          width: Math.max(0.6, R * 0.011),
          dpr,
        });
      }

      // One ribbon: two passes, wide then bright.
      const band = STRANDS[3];
      ctx.lineJoin = 'round';
      ctx.lineCap = 'round';
      for (const [wMul, a] of [[15, 0.1 + 0.12 * energy], [5, 0.22 + 0.24 * energy]]) {
        ctx.beginPath();
        for (let i = 0; i <= 72; i++) {
          const [x, y] = strandPoint(band, (i / 72) * TAU, spin, tilt, cx, cy, R * breathe);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.strokeStyle = hexToRgba(palette[1], a);
        ctx.lineWidth = Math.max(0.8, R * 0.012 * wMul) * dpr;
        ctx.stroke();
      }
    },
  },

  /** Bubble — a hollow glass shell whose rim carries the light, with a cool
   *  inner bloom, a white specular arc and a warm crescent. */
  bubble: {
    draw(p) {
      const { ctx, cy, cx, R, palette, t, intensity, dpr } = p;
      const energy = Math.min(1, intensity);
      const shell = R * (0.86 + 0.02 * Math.sin(t * 1.1) + 0.04 * energy);

      // Light refracting just inside the rim — the shell's thickness.
      const inner = ctx.createRadialGradient(cx, cy, shell * 0.5, cx, cy, shell);
      inner.addColorStop(0, 'rgba(0,0,0,0)');
      inner.addColorStop(0.72, hexToRgba(palette[1], 0.08 + 0.16 * energy));
      inner.addColorStop(1, hexToRgba(palette[0], 0.26 + 0.32 * energy));
      ctx.fillStyle = inner;
      ctx.beginPath();
      ctx.arc(cx, cy, shell, 0, TAU);
      ctx.fill();

      // Rim: one bloom pass, one bright hairline.
      const rim = ctx.createLinearGradient(cx - shell, cy - shell, cx + shell, cy + shell);
      rim.addColorStop(0, hexToRgba(palette[0], 0.85));
      rim.addColorStop(0.5, hexToRgba(palette[2], 0.45 + 0.3 * energy));
      rim.addColorStop(1, hexToRgba(palette[1], 0.85));
      ctx.beginPath();
      ctx.arc(cx, cy, shell, 0, TAU);
      ctx.strokeStyle = rim;
      ctx.lineWidth = Math.max(1, R * 0.1) * dpr;
      ctx.save();
      ctx.globalAlpha = 0.22 + 0.24 * energy;
      ctx.stroke();
      ctx.restore();
      ctx.lineWidth = Math.max(0.7, R * 0.016) * dpr;
      ctx.stroke();

      // Specular arc (top-left) and a warm reflection (lower-left).
      ctx.beginPath();
      ctx.arc(cx, cy, shell, Math.PI * 1.06, Math.PI * 1.42);
      ctx.strokeStyle = 'rgba(255,255,255,0.55)';
      ctx.lineWidth = Math.max(1, R * 0.03) * dpr;
      ctx.stroke();

      ctx.beginPath();
      ctx.arc(cx, cy, shell * 0.97, Math.PI * 0.6, Math.PI * 0.96);
      ctx.strokeStyle = hexToRgba(palette[1], 0.3 + 0.25 * energy);
      ctx.lineWidth = Math.max(1, R * 0.045) * dpr;
      ctx.stroke();
    },
  },

  /** Marble — a glass sphere with luminous ribbons and filaments caught
   *  inside, and a bright refractive base. */
  marble: {
    draw(p) {
      const { ctx, w, cx, cy, R, palette, t, intensity, dpr } = p;
      const energy = Math.min(1, intensity);

      // Glass body: faint centre, luminous shell.
      const shell = ctx.createRadialGradient(cx, cy, R * 0.15, cx, cy, R);
      shell.addColorStop(0, hexToRgba(palette[2], 0.04 + 0.06 * energy));
      shell.addColorStop(0.68, hexToRgba(palette[0], 0.07 + 0.1 * energy));
      shell.addColorStop(0.93, hexToRgba(palette[0], 0.34 + 0.3 * energy));
      shell.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.fillStyle = shell;
      ctx.fillRect(0, 0, w, w);

      // Internal ribbons, each a wide glow behind a bright core.
      ctx.lineJoin = 'round';
      ctx.lineCap = 'round';
      for (let i = 0; i < 3; i++) {
        const color = palette[(i + 1) % palette.length];
        ctx.save();
        ctx.translate(cx, cy);
        ctx.rotate(t * (0.1 + i * 0.04) * (1 + 0.5 * energy) + i * 2.1);
        for (const [wMul, a] of [
          [0.2, 0.09 + 0.12 * energy],
          [0.055, 0.3 + 0.32 * energy],
        ]) {
          ctx.strokeStyle = hexToRgba(color, a);
          ctx.lineWidth = R * wMul * dpr;
          ctx.beginPath();
          ctx.moveTo(-R * 0.86, -R * 0.12);
          ctx.bezierCurveTo(-R * 0.25, -R * 0.95, R * 0.3, R * 0.7, R * 0.86, R * 0.14);
          ctx.stroke();
        }
        ctx.restore();
      }

      // Fine filaments across the glass.
      for (let i = 0; i < 4; i++) {
        const y = cy - R * 0.5 + (i / 3) * R;
        ctx.strokeStyle = hexToRgba(palette[0], 0.07 + 0.12 * energy);
        ctx.lineWidth = Math.max(0.5, R * 0.007) * dpr;
        ctx.beginPath();
        ctx.moveTo(cx - R * 0.92, y);
        ctx.quadraticCurveTo(cx, y + R * 0.35, cx + R * 0.92, y);
        ctx.stroke();
      }

      // Refraction along the base of the marble.
      const base = ctx.createRadialGradient(cx, cy + R * 0.55, 0, cx, cy + R * 0.55, R * 0.75);
      base.addColorStop(0, hexToRgba(palette[1], 0.24 + 0.25 * energy));
      base.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.fillStyle = base;
      ctx.fillRect(0, 0, w, w);
    },
  },

  /** Orbit — a bright core circled by tilted luminous rings, each carrying a
   *  bead of light. The rings widen and quicken with the voice. */
  orbit: {
    draw(p) {
      const { ctx, w, cx, cy, R, palette, t, intensity, dpr } = p;
      const energy = Math.min(1, intensity);
      blob(ctx, w, cx, cy, R * (0.46 + 0.12 * energy), palette[0], {
        alpha: 0.32 + 0.26 * energy,
      });

      const rings = [
        { squash: 0.34, spin: 0.2, radius: 0.74, tint: 1 },
        { squash: 0.64, spin: -0.14, radius: 0.86, tint: 2 },
        { squash: 0.16, spin: 0.35, radius: 0.62, tint: 3 },
      ];
      for (const ring of rings) {
        const color = palette[ring.tint % palette.length];
        const radius = R * ring.radius * (1 + 0.07 * energy);
        const rot = t * ring.spin * (1 + 0.5 * energy);
        ctx.save();
        ctx.translate(cx, cy);
        ctx.rotate(rot);
        ctx.scale(1, ring.squash);
        ctx.strokeStyle = hexToRgba(color, 0.14 + 0.28 * energy);
        ctx.lineWidth = Math.max(0.7, R * 0.012) * dpr;
        ctx.beginPath();
        ctx.arc(0, 0, radius, 0, TAU);
        ctx.stroke();
        ctx.restore();

        // The bead rides the ring in absolute space (a blob fills the whole
        // canvas, so it must not be drawn inside the ring's transform).
        const a = t * (0.5 + 0.6 * energy) + ring.tint;
        const lx = Math.cos(a) * radius;
        const ly = Math.sin(a) * radius * ring.squash;
        blob(
          ctx, w,
          cx + lx * Math.cos(rot) - ly * Math.sin(rot),
          cy + lx * Math.sin(rot) + ly * Math.cos(rot),
          R * 0.13,
          color,
          { alpha: 0.7 + 0.3 * energy },
        );
      }
    },
  },

  /** Grid — a wireframe sphere of latitude and longitude lines around a soft
   *  core: the structure visible inside the reference marbles. */
  grid: {
    draw(p) {
      const { ctx, w, cx, cy, R, palette, t, intensity, dpr } = p;
      const energy = Math.min(1, intensity);
      const spin = t * (0.14 + 0.1 * energy);
      const tilt = 0.36;

      blob(ctx, w, cx, cy, R * (0.8 + 0.1 * energy), palette[2], {
        alpha: 0.08 + 0.12 * energy,
      });

      ctx.lineJoin = 'round';
      const project = (pt) => {
        const q = rotX(rotY(pt, spin), tilt);
        return [cx + q[0] * R, cy + q[1] * R];
      };

      // Latitudes.
      for (let lat = -60; lat <= 60; lat += 30) {
        const rad = (lat * Math.PI) / 180;
        const lr = Math.cos(rad);
        const ly = Math.sin(rad);
        ctx.beginPath();
        for (let i = 0; i <= 48; i++) {
          const th = (i / 48) * TAU;
          const [x, y] = project([Math.cos(th) * lr, ly, Math.sin(th) * lr]);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.strokeStyle = hexToRgba(palette[0], 0.1 + 0.2 * energy);
        ctx.lineWidth = Math.max(0.5, R * 0.008) * dpr;
        ctx.stroke();
      }

      // Longitudes.
      for (let lon = 0; lon < 180; lon += 30) {
        const a = (lon * Math.PI) / 180;
        const u = [Math.cos(a), 0, Math.sin(a)];
        ctx.beginPath();
        for (let i = 0; i <= 48; i++) {
          const th = (i / 48) * TAU;
          const [x, y] = project([
            u[0] * Math.cos(th),
            Math.sin(th),
            u[2] * Math.cos(th),
          ]);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.strokeStyle = hexToRgba(palette[1], 0.08 + 0.16 * energy);
        ctx.lineWidth = Math.max(0.5, R * 0.007) * dpr;
        ctx.stroke();
      }

      // A single bright equator seam keeps it from reading as a plain mesh.
      ctx.beginPath();
      for (let i = 0; i <= 64; i++) {
        const th = (i / 64) * TAU;
        const [x, y] = project([Math.cos(th), 0, Math.sin(th)]);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.strokeStyle = hexToRgba(palette[0], 0.28 + 0.3 * energy);
      ctx.lineWidth = Math.max(0.7, R * 0.014) * dpr;
      ctx.stroke();
    },
  },
};

const DEFAULT_STYLE = 'filament';

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
    ctx.shadowBlur = (14 + glowI * 16) * this.dpr;
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
    const highlight = ctx.createRadialGradient(cx - R * 0.3, cy - R * 0.34, 0, cx, cy, R * 0.98);
    highlight.addColorStop(0, 'rgba(255,255,255,0.16)');
    highlight.addColorStop(0.4, 'rgba(255,255,255,0.03)');
    highlight.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.fillStyle = highlight;
    ctx.fillRect(0, 0, w, w);

    const edge = ctx.createRadialGradient(cx, cy, R * 0.6, cx, cy, R);
    edge.addColorStop(0, 'rgba(0,0,0,0)');
    edge.addColorStop(0.88, 'rgba(0,0,0,0.07)');
    edge.addColorStop(1, 'rgba(0,0,0,0.18)');
    ctx.fillStyle = edge;
    ctx.fillRect(0, 0, w, w);
    ctx.restore();

    // Escaping ripples when the voice is loud — faint pressure in the air.
    if (glowI > 0.14 && !squareAnim) {
      const rippleT = (this.t * 1.5) % 1;
      for (let i = 0; i < 2; i++) {
        const p = (rippleT + i * 0.5) % 1;
        const rr = R + p * R * 0.22;
        const g = ctx.createRadialGradient(cx, cy, rr * 0.9, cx, cy, rr);
        g.addColorStop(0, hexToRgba(this.palette[0], (1 - p) * 0.07));
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
