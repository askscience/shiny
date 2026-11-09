/**
 * waveform — draw a min/max peak envelope into a canvas.
 *
 * Peaks come in either shape the app produces:
 *   - `[[min, max], …]` floats in -1..1 over the whole media (Studio), or
 *   - `{ rate, min: Int8[], max: Int8[] }`, `rate` buckets a second (Capture).
 *
 *   drawWaveform(canvas, { peaks, duration, from: 2, to: 9.5, gain: 1 });
 *
 * One column per CSS pixel, each the extreme of the buckets it covers, so a
 * transient never disappears at any zoom. The ink comes from the canvas's
 * own `color` (CSS), so a theme or a selected state recolours it for free.
 */

function normalize(peaks, duration) {
  if (!peaks) return null;
  if (Array.isArray(peaks)) {
    const n = peaks.length;
    return { n, rate: duration > 0 ? n / duration : n, min: (k) => peaks[k][0], max: (k) => peaks[k][1] };
  }
  if (peaks.min && peaks.max) {
    const n = Math.min(peaks.min.length, peaks.max.length);
    return { n, rate: peaks.rate || 100, min: (k) => peaks.min[k] / 127, max: (k) => peaks.max[k] / 127 };
  }
  return null;
}

/**
 * @param {HTMLCanvasElement} canvas
 * @param {object} o
 * @param {Array|object} o.peaks
 * @param {number} [o.duration]  media length (seconds) for `[[min,max]]` peaks
 * @param {number} [o.from]      media seconds at the left edge
 * @param {number} [o.to]        media seconds at the right edge
 * @param {number} [o.gain]      linear gain applied to the picture
 * @param {number} [o.alpha]     ink opacity
 * @param {string} [o.color]     override the CSS colour
 */
export function drawWaveform(canvas, o = {}) {
  const p = normalize(o.peaks, o.duration);
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || canvas.width / dpr || 100;
  const h = canvas.clientHeight || canvas.height / dpr || 24;
  const W = Math.max(1, Math.round(w * dpr));
  const H = Math.max(1, Math.round(h * dpr));
  if (canvas.width !== W) canvas.width = W;
  if (canvas.height !== H) canvas.height = H;
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  if (!p || !p.n) return;
  const from = o.from ?? 0;
  const to = o.to ?? p.n / p.rate;
  const gain = o.gain ?? 1;
  ctx.fillStyle = o.color || getComputedStyle(canvas).color || '#888';
  ctx.globalAlpha = o.alpha ?? 0.5;
  const mid = h / 2;
  const span = (to - from) / w;
  for (let x = 0; x < w; x++) {
    const a = from + span * x;
    const k0 = Math.floor(a * p.rate);
    const k1 = Math.max(k0 + 1, Math.ceil((a + span) * p.rate));
    let mn = 0;
    let mx = 0;
    for (let k = Math.max(0, k0); k < k1 && k < p.n; k++) {
      const lo = p.min(k);
      const hi = p.max(k);
      if (lo < mn) mn = lo;
      if (hi > mx) mx = hi;
    }
    mn = Math.max(-1, mn * gain);
    mx = Math.min(1, mx * gain);
    ctx.fillRect(x, mid - mx * mid, 1, Math.max(1, (mx - mn) * mid));
  }
  ctx.globalAlpha = 1;
}

/** A canvas that redraws itself when resized or when the theme changes. */
export function waveform(o = {}) {
  const canvas = document.createElement('canvas');
  canvas.className = `ui-waveform${o.className ? ` ${o.className}` : ''}`;
  let opts = { ...o };
  let frame = 0;
  const paint = () => {
    frame = 0;
    if (canvas.isConnected) drawWaveform(canvas, opts);
  };
  const schedule = () => {
    if (!frame) frame = requestAnimationFrame(paint);
  };
  const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(schedule) : null;
  ro?.observe(canvas);
  window.addEventListener('theme:change', schedule);
  canvas.update = (next = {}) => {
    opts = { ...opts, ...next };
    schedule();
  };
  canvas.destroy = () => {
    ro?.disconnect();
    window.removeEventListener('theme:change', schedule);
    if (frame) cancelAnimationFrame(frame);
  };
  schedule();
  return canvas;
}
