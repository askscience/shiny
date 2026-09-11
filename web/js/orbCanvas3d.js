/**
 * orbCanvas3d — the voice orb rendered with three.js (WebGL).
 *
 * Built to match the reference renders: luminous structures suspended inside a
 * glass sphere, glowing additively over whatever is behind the window.
 *
 *   filament — a ball of light trails (tubes around the sphere) + one ribbon
 *   bubble   — a hollow glass shell whose light lives in a Fresnel rim
 *   marble   — a glass sphere with luminous ribbons caught inside
 *   orbit    — tilted tori around a bright core, each carrying a bead
 *   grid     — a lat/long wireframe of light
 *
 * No post-processing pass: additive materials plus soft sprite halos give the
 * bloom, and keep the canvas genuinely transparent so the desktop shows
 * through (a composer would flatten the alpha to black).
 *
 * Reactivity is shared with the 2D fallback: `intensity` (0..1) and `pan`
 * (-1..1). Pan squashes the whole group away from the loud side; intensity
 * drives brightness, rotation speed and halo strength.
 */

import { paletteForState } from './orbPalette.js';

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
const DEFAULT_STYLE = 'filament';

/* ── geometry helpers (plain maths, no THREE needed) ─────────── */

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

function basisFor(n) {
  const helper = Math.abs(n[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
  const u = normalize(cross(n, helper));
  return { u, v: cross(n, u) };
}

/** Points of a great circle (or a smaller circle) around `normal`. */
function circlePoints(normal, count = 64, radius = 1) {
  const { u, v } = basisFor(normal);
  const pts = [];
  for (let i = 0; i < count; i++) {
    const th = (i / count) * TAU;
    const c = Math.cos(th);
    const s = Math.sin(th);
    pts.push([
      (u[0] * c + v[0] * s) * radius,
      (u[1] * c + v[1] * s) * radius,
      (u[2] * c + v[2] * s) * radius,
    ]);
  }
  return pts;
}

/** Golden-angle normals — an evenly spread ball of strands. */
function strandNormals(n) {
  const out = [];
  for (let i = 0; i < n; i++) {
    const y = 1 - (i / Math.max(1, n - 1)) * 2;
    const r = Math.sqrt(Math.max(0, 1 - y * y));
    const phi = i * 2.399963229728653;
    out.push([Math.cos(phi) * r, y, Math.sin(phi) * r]);
  }
  return out;
}

/* ── sprite halo texture ─────────────────────────────────────── */

let HALO_TEX = null;

function haloTexture() {
  if (HALO_TEX) return HALO_TEX;
  const c = document.createElement('canvas');
  c.width = 128;
  c.height = 128;
  const g = c.getContext('2d');
  const rg = g.createRadialGradient(64, 64, 0, 64, 64, 64);
  rg.addColorStop(0, 'rgba(255,255,255,1)');
  rg.addColorStop(0.22, 'rgba(255,255,255,0.5)');
  rg.addColorStop(0.5, 'rgba(255,255,255,0.16)');
  rg.addColorStop(0.78, 'rgba(255,255,255,0.04)');
  rg.addColorStop(1, 'rgba(255,255,255,0)');
  g.fillStyle = rg;
  g.fillRect(0, 0, 128, 128);
  HALO_TEX = new THREE.CanvasTexture(c);
  return HALO_TEX;
}

/* ── materials ───────────────────────────────────────────────── */

const FRESNEL_VERT = `
varying vec3 vNormalV;
varying vec3 vViewDir;
void main() {
  // View-space normal + view direction: no mat3(mat4) cast (illegal in
  // GLSL ES 1.00) and normalMatrix is injected by three for us.
  vec4 mv = modelViewMatrix * vec4(position, 1.0);
  vNormalV = normalize(normalMatrix * normal);
  vViewDir = normalize(-mv.xyz);
  gl_Position = projectionMatrix * mv;
}
`;

const FRESNEL_FRAG = `
uniform vec3 uColor;
uniform vec3 uRim;
uniform float uPower;
uniform float uIntensity;
uniform float uBias;
varying vec3 vNormalV;
varying vec3 vViewDir;
void main() {
  // Fresnel: dark through the middle, blazing at the silhouette.
  float rim = clamp(1.0 - abs(dot(normalize(vNormalV), normalize(vViewDir))), 0.0, 1.0);
  float f = pow(rim, uPower);
  vec3 c = mix(uColor, uRim, clamp(f * 1.5, 0.0, 1.0));
  float a = clamp(f * uIntensity + uBias, 0.0, 1.0);
  gl_FragColor = vec4(c, a);
}
`;

export class Orb3DRenderer {
  constructor(canvas, { size = 84, preview = false, style = null, renderer = null } = {}) {
    this.canvas = canvas;
    this.displaySize = size;
    this.preview = preview;
    this.styleId = STYLES[style] ? style : DEFAULT_STYLE;
    this.palette = paletteForState('idle');
    this.stateKey = 'idle';

    this.intensity = 0;
    this.pan = 0;
    this.targetLevel = 0;
    this.targetPan = 0;
    this.t = 0;
    this.running = true;

    this.parts = [];   // { obj, kind, baseOpacity, colorIndex, rimIndex }
    this.spinners = []; // { group, speed }

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
    this.camera.position.set(0, 0, 3.3);
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
      blending: THREE.AdditiveBlending,
      depthWrite: false,
      depthTest: false,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
  }

  _track(obj, kind, baseOpacity, colorIndex, rimIndex) {
    this.root.add(obj);
    const part = { obj, kind, baseOpacity, colorIndex, rimIndex };
    this.parts.push(part);
    return part;
  }

  /** A glowing tube through `points` (a closed loop by default). */
  _addTube(points, { radius = 0.006, opacity = 0.5, colorIndex = 0, closed = true, tubular = 72 } = {}) {
    const curve = new THREE.CatmullRomCurve3(
      points.map((p) => new THREE.Vector3(p[0], p[1], p[2])),
      closed,
      'catmullrom',
      0.5,
    );
    const geo = new THREE.TubeGeometry(curve, tubular, radius, 6, closed);
    const mesh = new THREE.Mesh(geo, this._basicMaterial(opacity));
    return this._track(mesh, 'basic', opacity, colorIndex);
  }

  /** A Fresnel shell: transparent in the middle, blazing at the silhouette. */
  _addShell(radius, { power = 2.6, intensity = 0.8, bias = 0, colorIndex = 0, rimIndex = 1 } = {}) {
    const geo = new THREE.SphereGeometry(radius, 48, 32);
    const mat = new THREE.ShaderMaterial({
      uniforms: {
        uColor: { value: new THREE.Color('#ffffff') },
        uRim: { value: new THREE.Color('#ffffff') },
        uPower: { value: power },
        uIntensity: { value: intensity },
        uBias: { value: bias },
      },
      vertexShader: FRESNEL_VERT,
      fragmentShader: FRESNEL_FRAG,
      transparent: true,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
      depthTest: false,
      side: THREE.DoubleSide,
    });
    const mesh = new THREE.Mesh(geo, mat);
    return this._track(mesh, 'shell', intensity, colorIndex, rimIndex);
  }

  /** A soft additive halo (bloom). `hex` overrides the palette colour. */
  _addHalo(scale, opacity, colorIndex = 0, position = [0, 0, 0], hex = null) {
    const mat = new THREE.SpriteMaterial({
      map: haloTexture(),
      color: new THREE.Color(hex || this.palette[colorIndex % this.palette.length]),
      transparent: true,
      opacity,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
      depthTest: false,
      toneMapped: false,
    });
    const sprite = new THREE.Sprite(mat);
    sprite.scale.set(scale, scale, 1);
    sprite.position.set(position[0], position[1], position[2]);
    const part = this._track(sprite, 'sprite', opacity, colorIndex);
    if (hex) part.lockedColor = hex;
    return part;
  }

  /** A tilted torus that can spin a bead around itself. */
  _addRing(radius, tube, rotation, { opacity = 0.5, colorIndex = 0, bead = 0 } = {}) {
    const group = new THREE.Group();
    group.rotation.set(rotation[0], rotation[1], rotation[2]);
    this.root.add(group);

    const geo = new THREE.TorusGeometry(radius, tube, 8, 128);
    const mesh = new THREE.Mesh(geo, this._basicMaterial(opacity));
    group.add(mesh);
    this.parts.push({ obj: mesh, kind: 'basic', baseOpacity: opacity, colorIndex });

    if (bead > 0) {
      const beadGeo = new THREE.SphereGeometry(bead, 16, 12);
      const beadMesh = new THREE.Mesh(beadGeo, this._basicMaterial(0.95));
      beadMesh.position.set(radius, 0, 0);
      group.add(beadMesh);
      this.parts.push({ obj: beadMesh, kind: 'basic', baseOpacity: 0.95, colorIndex: colorIndex + 1 });
      this.spinners.push({ group, speed: 0.35 + colorIndex * 0.12 });
    }
    return group;
  }

  /* ── styles ────────────────────────────────────────────────── */

  build() {
    for (const p of this.parts) {
      p.obj.geometry?.dispose?.();
      p.obj.material?.dispose?.();
    }
    // Detach everything (ring groups included) before rebuilding.
    this.root.clear();
    this.parts = [];
    this.spinners = [];
    const build = STYLES[this.styleId] || STYLES[DEFAULT_STYLE];
    build.call(this);
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
    this.palette = paletteForState(state);
    this.applyColors();
  }

  refreshPalette() {
    this.setPalette(this.stateKey);
  }

  applyColors() {
    for (const p of this.parts) {
      if (p.lockedColor) continue;
      const color = this.palette[p.colorIndex % this.palette.length];
      if (p.kind === 'shell') {
        p.obj.material.uniforms.uColor.value.set(color);
        const rim = this.palette[(p.rimIndex ?? p.colorIndex + 1) % this.palette.length];
        p.obj.material.uniforms.uRim.value.set(rim);
      } else {
        p.obj.material.color.set(color);
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
      // Synthesize a voice so every style shows off in the settings grid.
      this.targetLevel = 0.42 + 0.34 * Math.sin(this.t * 1.15);
      this.targetPan = Math.sin(this.t * 0.42);
    }
    const k = this.targetLevel > this.intensity ? 0.45 : 0.1;
    this.intensity += (this.targetLevel - this.intensity) * k;
    this.pan += (this.targetPan - this.pan) * 0.16;

    const e = this.intensity;
    const active = this.stateKey === 'listening' || this.stateKey === 'conversation'
      || this.stateKey === 'processing' || this.stateKey === 'speaking';
    const boost = active ? 0.2 : 0;
    const energy = Math.min(1, e + boost);

    // Slow tumble; the voice makes it a touch more alive.
    this.root.rotation.y = this.t * (0.14 + 0.3 * energy);
    this.root.rotation.x = Math.sin(this.t * 0.17) * 0.16;

    // Stereo: squash the body and lean away from the loud side.
    const side = Math.abs(this.pan);
    const squish = Math.min(0.42, side * (0.22 + 0.5 * e));
    this.root.scale.set(1 - squish, 1 + squish * 0.45, 1);
    this.root.position.x = -this.pan * (0.08 + 0.18 * e);

    for (const s of this.spinners) {
      s.group.rotation.z += 0.006 * s.speed * (1 + energy);
    }

    for (const p of this.parts) {
      const factor = p.kind === 'sprite'
        ? 0.45 + 1.0 * energy
        : 0.72 + 0.6 * energy;
      if (p.kind === 'shell') {
        p.obj.material.uniforms.uIntensity.value = p.baseOpacity * factor;
      } else {
        p.obj.material.opacity = Math.min(1, p.baseOpacity * factor);
      }
    }

    this.renderer.render(this.scene, this.camera);
    requestAnimationFrame(this._loop);
  }
}

/* ── per-style scenes ────────────────────────────────────────── */

const STYLES = {
  /** A ball of light trails inside glass, with one bright ribbon. */
  filament() {
    const norms = strandNormals(16);
    for (let i = 0; i < norms.length; i++) {
      this._addTube(circlePoints(norms[i], 64), {
        radius: 0.0045,
        opacity: 0.42,
        colorIndex: i % 3,
        tubular: 90,
      });
    }
    // The ribbon: a wide soft pass and a bright thin core.
    const ribbon = circlePoints(norms[4], 96);
    this._addTube(ribbon, { radius: 0.05, opacity: 0.16, colorIndex: 2, tubular: 120 });
    this._addTube(ribbon, { radius: 0.012, opacity: 0.6, colorIndex: 3, tubular: 120 });
    this._addShell(1.02, { power: 1.5, intensity: 0.12, colorIndex: 0, rimIndex: 2 });
    this._addHalo(3.0, 0.22, 0);
  },

  /** A hollow glass shell: light lives in the rim. */
  bubble() {
    this._addShell(0.94, { power: 3.2, intensity: 0.95, colorIndex: 0, rimIndex: 1 });
    this._addShell(0.9, { power: 1.6, intensity: 0.26, colorIndex: 1, rimIndex: 2 });
    this._addHalo(2.7, 0.15, 0);
    this._addHalo(0.9, 0.5, 0, [-0.42, 0.44, 0.3], '#ffffff');
    this._addHalo(1.1, 0.22, 2, [-0.3, -0.5, 0.4]);
  },

  /** Glass with luminous ribbons and filaments caught inside. */
  marble() {
    this._addShell(0.98, { power: 2.4, intensity: 0.65, colorIndex: 0, rimIndex: 2 });
    const norms = strandNormals(3);
    for (let i = 0; i < norms.length; i++) {
      const pts = circlePoints(norms[i], 64, 0.62 + i * 0.08);
      this._addTube(pts, { radius: 0.022, opacity: 0.34, colorIndex: i + 1, tubular: 90 });
      this._addTube(pts, { radius: 0.006, opacity: 0.5, colorIndex: i, tubular: 90 });
    }
    // A few fine filaments through the glass.
    for (let lat = -50; lat <= 50; lat += 50) {
      const rad = (lat * Math.PI) / 180;
      const r = Math.cos(rad) * 0.92;
      const y = Math.sin(rad) * 0.92;
      const pts = [];
      for (let i = 0; i < 64; i++) {
        const th = (i / 64) * TAU;
        pts.push([Math.cos(th) * r, y, Math.sin(th) * r]);
      }
      this._addTube(pts, { radius: 0.0035, opacity: 0.22, colorIndex: 0, tubular: 80 });
    }
    this._addHalo(2.4, 0.14, 2);
    this._addHalo(1.3, 0.22, 1, [0, -0.5, 0.35]);
  },

  /** Tilted luminous rings around a bright core, each with a bead. */
  orbit() {
    this._addHalo(1.9, 0.3, 0);
    this._addShell(0.34, { power: 1.2, intensity: 0.9, colorIndex: 3, rimIndex: 0 });
    this._addRing(0.78, 0.006, [Math.PI / 2.3, 0, 0.3], { opacity: 0.5, colorIndex: 1, bead: 0.052 });
    this._addRing(0.9, 0.005, [Math.PI / 1.7, 0.45, -0.2], { opacity: 0.42, colorIndex: 2, bead: 0.046 });
    this._addRing(0.64, 0.005, [Math.PI / 3.0, -0.3, 0.6], { opacity: 0.46, colorIndex: 0, bead: 0.04 });
  },

  /** A lat/long wireframe of light with a bright equator. */
  grid() {
    // Latitudes.
    for (let lat = -60; lat <= 60; lat += 30) {
      const rad = (lat * Math.PI) / 180;
      const r = Math.cos(rad);
      const y = Math.sin(rad);
      const pts = [];
      for (let i = 0; i < 64; i++) {
        const th = (i / 64) * TAU;
        pts.push([Math.cos(th) * r, y, Math.sin(th) * r]);
      }
      this._addTube(pts, { radius: 0.004, opacity: 0.3, colorIndex: 0, tubular: 80 });
    }
    // Longitudes (great circles through the poles).
    for (let lon = 0; lon < 180; lon += 30) {
      this._addTube(circlePoints([Math.cos((lon * Math.PI) / 180), 0, Math.sin((lon * Math.PI) / 180)], 64), {
        radius: 0.0035,
        opacity: 0.26,
        colorIndex: 1,
        tubular: 80,
      });
    }
    // Bright equator seam.
    this._addTube(circlePoints([0, 1, 0], 96), { radius: 0.012, opacity: 0.6, colorIndex: 3, tubular: 120 });
    this._addShell(1.0, { power: 2.0, intensity: 0.16, colorIndex: 0, rimIndex: 2 });
    this._addHalo(2.3, 0.13, 2);
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
