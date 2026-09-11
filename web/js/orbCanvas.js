/**
 * orbCanvas — the voice orb façade.
 *
 * The orb is rendered with three.js (WebGL) from `orbCanvas3d.js`, which is
 * what matches the reference look. If three.js or WebGL is unavailable the
 * façade silently falls back to the canvas-2D implementation in
 * `orbCanvas2d.js`, so a broken/absent GPU never costs you the assistant.
 *
 * It also owns the orb's shared state (palette, signal, style) so a renderer
 * that resolves asynchronously is created already knowing what to show.
 *
 * Renderer contract (both implementations):
 *   setPalette(state) setSignal({ level, pan }) setStyle(id)
 *   refreshPalette()  destroy()
 */

import { getOrbStyle } from './preferences.js';

const state = {
  palette: 'idle',
  level: 0,
  pan: 0,
  style: getOrbStyle(),
};

let implPromise = null;
let active = null;

function resolveImpl() {
  if (!implPromise) {
    implPromise = import('./orbCanvas3d.js')
      .then((m) => m.probe().then(() => m))
      .catch((err) => {
        console.warn('[orb] three.js unavailable — using the 2D orb.', err);
        return null;
      });
  }
  return implPromise;
}

function makeHandle() {
  const handle = {
    renderer: null,
    destroyed: false,
    palette: state.palette,
    apply(r) {
      r.setPalette?.(handle.palette);
      r.setSignal?.({ level: state.level, pan: state.pan });
    },
    refreshPalette() {
      handle.renderer?.setPalette?.(handle.palette);
    },
    setPalette(next) {
      handle.palette = next;
      handle.renderer?.setPalette?.(next);
    },
    destroy() {
      handle.destroyed = true;
      handle.renderer?.destroy?.();
      handle.renderer = null;
    },
  };
  return handle;
}

async function make2d(canvas, opts) {
  const m2 = await import('./orbCanvas2d.js');
  return m2.createRenderer(canvas, opts);
}

function attach(handle, canvas, opts) {
  resolveImpl()
    .then((m3) => (m3 ? m3.createRenderer(canvas, opts) : make2d(canvas, opts)))
    .catch(async (err) => {
      console.warn('[orb] 3D renderer failed — falling back to 2D.', err);
      return make2d(canvas, opts);
    })
    .then((r) => {
      if (!r) return;
      if (handle.destroyed) {
        r.destroy?.();
        return;
      }
      handle.renderer = r;
      handle.apply(r);
    })
    .catch((err) => console.warn('[orb] no orb renderer available.', err));
  return handle;
}

/** Boot the app's single orb. Returns immediately; the renderer attaches when
 *  the 3D module has loaded. */
export function initOrbCanvas(canvas) {
  if (active) active.destroy();
  active = makeHandle();
  return attach(active, canvas, { size: 84, style: state.style });
}

/** A self-animating preview card (settings page). One per style. */
export function createOrbPreview(canvas, styleId, size = 52) {
  const handle = makeHandle();
  return attach(handle, canvas, { size, preview: true, style: styleId });
}

export function setOrbPalette(next) {
  state.palette = next;
  if (active) active.palette = next;
  active?.renderer?.setPalette?.(next);
}

export function setOrbIntensity(v) {
  if (typeof v === 'number') {
    state.level = Math.min(1, Math.max(0, v));
    state.pan = 0;
  } else if (v && typeof v === 'object') {
    const level = Number(v.level);
    const pan = Number(v.pan);
    if (Number.isFinite(level)) state.level = Math.min(1, Math.max(0, level));
    if (Number.isFinite(pan)) state.pan = Math.min(1, Math.max(-1, pan));
  }
  active?.renderer?.setSignal?.({ level: state.level, pan: state.pan });
}

export function resetOrbIntensity() {
  setOrbIntensity({ level: 0, pan: 0 });
}

export function setOrbStyle(id) {
  state.style = id;
  active?.renderer?.setStyle?.(id);
}

export function getOrbStyleId() {
  return state.style;
}
