/** Siri-style fluid orb — canvas only, circular clip */

import { DEFAULT_ACCENT } from './accent.js';

const PALETTES_DARK = {
  idle: ['#4a7fd4', '#5eead4', '#a78bfa', '#6b9de8'],
  listening: ['#fcd34d', '#4a7fd4', '#60a5fa', '#fbbf24'],
  conversation: ['#5eead4', '#4a7fd4', '#2dd4bf', '#6b9de8'],
  compose: ['#4a7fd4', '#6b9de8', '#5eead4', '#93c5fd'],
  processing: ['#4a7fd4', '#6b9de8', '#5eead4', '#93c5fd'],
  speaking: ['#6b9de8', '#4a7fd4', '#5eead4', '#67e8f9'],
  error: ['#fca5a5', '#f87171', '#fb7185', '#ef4444'],
  downloading: ['#93c5fd', '#60a5fa', '#818cf8', '#3b82f6'],
  disabled: ['#64748b', '#475569', '#94a3b8', '#334155'],
};

const SQUARE_STATES = new Set(['listening', 'conversation', 'processing', 'speaking']);

let themeMode = 'dark';
let accentColor = DEFAULT_ACCENT;

function hexToRgba(hex, a) {
  const h = hex.replace('#', '');
  const n = parseInt(h.length === 3 ? h.split('').map((c) => c + c).join('') : h, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  return `rgba(${r},${g},${b},${a})`;
}

function lighten(hex, amount) {
  const h = hex.replace('#', '');
  const n = parseInt(h.length === 3 ? h.split('').map((c) => c + c).join('') : h, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  const mix = (c) => Math.min(255, Math.round(c + (255 - c) * amount));
  return `#${[mix(r), mix(g), mix(b)].map((c) => c.toString(16).padStart(2, '0')).join('')}`;
}

function paletteForState(state) {
  const base = [...(PALETTES_DARK[state] || PALETTES_DARK.idle)];
  if (state === 'idle' || state === 'listening' || state === 'compose' || state === 'conversation' || state === 'processing') {
    base[0] = accentColor;
    base[1] = lighten(accentColor, 0.2);
  }
  return base;
}

let renderer = null;

class OrbRenderer {
  constructor(canvas) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d', { alpha: true });
    this.displaySize = 84;
    this.palette = paletteForState('idle');
    this.stateKey = 'idle';
    this.animationMode = 'orbit';
    this.themeMode = themeMode;
    this.intensity = 0;
    this.t = 0;
    this.running = true;

    this.blobs = [
      { phase: 0, speed: 0.9, orbit: 0.22, radius: 0.55 },
      { phase: 2.1, speed: 1.15, orbit: 0.18, radius: 0.48 },
      { phase: 4.2, speed: 0.75, orbit: 0.26, radius: 0.52 },
      { phase: 1.3, speed: 1.05, orbit: 0.2, radius: 0.44 },
    ];

    this.resize();
    window.addEventListener('resize', () => this.resize());
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

  setPalette(state) {
    this.stateKey = state;
    this.palette = paletteForState(state);
    this.animationMode = SQUARE_STATES.has(state) ? 'square' : 'orbit';
  }

  setTheme(mode) {
    this.themeMode = mode === 'light' ? 'light' : 'dark';
    this.setPalette(this.stateKey);
  }

  setIntensity(v) {
    this.intensity = Math.min(1, Math.max(0, v));
  }

  destroy() {
    this.running = false;
  }

  _loop(ts) {
    if (!this.running) return;
    this.t = ts * 0.001;
    this.draw();
    requestAnimationFrame(this._loop);
  }

  _drawBlob(ctx, w, x, y, r, color, { sharp = false } = {}) {
    const midA = sharp
      ? (this.themeMode === 'light' ? 0.92 : 0.88)
      : (this.themeMode === 'light' ? 0.78 : 0.65);
    const outerA = sharp
      ? (this.themeMode === 'light' ? 0.35 : 0.28)
      : (this.themeMode === 'light' ? 0.22 : 0.15);
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

  draw() {
    const { ctx, px: w } = this;
    const cx = w / 2;
    const cy = w / 2;
    const R = w * 0.44;
    const stateBoost =
      this.stateKey === 'listening' || this.stateKey === 'conversation' ? 0.22
        : this.stateKey === 'processing' || this.stateKey === 'speaking' || this.stateKey === 'compose' ? 0.18
          : 0;
    const pulse = 1 + Math.min(1, this.intensity + stateBoost) * 0.45;
    const glowI = Math.min(1, this.intensity + stateBoost);
    const squareAnim = this.animationMode === 'square';
    const speedMul =
      this.stateKey === 'processing' ? 1.35
        : this.stateKey === 'listening' || this.stateKey === 'conversation' || this.stateKey === 'compose' ? 1.2
          : this.stateKey === 'speaking' ? 1.15
            : 1;

    ctx.clearRect(0, 0, w, w);

    ctx.save();
    const glowA = this.themeMode === 'light'
      ? 0.72 + glowI * 0.28
      : 0.55 + glowI * 0.25;
    ctx.shadowColor = hexToRgba(this.palette[0], glowA);
    ctx.shadowBlur = (squareAnim ? 6 + glowI * 6 : (this.themeMode === 'light' ? 22 + glowI * 26 : 14 + glowI * 18)) * this.dpr;
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(0,0,0,0.004)';
    ctx.fill();
    ctx.restore();

    ctx.save();
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, Math.PI * 2);
    ctx.clip();

    ctx.fillStyle = '#06080a';
    ctx.fillRect(0, 0, w, w);

    ctx.globalCompositeOperation = 'screen';

    if (squareAnim) {
      const d = R * 0.24 * pulse;
      const rot = this.t * 0.65 * speedMul;
      const corners = [
        [-1, -1], [1, -1], [1, 1], [-1, 1],
      ];
      const blobOpts = { sharp: true };
      for (let i = 0; i < this.blobs.length; i++) {
        const [sx, sy] = corners[i];
        const cos = Math.cos(rot);
        const sin = Math.sin(rot);
        const rx = sx * cos - sy * sin;
        const ry = sx * sin + sy * cos;
        const x = cx + rx * d;
        const y = cy + ry * d;
        const r = this.blobs[i].radius * w * pulse * 0.42;
        const color = this.palette[i % this.palette.length];
        this._drawBlob(ctx, w, x, y, r, color, blobOpts);
      }
    } else {
      for (let i = 0; i < this.blobs.length; i++) {
        const b = this.blobs[i];
        const color = this.palette[i % this.palette.length];
        const angle = this.t * b.speed * speedMul + b.phase;
        const wobble = Math.sin(this.t * 1.7 + b.phase * 2) * 0.06 * w;
        const orbit = b.orbit * w * pulse;
        const x = cx + Math.cos(angle) * orbit + wobble;
        const y = cy + Math.sin(angle * 1.25 + b.phase) * orbit - wobble * 0.5;
        const r = b.radius * w * pulse * (0.92 + Math.sin(this.t * 2 + i) * 0.08);
        this._drawBlob(ctx, w, x, y, r, color);
      }
    }

    ctx.globalAlpha = 1;

    if (!squareAnim) {
      ctx.globalAlpha = this.themeMode === 'light' ? 0.65 : 0.55;
      const swirl = ctx.createRadialGradient(
        cx + Math.cos(this.t * 0.4) * R * 0.35,
        cy + Math.sin(this.t * 0.35) * R * 0.35,
        0,
        cx,
        cy,
        R * 1.1,
      );
      swirl.addColorStop(0, hexToRgba(this.palette[2], 0.5));
      swirl.addColorStop(0.5, hexToRgba(this.palette[1], 0.2));
      swirl.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.fillStyle = swirl;
      ctx.fillRect(0, 0, w, w);
      ctx.globalAlpha = 1;
    }

    ctx.globalCompositeOperation = 'source-over';

    const highlight = ctx.createRadialGradient(cx - R * 0.25, cy - R * 0.3, 0, cx, cy, R * 0.95);
    highlight.addColorStop(0, 'rgba(255,255,255,0.22)');
    highlight.addColorStop(0.35, 'rgba(255,255,255,0.04)');
    highlight.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.fillStyle = highlight;
    ctx.fillRect(0, 0, w, w);

    const edge = ctx.createRadialGradient(cx, cy, R * 0.55, cx, cy, R);
    edge.addColorStop(0, 'rgba(0,0,0,0)');
    edge.addColorStop(0.85, 'rgba(0,0,0,0.12)');
    edge.addColorStop(1, 'rgba(0,0,0,0.5)');
    ctx.fillStyle = edge;
    ctx.fillRect(0, 0, w, w);

    ctx.restore();

    if (glowI > 0.12 && !squareAnim) {
      const rippleT = (this.t * 1.8) % 1;
      for (let i = 0; i < 2; i++) {
        const p = (rippleT + i * 0.5) % 1;
        const rr = R + p * R * 0.28;
        const g = ctx.createRadialGradient(cx, cy, rr * 0.85, cx, cy, rr);
        const rippleA = this.themeMode === 'light' ? 0.28 : 0.12;
        g.addColorStop(0, hexToRgba(this.palette[0], (1 - p) * rippleA));
        g.addColorStop(1, 'rgba(0,0,0,0)');
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.arc(cx, cy, rr, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }
}

function refreshAccentPalette() {
  if (renderer) renderer.setPalette(renderer.stateKey);
}

export function initOrbCanvas(canvas) {
  if (renderer) renderer.destroy();
  themeMode = document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
  accentColor = localStorage.getItem('ui.accent') || DEFAULT_ACCENT;
  renderer = new OrbRenderer(canvas);
  renderer.setTheme(themeMode);
  window.addEventListener('accent:change', (e) => {
    accentColor = e.detail?.accent || DEFAULT_ACCENT;
    refreshAccentPalette();
  });
  return renderer;
}

export function setOrbTheme(theme) {
  themeMode = theme === 'light' ? 'light' : 'dark';
  renderer?.setTheme(themeMode);
}

export function setOrbPalette(state) {
  renderer?.setPalette(state);
}

export function setOrbIntensity(v) {
  renderer?.setIntensity(v);
}

export function resetOrbIntensity() {
  renderer?.setIntensity(0);
}
