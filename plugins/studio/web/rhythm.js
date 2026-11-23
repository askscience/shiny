/**
 * rhythm.js — a voice's rhythm as grid cells, the way the engine plays it.
 *
 * `e<hits>[,<rotation>]` is a Euclidean rhythm: onset i on step
 * floor(i * steps / hits), the first on the beat, then rotated right; any
 * other text is an explicit pattern where only `x` (hit) and `.` (rest)
 * count. This is the engine's `rhythm_hits` (src/engine.rs); the grid needs
 * the cells on every repaint, so it keeps this copy, pinned to the engine by
 * the same cases in rhythm.test.mjs and engine.rs's `rhythm_tests`.
 */

/** `hits` onsets spread over `steps`, the first on step 0. */
export function euclid(hits, steps) {
  const p = new Array(Math.max(0, steps)).fill(false);
  const h = Math.min(Math.max(0, hits), steps);
  for (let i = 0; i < h; i++) p[Math.floor((i * steps) / h)] = true;
  return p;
}

/** Cells rotated right by `offset` (cell i moves to i + offset). */
export function rotateRight(p, offset) {
  const n = p.length;
  if (!n) return [];
  const off = n - (offset % n);
  return [...p.slice(off), ...p.slice(0, off)];
}

/** A rhythm string as `steps` on/off cells. */
export function rhythmToCells(rhythm, steps) {
  const r = String(rhythm || '').trim();
  const out = new Array(Math.max(0, steps)).fill(false);
  if (r.startsWith('e')) {
    const m = r.match(/^e\s*(\d+)\s*(?:,\s*(\d+)\s*)?$/);
    if (!m) return out;
    return rotateRight(euclid(parseInt(m[1], 10) || 0, steps), parseInt(m[2] || '0', 10) || 0);
  }
  const marks = [...r].filter((c) => c === 'x' || c === '.');
  for (let i = 0; i < steps && i < marks.length; i++) out[i] = marks[i] === 'x';
  return out;
}

/** Cells back to an explicit rhythm string. */
export function cellsToRhythm(cells) {
  return cells.map((b) => (b ? 'x' : '.')).join('');
}
