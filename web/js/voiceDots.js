/**
 * voiceDots.js — the voice bar's one effect: a grid of dots that fills the bar.
 *
 * The dots react to the audio (the mic while listening, the voice while the
 * reply is spoken) and/or to the assistant's tool calls — Settings →
 * Appearance → "Dots react to" (`orb_react`: voice, tools, both). How they move
 * is the effect (`orb_effect`):
 *
 *  - glyph    a dot-matrix display: the level scrolls by like a scope, a tool
 *             draws its own mark dot by dot, thinking hops three dots (default)
 *  - eq       an equaliser mirrored from the centre; a scanner sweeps for tools
 *  - ripple   a halftone that swells with the voice and rings on every peak; a
 *             comet runs round the edge for tools
 *  - firefly  sparks gathered toward the stereo pan; a snake roams for tools
 *
 * The grid runs edge to edge, half a pitch in from every side: the bar's
 * rounded ends simply clip the corner dots. Their colour is the bar's ink
 * (`color` on the host, see sphere.css) — the theme's text on the theme plate,
 * the accent's contrast on an accent body.
 *
 * The signal: `voice:state` (sphere.js), `voice:level` (recognition.js /
 * playback.js) and `agent:step` (dockStep.js, with the running tool's action).
 * The Settings preview mounts the same renderer on a demo signal instead.
 */

import { getImmersive } from './preferences.js';
import { GLYPHS, dotGlyph } from './voiceDotGlyphs.js';

/* ── Signals ─────────────────────────────────────────────────── */

const AUDIBLE = new Set(['listening', 'conversation', 'speaking']);

function makeSignal() {
  return {
    voice: 'idle',
    tool: null,        // { glyph, t0 } while a tool runs
    stepping: false,   // a step line is up (the assistant is working)
    level: 0,
    target: 0,
    pan: 0,
    levelAt: 0,
    bands: new Float32Array(16),
    shape: new Float32Array(16).fill(0.5),
    history: new Float32Array(96),
    histAt: 0,
    above: false,
    events: [],        // { type: 'peak'|'tool', t, x, s }
  };
}

/** Advance the shared parts of a signal: level smoothing, bands, history, peaks. */
function settle(sig, now) {
  // The level events stop when the audio does: let it fall back on its own.
  if (now - sig.levelAt > 160) sig.target = 0;
  sig.level += (sig.target - sig.level) * 0.35;
  for (let i = 0; i < 16; i++) {
    // The bar has one level, not a spectrum: each band wanders around it.
    sig.shape[i] += (Math.random() - 0.5) * 0.18;
    sig.shape[i] = Math.min(1, Math.max(0.25, sig.shape[i]));
    const centreWeight = 1 - (i / 16) * 0.55;
    const t = sig.level * sig.shape[i] * centreWeight * 1.25;
    sig.bands[i] += (t - sig.bands[i]) * 0.4;
  }
  const above = sig.level > 0.55;
  if (above && !sig.above) sig.events.push({ type: 'peak', t: now, x: sig.pan, s: sig.level });
  sig.above = above;
  if (sig.events.length) sig.events = sig.events.filter((e) => now - e.t < 1600);
  if (now - sig.histAt > 45) {
    sig.histAt = now;
    sig.history.copyWithin(0, 1);
    sig.history[sig.history.length - 1] = sig.level;
  }
}

function startTool(sig, glyph, now) {
  sig.tool = { glyph, t0: now };
  sig.events.push({ type: 'tool', t: now, x: 0, s: 1 });
}

/** The live signal: one for the whole shell, fed by window events. */
let liveSignal = null;
function live() {
  if (liveSignal) return liveSignal;
  const sig = makeSignal();
  sig.voice = document.documentElement.dataset.voice || 'idle';
  window.addEventListener('voice:state', (e) => { sig.voice = e.detail?.state || 'idle'; });
  window.addEventListener('voice:level', (e) => {
    const d = e.detail;
    const level = Number(typeof d === 'number' ? d : d?.level);
    const pan = Number(d?.pan);
    if (Number.isFinite(level)) { sig.target = Math.min(1, Math.max(0, level)); sig.levelAt = performance.now(); }
    if (Number.isFinite(pan)) sig.pan += (Math.min(1, Math.max(-1, pan)) - sig.pan) * 0.2;
  });
  window.addEventListener('agent:step', (e) => {
    const { message, action } = e.detail || {};
    if (!message) { sig.stepping = false; sig.tool = null; return; }
    sig.stepping = true;
    const glyph = dotGlyph(action);
    if (!glyph) { sig.tool = null; return; }
    // A new tool (or the same one called again on a new step) starts over.
    startTool(sig, glyph, performance.now());
  });
  sig.tick = (now) => settle(sig, now);
  liveSignal = sig;
  return sig;
}

/** A demo signal for the Settings preview: one whole exchange, looped. */
const DEMO = [
  ['idle', 1400], ['listening', 3000], ['processing', 900],
  ['tool:search', 2000], ['tool:cloud', 1800], ['processing', 600], ['speaking', 3400],
];
const DEMO_LEN = DEMO.reduce((a, [, d]) => a + d, 0);

function demo() {
  const sig = makeSignal();
  let scene = '';
  let syllableUntil = 0;
  sig.tick = (now) => {
    let t = now % DEMO_LEN;
    let next = 'idle';
    for (const [s, d] of DEMO) { if (t < d) { next = s; break; } t -= d; }
    const [kind, glyph] = next.split(':');
    if (next !== scene) {
      scene = next;
      sig.voice = kind === 'tool' ? 'processing' : kind;
      sig.stepping = kind === 'tool' || kind === 'processing';
      if (kind === 'tool') startTool(sig, glyph, now); else sig.tool = null;
    }
    if (AUDIBLE.has(sig.voice)) {
      if (now > syllableUntil) {
        const pause = Math.random() < 0.22;
        sig.target = pause ? 0.02 : 0.3 + Math.random() * 0.7;
        syllableUntil = now + (pause ? 120 + Math.random() * 240 : 80 + Math.random() * 140);
      }
      sig.levelAt = now;
    }
    settle(sig, now);
  };
  return sig;
}

/* ── Helpers ─────────────────────────────────────────────────── */

const clamp01 = (v) => (v < 0 ? 0 : v > 1 ? 1 : v);
function hash(a, b, c) {
  let h = (a * 374761393 + b * 668265263 + c * 2147483647) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
}
const thinking = (sig) => !sig.tool;

/* ── The effects: per-dot intensity 0..1 for each driver ──────
   g = the grid ({ cols, rows, cx, cy, … }), st = per-bar scratch state. */

const EFFECTS = {
  glyph: {
    size: 0.2,
    audio(c, r, g, now, st, sig) {
      const h = sig.history;
      const k = h.length - g.cols + c;
      const lv = k >= 0 ? h[k] : 0;
      return clamp01(lv * (g.rows / 2 + 0.4) - Math.abs(r - g.cy) + 0.15);
    },
    tool(c, r, g, now, st, sig) {
      if (thinking(sig)) {
        // Three dots hopping in the middle, like "is typing…".
        const mid = Math.round(g.cx);
        for (let k = -1; k <= 1; k++) {
          if (c !== mid + k * 2) continue;
          const hop = Math.max(0, Math.sin(now / 160 - (k + 1) * 0.9));
          if (r === Math.round(g.cy - hop * 1.2)) return 0.9;
        }
        return 0;
      }
      const rows = GLYPHS[sig.tool.glyph] || GLYPHS.spark;
      const gw = rows[0].length;
      const gx = c - Math.round(g.cx - (gw - 1) / 2);
      const gy = r - Math.round(g.cy - 2);
      if (gy < 0 || gy >= 5 || gx < 0 || gx >= gw || rows[gy][gx] !== '#') return 0;
      // The mark assembles dot by dot, then a scan line runs across it.
      const appear = clamp01((now - sig.tool.t0 - hash(c, r, st.seed) * 320) / 120);
      const scan = (now / 9) % (gw + 8) - 4;
      return clamp01(appear * (0.7 + Math.exp(-((gx - scan) ** 2) / 1.2) * 0.35));
    },
  },

  eq: {
    size: 0.25,
    audio(c, r, g, now, st, sig) {
      const half = (g.cols - 1) / 2;
      const band = Math.min(15, Math.floor((Math.abs(c - half) / Math.max(1, half)) * 15.99));
      const h = sig.bands[band] * 1.25 * (g.rows / 2 + 0.5);
      return clamp01(h - Math.abs(r - g.cy) + 0.2);
    },
    tool(c, r, g, now, st, sig) {
      const slow = thinking(sig);
      const period = slow ? 2200 : 1300;
      const ph = (now % period) / period;
      const pos = (ph < 0.5 ? ph * 2 : 2 - ph * 2) * (g.cols - 1);
      const d = (pos - c) * (ph < 0.5 ? 1 : -1); // > 0: behind the head
      let v = d < -0.6 ? 0 : d < 0.6 ? 1 : Math.exp(-(d - 0.6) / 2.2);
      if (slow) v *= 0.55;
      const flash = sig.tool ? Math.exp(-(now - sig.tool.t0) / 160) * 0.8 : 0;
      return Math.max(v, flash);
    },
  },

  ripple: {
    size: 0.85,
    audio(c, r, g, now, st, sig) {
      const px = g.cx + sig.pan * g.cols * 0.3;
      const d = Math.hypot(c - px, (r - g.cy) * 1.4);
      let v = sig.level * Math.exp(-(d * d) / (g.cols * 1.8));
      for (const e of sig.events) {
        if (e.type !== 'peak') continue;
        const age = (now - e.t) / 1000;
        const dd = Math.hypot(c - (g.cx + e.x * g.cols * 0.3), (r - g.cy) * 1.4) - age * g.cols * 0.9;
        v += e.s * Math.exp(-(dd * dd) / 1.4) * Math.max(0, 1 - age / 1.1) * 0.8;
      }
      return clamp01(v);
    },
    tool(c, r, g, now, st, sig) {
      const slow = thinking(sig);
      const n = g.perimeter.length;
      const head = (now / (slow ? 70 : 38)) % n;
      const i = g.perIndex[r * g.cols + c];
      let v = 0;
      if (i >= 0) {
        const back = (head - i + n) % n;
        const len = slow ? 4 : 9;
        v = back < len ? (1 - back / len) ** 1.5 : 0;
      }
      for (const e of sig.events) {
        if (e.type !== 'tool') continue;
        const age = (now - e.t) / 1000;
        const dd = Math.hypot(c - g.cx, (r - g.cy) * 1.4) - age * g.cols * 1.1;
        v += Math.exp(-(dd * dd) / 1.6) * Math.max(0, 1 - age / 0.9);
      }
      return clamp01(v * (slow ? 0.7 : 1));
    },
  },

  firefly: {
    size: 0.45,
    frame(g, now, st, sig, mode, wAudio, wTool) {
      const n = g.cols * g.rows;
      if (!st.v || st.v.length !== n) { st.v = new Float32Array(n); st.snake = []; st.step = 0; }
      for (let i = 0; i < n; i++) st.v[i] *= 0.92;
      const tick = Math.floor(now / 40);
      if (tick !== st.tick) {
        st.tick = tick;
        // Sparks: as many as the level, gathered toward the pan.
        const px = g.cx + sig.pan * g.cols * 0.35;
        const rate = 0.006 + sig.level * 0.16;
        for (let r = 0; r < g.rows; r++) {
          for (let c = 0; c < g.cols; c++) {
            const bias = Math.exp(-((c - px) ** 2) / (g.cols * 2.5));
            const p = mode === 'idle' ? 0.004 : rate * (0.35 + 0.65 * bias) * wAudio;
            const i = r * g.cols + c;
            if (hash(c, r, tick) < p) st.v[i] = Math.max(st.v[i], 0.6 + 0.4 * hash(r, c, tick));
          }
        }
        // A burst of sparks on every new tool.
        for (const e of sig.events) {
          if (e.type === 'tool' && !st.burst?.has(e)) {
            (st.burst ||= new WeakSet()).add(e);
            for (let i = 0; i < n; i++) if (Math.random() < 0.45) st.v[i] = 1;
          }
        }
      }
      // The snake roams while a tool runs; it shortens and slows while thinking.
      const slow = thinking(sig);
      const len = slow ? 3 : 6;
      if (wTool > 0.05 && now - st.step > (slow ? 150 : 70)) {
        st.step = now;
        const head = st.snake[0] || [Math.floor(g.cx), Math.floor(g.cy)];
        if (!st.dir || Math.random() < 0.28) {
          const dirs = [[1, 0], [-1, 0], [0, 1], [0, -1], [1, 0], [-1, 0]];
          st.dir = dirs[Math.floor(Math.random() * dirs.length)];
        }
        let nx = head[0] + st.dir[0];
        let ny = head[1] + st.dir[1];
        if (nx < 0 || nx >= g.cols) { st.dir = [-st.dir[0], 0]; nx = head[0] + st.dir[0]; }
        if (ny < 0 || ny >= g.rows) {
          st.dir = [st.dir[0] || 1, 0];
          ny = head[1];
          nx = Math.min(g.cols - 1, Math.max(0, head[0] + st.dir[0]));
        }
        st.snake.unshift([nx, ny]);
        st.snake.length = Math.min(st.snake.length, len);
      }
      st.snakeMap = new Map();
      st.snake.forEach(([x, y], i) => st.snakeMap.set(y * g.cols + x, 1 - i / (len + 1)));
    },
    audio(c, r, g, now, st) { return st.v[r * g.cols + c]; },
    idle(c, r, g, now, st) { return st.v[r * g.cols + c] * 0.6; },
    tool(c, r, g, now, st) {
      return Math.max(st.snakeMap.get(r * g.cols + c) || 0, st.v[r * g.cols + c]);
    },
  },
};

/** The resting grid: a slow, faint wave across it (still under reduced motion). */
function idleShimmer(c, r, g, now, reduced) {
  if (reduced) return 0.06;
  return 0.1 * (0.5 + 0.5 * Math.sin(now / 900 - c * 0.35 - r * 0.2));
}

/* ── The renderer ────────────────────────────────────────────── */

const reducedMotion = typeof matchMedia === 'function'
  ? matchMedia('(prefers-reduced-motion: reduce)')
  : { matches: false };

/**
 * Mount the dot grid on `host` (its box is the grid's, its `color` the dots').
 * `demo: true` runs the Settings preview's looped exchange instead of the live
 * signal; `effect` / `react` then come from `options()` rather than the stored
 * settings. Returns `{ destroy }`.
 */
export function mountVoiceDots(host, { demo: isDemo = false, options = null } = {}) {
  const canvas = document.createElement('canvas');
  // No 2D canvas (a stubbed DOM, a locked-down renderer): the bar stays a plate.
  const ctx = typeof canvas.getContext === 'function' ? canvas.getContext('2d') : null;
  if (!ctx) return { refresh() {}, destroy() {} };
  canvas.className = 'voice-dots';
  canvas.setAttribute('aria-hidden', 'true');
  host.prepend(canvas);
  const sig = isDemo ? demo() : live();
  const st = { seed: Math.floor(Math.random() * 1e6) };
  let g = null;
  let w = 0;
  let h = 0;
  let wAudio = 0;
  let wTool = 0;
  let color = '#fff';
  let colorAt = 0;
  let lastDraw = 0;
  let raf = 0;
  let prefs = readPrefs();

  function readPrefs() {
    if (options) return options();
    const s = getImmersive();
    return { effect: s.orb_effect, react: s.orb_react };
  }
  const onPrefs = () => { prefs = readPrefs(); };
  if (!options) window.addEventListener('desktop:immersive', onPrefs);

  function layout() {
    const rect = host.getBoundingClientRect();
    w = rect.width;
    h = rect.height;
    if (w < 4 || h < 4) { g = null; return; }
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    // A side rail stands the bar up: the grid runs along its long side.
    const vertical = h > w * 1.2;
    const long = vertical ? h : w;
    const short = vertical ? w : h;
    // Five rows on a thin bar, more as it grows tall — the pitch stays near 7px.
    const rows = Math.max(5, Math.round(short / 7));
    const pitch = short / rows;
    const cols = Math.ceil(long / pitch);
    const grid = {
      vertical, cols, rows, pitch,
      ox: (long - (cols - 1) * pitch) / 2,
      oy: pitch / 2,
      cx: (cols - 1) / 2,
      cy: (rows - 1) / 2,
      perimeter: [],
    };
    for (let c = 0; c < cols; c++) grid.perimeter.push([c, 0]);
    for (let r = 1; r < rows; r++) grid.perimeter.push([cols - 1, r]);
    for (let c = cols - 2; c >= 0; c--) grid.perimeter.push([c, rows - 1]);
    for (let r = rows - 2; r > 0; r--) grid.perimeter.push([0, r]);
    grid.perIndex = new Int16Array(cols * rows).fill(-1);
    grid.perimeter.forEach(([c, r], i) => { grid.perIndex[r * cols + c] = i; });
    g = grid;
  }

  function drive() {
    const toolish = !!sig.tool || sig.stepping || sig.voice === 'processing';
    const audible = AUDIBLE.has(sig.voice);
    if (prefs.react === 'voice') return audible ? 'audio' : 'idle';
    if (prefs.react === 'tools') return toolish ? 'tool' : 'idle';
    return toolish ? 'tool' : audible ? 'audio' : 'idle';
  }

  function frame(now) {
    raf = requestAnimationFrame(frame);
    sig.tick(now);
    if (document.hidden || !g) return;
    const mode = drive();
    const settled = mode === 'idle' && wAudio < 0.01 && wTool < 0.01;
    // At rest the shimmer needs a dozen frames a second, not sixty.
    if (settled && now - lastDraw < (reducedMotion.matches ? 1000 : 80)) return;
    lastDraw = now;
    if (now - colorAt > 500) { color = getComputedStyle(host).color; colorAt = now; }

    const fx = EFFECTS[prefs.effect] || EFFECTS.glyph;
    wAudio += ((mode === 'audio' ? 1 : 0) - wAudio) * 0.12;
    wTool += ((mode === 'tool' ? 1 : 0) - wTool) * 0.12;
    const wIdle = 1 - Math.max(wAudio, wTool);
    if (fx.frame) fx.frame(g, now, st, sig, mode, wAudio, wTool);

    const dpr = window.devicePixelRatio || 1;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = color;
    const rMin = g.pitch * 0.17;
    const rMax = g.pitch * (0.25 + 0.22 * fx.size);
    const reduced = reducedMotion.matches;
    for (let r = 0; r < g.rows; r++) {
      for (let c = 0; c < g.cols; c++) {
        let v = 0;
        if (wIdle > 0.01) {
          const idle = fx.idle ? fx.idle(c, r, g, now, st, sig) : idleShimmer(c, r, g, now, reduced);
          v = Math.max(v, idle * wIdle);
        }
        if (wAudio > 0.01) v = Math.max(v, fx.audio(c, r, g, now, st, sig) * wAudio);
        if (wTool > 0.01) v = Math.max(v, fx.tool(c, r, g, now, st, sig) * wTool);
        const along = g.ox + c * g.pitch;
        const across = g.oy + r * g.pitch;
        ctx.globalAlpha = 0.2 + 0.8 * v;
        ctx.beginPath();
        if (g.vertical) ctx.arc(across, along, rMin + (rMax - rMin) * v, 0, Math.PI * 2);
        else ctx.arc(along, across, rMin + (rMax - rMin) * v, 0, Math.PI * 2);
        ctx.fill();
      }
    }
    ctx.globalAlpha = 1;
  }

  const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(layout) : null;
  ro?.observe(host);
  layout();
  raf = requestAnimationFrame(frame);

  return {
    refresh: onPrefs,
    destroy() {
      cancelAnimationFrame(raf);
      ro?.disconnect();
      if (!options) window.removeEventListener('desktop:immersive', onPrefs);
      canvas.remove();
    },
  };
}

/** Mount the grid on the shell's own voice bar (once, at boot). */
let shellBar = null;
export function initVoiceDots() {
  const bar = document.getElementById('sphere-container');
  if (bar && !shellBar) shellBar = mountVoiceDots(bar);
}
