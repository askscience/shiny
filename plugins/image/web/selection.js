/**
 * Selection masks.
 *
 * A selection is a canvas-sized, single-channel 8-bit coverage mask. The
 * marquee, lasso and magic-wand tools build one here, it is uploaded to the
 * server once per change, and every operation is confined to it there. The
 * overlay draws its outline as marching ants.
 */

export function newMask(w, h) {
  return new Uint8Array(w * h);
}

export function maskIsEmpty(mask) {
  for (let i = 0; i < mask.length; i += 1) if (mask[i] > 0) return false;
  return true;
}

export function maskBounds(mask, w, h) {
  let minX = w;
  let minY = h;
  let maxX = -1;
  let maxY = -1;
  for (let y = 0; y < h; y += 1) {
    const row = y * w;
    for (let x = 0; x < w; x += 1) {
      if (mask[row + x] > 0) {
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
    }
  }
  if (maxX < 0) return null;
  return { x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1 };
}

/** Draw a shape into a fresh mask with 3× supersampled edges. */
export function maskShape(w, h, { shape = 'rect', x = 0, y = 0, width = 0, height = 0 }) {
  const mask = newMask(w, h);
  const x0 = Math.max(0, Math.floor(x));
  const y0 = Math.max(0, Math.floor(y));
  const x1 = Math.min(w, Math.ceil(x + width));
  const y1 = Math.min(h, Math.ceil(y + height));
  const ss = 3;
  for (let py = y0; py < y1; py += 1) {
    for (let px = x0; px < x1; px += 1) {
      let inside = 0;
      for (let sy = 0; sy < ss; sy += 1) {
        for (let sx = 0; sx < ss; sx += 1) {
          const fx = px + (sx + 0.5) / ss;
          const fy = py + (sy + 0.5) / ss;
          if (shape === 'ellipse') {
            const rx = width / 2;
            const ry = height / 2;
            const cx = x + rx;
            const cy = y + ry;
            if (rx > 0 && ry > 0) {
              const v = ((fx - cx) / rx) ** 2 + ((fy - cy) / ry) ** 2;
              if (v <= 1) inside += 1;
            }
          } else if (fx >= x && fx <= x + width && fy >= y && fy <= y + height) {
            inside += 1;
          }
        }
      }
      if (inside > 0) mask[py * w + px] = Math.round((inside / (ss * ss)) * 255);
    }
  }
  return mask;
}

/** Rasterise a closed polygon (lasso) with supersampled edges. */
export function maskPolygon(w, h, points) {
  const mask = newMask(w, h);
  const pts = points.filter((p) => Array.isArray(p) && p.length >= 2);
  if (pts.length < 3) return mask;
  let minX = w;
  let minY = h;
  let maxX = 0;
  let maxY = 0;
  for (const [px, py] of pts) {
    minX = Math.min(minX, px);
    minY = Math.min(minY, py);
    maxX = Math.max(maxX, px);
    maxY = Math.max(maxY, py);
  }
  const x0 = Math.max(0, Math.floor(minX));
  const y0 = Math.max(0, Math.floor(minY));
  const x1 = Math.min(w, Math.ceil(maxX) + 1);
  const y1 = Math.min(h, Math.ceil(maxY) + 1);
  const ss = 2;
  for (let py = y0; py < y1; py += 1) {
    for (let px = x0; px < x1; px += 1) {
      let inside = 0;
      for (let sy = 0; sy < ss; sy += 1) {
        for (let sx = 0; sx < ss; sx += 1) {
          if (pointInPoly(pts, px + (sx + 0.5) / ss, py + (sy + 0.5) / ss)) inside += 1;
        }
      }
      if (inside > 0) mask[py * w + px] = Math.round((inside / (ss * ss)) * 255);
    }
  }
  return mask;
}

function pointInPoly(pts, x, y) {
  let inside = false;
  for (let i = 0, j = pts.length - 1; i < pts.length; j = i, i += 1) {
    const [xi, yi] = pts[i];
    const [xj, yj] = pts[j];
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

/**
 * Magic wand over raw RGBA pixels. `sampleAll` compares against the composite
 * (the caller passes composite pixels); otherwise the caller passes the layer's
 * own rendered pixels.
 */
export function maskWand(rgba, w, h, { x, y, tolerance = 32, contiguous = true }) {
  const mask = newMask(w, h);
  const sx = Math.max(0, Math.min(w - 1, Math.round(x)));
  const sy = Math.max(0, Math.min(h - 1, Math.round(y)));
  const si = (sy * w + sx) * 4;
  const seed = [rgba[si], rgba[si + 1], rgba[si + 2], rgba[si + 3]];
  const tol = Math.max(0, tolerance);
  const close = (i) => {
    let d = 0;
    for (let c = 0; c < 4; c += 1) d = Math.max(d, Math.abs(rgba[i + c] - seed[c]));
    return d <= tol;
  };
  if (contiguous) {
    const stack = [[sx, sy]];
    const seen = new Uint8Array(w * h);
    while (stack.length) {
      const [cx, cy] = stack.pop();
      const idx = cy * w + cx;
      if (seen[idx]) continue;
      seen[idx] = 1;
      if (!close(idx * 4)) continue;
      mask[idx] = 255;
      if (cx > 0) stack.push([cx - 1, cy]);
      if (cx + 1 < w) stack.push([cx + 1, cy]);
      if (cy > 0) stack.push([cx, cy - 1]);
      if (cy + 1 < h) stack.push([cx, cy + 1]);
    }
  } else {
    for (let i = 0; i < w * h; i += 1) if (close(i * 4)) mask[i] = 255;
  }
  return mask;
}

/** Combine a new mask into the current one. */
export function combineMask(base, next, w, h, mode = 'new') {
  if (mode === 'new' || !base) return next;
  const out = new Uint8Array(base.length);
  for (let i = 0; i < base.length; i += 1) {
    const a = base[i];
    const b = next[i];
    if (mode === 'add') out[i] = Math.max(a, b);
    else if (mode === 'subtract') out[i] = Math.max(0, a - b);
    else if (mode === 'intersect') out[i] = Math.min(a, b);
    else out[i] = b;
  }
  return out;
}

export function invertMask(mask) {
  const out = new Uint8Array(mask.length);
  for (let i = 0; i < mask.length; i += 1) out[i] = 255 - mask[i];
  return out;
}

export function maskAll(w, h) {
  return new Uint8Array(w * h).fill(255);
}

/** Box-blur the mask to feather the selection edge. */
export function featherMask(mask, w, h, radius) {
  const r = Math.max(0, Math.round(radius));
  if (r === 0) return mask;
  const tmp = new Uint8Array(mask.length);
  for (let y = 0; y < h; y += 1) {
    for (let x = 0; x < w; x += 1) {
      const lo = Math.max(0, x - r);
      const hi = Math.min(w - 1, x + r);
      let sum = 0;
      for (let xx = lo; xx <= hi; xx += 1) sum += mask[y * w + xx];
      tmp[y * w + x] = sum / (hi - lo + 1);
    }
  }
  const out = new Uint8Array(mask.length);
  for (let y = 0; y < h; y += 1) {
    const lo = Math.max(0, y - r);
    const hi = Math.min(h - 1, y + r);
    for (let x = 0; x < w; x += 1) {
      let sum = 0;
      for (let yy = lo; yy <= hi; yy += 1) sum += tmp[yy * w + x];
      out[y * w + x] = sum / (hi - lo + 1);
    }
  }
  return out;
}

/** Boundary edge segments (doc coordinates) for the marching-ants overlay. */
export function maskEdges(mask, w, h) {
  const edges = [];
  const has = (x, y) => x >= 0 && y >= 0 && x < w && y < h && mask[y * w + x] > 127;
  // Horizontal runs where the row transitions selected/unselected.
  for (let y = 0; y < h; y += 1) {
    let runStart = -1;
    for (let x = 0; x <= w; x += 1) {
      const boundary = x < w && has(x, y) !== has(x, y - 1);
      if (boundary && runStart < 0) runStart = x;
      if ((!boundary || x === w) && runStart >= 0) {
        edges.push([runStart, y, x, y]);
        runStart = -1;
      }
    }
  }
  // Vertical runs.
  for (let x = 0; x < w; x += 1) {
    let runStart = -1;
    for (let y = 0; y <= h; y += 1) {
      const boundary = y < h && has(x, y) !== has(x - 1, y);
      if (boundary && runStart < 0) runStart = y;
      if ((!boundary || y === h) && runStart >= 0) {
        edges.push([x, runStart, x, y]);
        runStart = -1;
      }
    }
  }
  return edges;
}
