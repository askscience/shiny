/**
 * orbCanvas3d — the voice orb rendered with three.js (WebGL).
 *
 * Every look is a variant of one idea, taken from the reference render in
 * `web/orbs/eclipse.jpg`: a thin ring of spectral light on black.
 *
 *   corona — the hairline ring throwing fine sparks outward     (default)
 *   halo   — the ring itself rippling as a smooth radial wave
 *   ripple — echoes that spread outward from the ring
 *   flare  — a crown of long rays growing out of the ring
 *   aura   — two rings riding a wave in and out of the screen
 *
 * Nothing rotates and the ring always faces you: the voice is what moves the
 * light. `intensity` (0..1) grows the waves and the spikes, `pan` (-1..1)
 * squashes the whole group away from the loud side.
 *
 * No post-processing pass: additive materials keep the canvas genuinely
 * transparent so the desktop shows through (a composer would flatten the alpha
 * to black). Ring colours come from `spectrumForState()`, so the fan follows
 * the user's accent (and falls back to red / grey on error and disabled).
 */

import { spectrumForState, isLightCanvas } from './orbPalette.js';
import { DEFAULT_ORB_STYLE } from './preferences.js';

let THREE = null;
let threePromise = null;

/** Load the vendored three.js build (local file — the app stays offline-first). */
function loadThree() {
  if (THREE) return Promise.resolve(THREE);
  if (!threePromise) {
    threePromise = import('../vendor/three/three.module.min.js')
      .then((m) => {
        THREE = m;
        return m;
      })
      .catch((err) => {
        threePromise = null;
        throw err;
      });
  }
  return threePromise;
}

const TAU = Math.PI * 2;
const DEFAULT_STYLE = DEFAULT_ORB_STYLE;

/* ── ring helpers (plain maths + geometry, no THREE at module scope) ── */

/**
 * Paint a torus so its colour sweeps once around the ring. Torus vertices
 * carry the angle of their own centreline point in x/y, so the sweep needs no
 * extra bookkeeping — and the colours live in the geometry's local space, so
 * the ring can still be deformed later without repainting.
 */
function paintGradientRing(geo, colors) {
  const pos = geo.attributes.position;
  const arr = new Float32Array(pos.count * 3);
  const c = new THREE.Color();
  const next = new THREE.Color();
  const n = colors.length;
  for (let i = 0; i < pos.count; i++) {
    const ang = Math.atan2(pos.getY(i), pos.getX(i));
    const u = ((ang / TAU) % 1 + 1) % 1;
    const f = u * n;
    const i0 = Math.floor(f) % n;
    const i1 = (i0 + 1) % n;
    c.set(colors[i0]).lerp(next.set(colors[i1]), f - Math.floor(f));
    arr[i * 3] = c.r;
    arr[i * 3 + 1] = c.g;
    arr[i * 3 + 2] = c.b;
  }
  geo.setAttribute('color', new THREE.BufferAttribute(arr, 3));
  geo.attributes.color.needsUpdate = true;
}

/** Lay a spike out along its own direction: `len` running outward from the
 *  ring it sits on. The cone's own geometry is a unit length, so the scale is
 *  the length and the position keeps its base on the ring. */
function placeSpike(part, len) {
  const l = Math.max(0.0005, len);
  const d = part.spike.radius + l * 0.5;
  part.obj.scale.set(1, l, 1);
  part.obj.position.set(part.spike.dir[0] * d, part.spike.dir[1] * d, 0);
}

export class Orb3DRenderer {
  constructor(canvas, { size = 84, preview = false, style = null, renderer = null } = {}) {
    this.canvas = canvas;
    this.displaySize = size;
    this.preview = preview;
    this.styleId = STYLES[style] ? style : DEFAULT_STYLE;
    this.stateKey = 'idle';
    // Light themes get ink on paper rather than light on black.
    this.additive = !isLightCanvas();

    this.intensity = 0;
    this.pan = 0;
    this.targetLevel = 0;
    this.targetPan = 0;
    this.t = 0;
    this.running = true;

    this.parts = [];     // { obj, kind, baseOpacity, spectrumIndex, pulse }
    this.pulses = [];    // the shared "thinking" rings (rebuilt per style)
    this.animate = null; // per-style per-frame hook: (energy, t, thinking) => void

    // `renderer` is injectable so the scene graph can be exercised headlessly.
    this.renderer = renderer || new THREE.WebGLRenderer({
      canvas,
      alpha: true,
      antialias: !preview,
      powerPreference: preview ? 'low-power' : 'high-performance',
    });
    this.renderer.setClearColor(0x000000, 0);
    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(38, 1, 0.1, 60);
    // Close enough that a radius-1 ring nearly fills the canvas, like the
    // reference render.
    this.camera.position.set(0, 0, 3.0);
    this.root = new THREE.Group();
    this.scene.add(this.root);

    this.resize();
    this.build();
    this.applyColors();

    this._onResize = () => this.resize();
    window.addEventListener('resize', this._onResize);
    this._loop = this._loop.bind(this);
    requestAnimationFrame(this._loop);
  }

  resize() {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    this.dpr = dpr;
    this.renderer.setPixelRatio(dpr);
    this.renderer.setSize(this.displaySize, this.displaySize, false);
    this.canvas.style.width = `${this.displaySize}px`;
    this.canvas.style.height = `${this.displaySize}px`;
  }

  /* ── building blocks ───────────────────────────────────────── */

  _basicMaterial(opacity) {
    return new THREE.MeshBasicMaterial({
      color: 0xffffff,
      transparent: true,
      opacity,
      // Additive light is right on black, where overlapping passes read as
      // glow. On paper it would sum the ink back up towards white, so a light
      // canvas composites normally instead.
      blending: this.additive ? THREE.AdditiveBlending : THREE.NormalBlending,
      depthWrite: false,
      depthTest: false,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
  }

  _track(obj, kind, baseOpacity) {
    this.root.add(obj);
    const part = { obj, kind, baseOpacity };
    this.parts.push(part);
    return part;
  }

  /** A torus whose colour runs once around the ring. */
  _addRing(radius, tube, { opacity = 0.5, radial = 8, tubular = 220 } = {}) {
    const geo = new THREE.TorusGeometry(radius, tube, radial, tubular);
    const mat = this._basicMaterial(opacity);
    mat.vertexColors = true;
    const mesh = new THREE.Mesh(geo, mat);
    const part = this._track(mesh, 'ring', opacity);
    paintGradientRing(geo, spectrumForState(this.stateKey));
    return part;
  }

  /** The ring itself: a soft halo pass behind a bright hairline, both at the
   *  same radius so they read as one ring. */
  _addRingBody(radius, { soft = 0.045, softOpacity = 0.22, core = 0.014, coreOpacity = 0.95 } = {}) {
    const parts = [];
    if (soft > 0) parts.push(this._addRing(radius, soft, { opacity: softOpacity, tubular: 180 }));
    parts.push(this._addRing(radius, core, { opacity: coreOpacity, tubular: 280 }));
    return parts;
  }

  /** Spikes radiating outward from the ring, evenly spaced. */
  _addSpikes(count, { radius, width = 0.012, opacity = 0.6, offset = 0, spectrum = 8 } = {}) {
    const up = new THREE.Vector3(0, 1, 0);
    const parts = [];
    for (let i = 0; i < count; i++) {
      const th = offset + (i / count) * TAU;
      const dir = [Math.cos(th), Math.sin(th), 0];
      const geo = new THREE.ConeGeometry(width, 1, 6, 1, true);
      const mesh = new THREE.Mesh(geo, this._basicMaterial(opacity));
      mesh.quaternion.setFromUnitVectors(up, new THREE.Vector3(dir[0], dir[1], dir[2]));
      const part = this._track(mesh, 'spike', opacity);
      part.spectrumIndex = i % spectrum;
      part.spike = { dir, radius };
      placeSpike(part, 0);
      parts.push(part);
    }
    return parts;
  }

  /** A per-frame wave that travels around a ring, in its own plane. */
  _waveRing(part, { waves = 6, amp = 0.08, speed = 1.5 } = {}) {
    const pos = part.obj.geometry.attributes.position;
    const base = Float32Array.from(pos.array);
    return (energy, t) => {
      const a = amp * energy;
      const phase = t * speed;
      const arr = pos.array;
      for (let i = 0; i < pos.count; i++) {
        const ix = i * 3;
        const x = base[ix];
        const y = base[ix + 1];
        const k = 1 + a * Math.sin(waves * Math.atan2(y, x) + phase);
        arr[ix] = x * k;
        arr[ix + 1] = y * k;
      }
      pos.needsUpdate = true;
    };
  }

  /** A per-frame wave that pushes a ring in and out of the screen. */
  _ribbonRing(part, { waves = 4, amp = 0.2, speed = 1.2, phase = 0 } = {}) {
    const pos = part.obj.geometry.attributes.position;
    const base = Float32Array.from(pos.array);
    return (energy, t) => {
      const a = amp * energy;
      const p = phase + t * speed;
      const arr = pos.array;
      for (let i = 0; i < pos.count; i++) {
        const ix = i * 3;
        const x = base[ix];
        const y = base[ix + 1];
        arr[ix] = x;
        arr[ix + 1] = y;
        arr[ix + 2] = base[ix + 2] + a * Math.sin(waves * Math.atan2(y, x) + p);
      }
      pos.needsUpdate = true;
    };
  }

  /* ── styles ────────────────────────────────────────────────── */

  build() {
    for (const p of this.parts) {
      p.obj.geometry?.dispose?.();
      p.obj.material?.dispose?.();
    }
    this.root.clear();
    this.parts = [];
    const build = STYLES[this.styleId] || STYLES[DEFAULT_STYLE];
    this.animate = build.call(this) || null;
    // The shared "thinking" pulses: small rings that grow through the orb and
    // fade, on top of whatever the style does. Every look gets them.
    this.pulses = [0, 1].map(() => {
      const part = this._addRing(0.55, 0.006, { opacity: 0, tubular: 180 });
      part.pulse = true;
      part.obj.visible = false;
      return part;
    });
  }

  setStyle(id) {
    const next = STYLES[id] ? id : DEFAULT_STYLE;
    if (next === this.styleId) return;
    this.styleId = next;
    this.build();
    this.applyColors();
  }

  getStyle() {
    return this.styleId;
  }

  setPalette(state) {
    this.stateKey = state;
    this.applyColors();
  }

  refreshPalette() {
    this.setPalette(this.stateKey);
  }

  applyColors() {
    const spectrum = spectrumForState(this.stateKey);
    for (const p of this.parts) {
      if (p.kind === 'ring') {
        paintGradientRing(p.obj.geometry, spectrum);
      } else if (p.spectrumIndex != null) {
        p.obj.material.color.set(spectrum[p.spectrumIndex % spectrum.length]);
      }
    }
  }

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
    for (const p of this.parts) {
      p.obj.geometry?.dispose?.();
      p.obj.material?.dispose?.();
    }
    this.parts = [];
    this.renderer.dispose?.();
  }

  _loop(ts) {
    if (!this.running) return;
    this.t = ts * 0.001;

    if (this.preview) {
      // Settings previews breathe with a synthetic voice, always centred.
      this.targetLevel = 0.38 + 0.24 * Math.sin(this.t * 0.9);
      this.targetPan = 0;
    }
    const k = this.targetLevel > this.intensity ? 0.45 : 0.1;
    this.intensity += (this.targetLevel - this.intensity) * k;
    this.pan += (this.targetPan - this.pan) * 0.16;

    const e = this.intensity;
    const active = this.stateKey === 'listening' || this.stateKey === 'conversation'
      || this.stateKey === 'processing' || this.stateKey === 'speaking';
    const energy = Math.min(1, e + (active ? 0.2 : 0));

    // Waiting for the assistant: there is no voice to react to, so the orb runs
    // its own slow rhythm (the ring breathes and the style's own gesture keeps
    // moving) and pulses keep leaving it. Shared by every look.
    const thinking = this.stateKey === 'processing';
    const rhythm = Math.sin(this.t * 2.4);
    const drive = thinking ? Math.max(energy, 0.34 + 0.16 * rhythm) : energy;
    const breathe = thinking ? 1 + 0.16 * rhythm : 1;

    // No rotation — the ring always faces you. Stereo still squashes the body
    // and leans it away from the loud side.
    const side = Math.abs(this.pan);
    const squish = Math.min(0.42, side * (0.22 + 0.5 * e));
    this.root.scale.set(1 - squish, 1 + squish * 0.45, 1);
    this.root.position.x = -this.pan * (0.08 + 0.18 * e);

    for (const p of this.parts) {
      if (p.pulse) continue; // driven by _updatePulses instead
      p.obj.material.opacity = Math.min(1, p.baseOpacity * (0.9 + 0.3 * energy) * breathe);
    }

    // The voice moves the light: waves and spikes, never rotation.
    this.animate?.(drive, this.t, thinking);
    this._updatePulses(thinking);

    this.renderer.render(this.scene, this.camera);
    requestAnimationFrame(this._loop);
  }

  /** The "thinking" pulses: soft rings that leave the orb and fade, so the orb
   *  reads as busy while the assistant works. Silent at every other state. */
  _updatePulses(thinking) {
    for (let i = 0; i < this.pulses.length; i++) {
      const p = this.pulses[i];
      if (!thinking) {
        if (p.obj.visible) {
          p.obj.visible = false;
          p.obj.material.opacity = 0;
        }
        continue;
      }
      const f = (this.t * 0.5 + i / this.pulses.length) % 1;
      const s = 1 + f * 0.85;
      p.obj.visible = true;
      p.obj.scale.set(s, s, 1);
      p.obj.material.opacity = (1 - f) * (1 - f) * 0.6;
    }
  }
}

/* ── per-style scenes ────────────────────────────────────────── */

const STYLES = {
  /** Corona — the reference ring, throwing fine sparks outward on sound. */
  corona() {
    const radius = 0.84;
    this._addRingBody(radius);
    const sparks = this._addSpikes(64, { radius, width: 0.014, opacity: 0.6 });
    return (energy, t) => {
      for (let i = 0; i < sparks.length; i++) {
        const p = sparks[i];
        const flicker = 0.5 + 0.5 * Math.sin(t * 2.6 + i * 1.9);
        placeSpike(p, 0.22 * energy * flicker);
        p.obj.material.opacity = Math.min(1, energy * 1.6 * flicker);
      }
    };
  },

  /** Halo — the ring itself rippling as a smooth radial wave. */
  halo() {
    const radius = 0.82;
    const ring = this._addRingBody(radius, { soft: 0.06, softOpacity: 0.2, core: 0.015 });
    const waves = ring.map((p) => this._waveRing(p, { waves: 9, amp: 0.07, speed: 1.7 }));
    return (energy, t) => {
      for (const wave of waves) wave(energy, t);
    };
  },

  /** Ripple — echoes that spread outward from the ring while you speak. */
  ripple() {
    const radius = 0.82;
    this._addRingBody(radius);
    const echoes = [0, 1, 2].map(
      () => this._addRing(radius, 0.009, { opacity: 0, radial: 5, tubular: 200 }),
    );
    return (energy, t) => {
      for (let i = 0; i < echoes.length; i++) {
        const p = echoes[i];
        const f = (t * 0.5 + i / echoes.length) % 1;
        const grow = 1 + f * 0.45;
        p.obj.scale.set(grow, grow, 1);
        p.obj.material.opacity = Math.min(1, energy * 1.3) * (1 - f) * 0.9;
      }
    };
  },

  /** Flare — a crown of long rays growing out of the ring. */
  flare() {
    const radius = 0.8;
    this._addRingBody(radius, { soft: 0.06, softOpacity: 0.22, core: 0.015 });
    const rays = this._addSpikes(14, { radius: radius - 0.02, width: 0.05, opacity: 0.5 });
    return (energy) => {
      for (const p of rays) {
        placeSpike(p, 0.5 * energy);
        p.obj.material.opacity = Math.min(1, energy * 1.15);
      }
    };
  },

  /** Aura — two rings riding a wave in and out of the screen, counter-phase. */
  aura() {
    const inner = this._addRingBody(0.72, { soft: 0.04, softOpacity: 0.18, core: 0.013 });
    const outer = this._addRingBody(0.93, { soft: 0.04, softOpacity: 0.18, core: 0.013 });
    const waves = [
      ...inner.map((p) => this._ribbonRing(p, { waves: 3, amp: 0.11, speed: 1.2, phase: 0 })),
      ...outer.map((p) => this._ribbonRing(p, { waves: 3, amp: 0.11, speed: 1.2, phase: Math.PI })),
    ];
    return (energy, t) => {
      for (const wave of waves) wave(energy, t);
    };
  },
};

/** Async factory: loads three.js, then builds the renderer. */
export async function createRenderer(canvas, opts = {}) {
  await loadThree();
  return new Orb3DRenderer(canvas, opts);
}

/** Cheap availability probe so the facade can fall back before committing. */
export function probe() {
  return loadThree();
}
