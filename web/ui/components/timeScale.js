/**
 * timeScale — the mapping between time and pixels that every timeline
 * shares: seconds and timecodes (Capture's editor) or bars and beats
 * (Studio). A scale is a plain object; zooming makes a new one.
 *
 *   const s = secondsScale({ pps: 80 });   // 80 px per second
 *   s.toPx(2.5)        // 200
 *   s.fromPx(200)      // 2.5
 *   s.ticks(0, 30)     // [{ t, major, label }] spaced for legibility
 *   s.label(75.5)      // "1:15.5" (precision follows the zoom)
 *
 * Tick spacing picks the smallest "nice" step whose major ticks are at
 * least `minMajorPx` apart, so the ruler never crowds at any zoom.
 */

const SECOND_STEPS = [0.04, 0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

/** The smallest step in `steps` whose span is at least `minPx` pixels. */
export function niceStep(pxPerUnit, minPx, steps = SECOND_STEPS) {
  for (const s of steps) if (s * pxPerUnit >= minPx) return s;
  return steps[steps.length - 1];
}

/** `m:ss`, `h:mm:ss`, with `digits` decimals of a second. */
export function formatTimecode(t, digits = 0) {
  const neg = t < 0;
  let v = Math.abs(t);
  const scale = 10 ** digits;
  v = Math.round(v * scale) / scale;
  const h = Math.floor(v / 3600);
  const m = Math.floor((v % 3600) / 60);
  const s = v % 60;
  const ss = digits ? s.toFixed(digits).padStart(3 + digits, '0') : String(Math.floor(s)).padStart(2, '0');
  const body = h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
  return neg ? `-${body}` : body;
}

function minorCount(step) {
  if (step >= 60) return step % 300 === 0 ? 5 : 4;
  const lead = step / 10 ** Math.floor(Math.log10(step));
  return lead === 2 ? 4 : 5;
}

export function secondsScale({ pps = 60, minMajorPx = 84 } = {}) {
  const major = niceStep(pps, minMajorPx);
  const digits = major < 0.1 ? 2 : major < 1 ? 1 : 0;
  const minor = major / minorCount(major);
  return {
    kind: 'seconds',
    pps,
    unit: 1,
    toPx: (t) => t * pps,
    fromPx: (x) => x / pps,
    label: (t) => formatTimecode(t, digits),
    major,
    minor,
    ticks(from, to) {
      const out = [];
      const start = Math.floor(Math.max(0, from) / minor) * minor;
      const n = Math.ceil((to - start) / minor) + 1;
      for (let i = 0; i < n && i < 5000; i++) {
        const t = start + i * minor;
        const k = Math.round(t / minor);
        const isMajor = Math.abs(t / major - Math.round(t / major)) < 1e-6;
        out.push({ t, major: isMajor, label: isMajor ? formatTimecode(t, digits) : '', k });
      }
      return out;
    },
    /** Snap `t` to the nearest minor tick when within `px` pixels. */
    snap(t, px = 6) {
      const q = Math.round(t / minor) * minor;
      return Math.abs(q - t) * pps <= px ? q : t;
    },
  };
}

/** Bars and beats: `ppb` pixels per beat, `beatsPerBar` beats a bar. */
export function beatsScale({ ppb = 24, beatsPerBar = 4, minMajorPx = 48 } = {}) {
  const bars = [1, 2, 4, 8, 16, 32, 64];
  const barPx = ppb * beatsPerBar;
  const everyBars = bars.find((b) => b * barPx >= minMajorPx) || 64;
  return {
    kind: 'beats',
    pps: ppb,
    unit: 1,
    toPx: (b) => b * ppb,
    fromPx: (x) => x / ppb,
    label: (b) => String(Math.floor(b / beatsPerBar) + 1),
    major: everyBars * beatsPerBar,
    minor: ppb >= 12 ? 1 : beatsPerBar,
    ticks(from, to) {
      const out = [];
      const step = ppb >= 12 ? 1 : beatsPerBar;
      const start = Math.floor(Math.max(0, from) / step) * step;
      for (let b = start; b <= to && out.length < 5000; b += step) {
        const isBar = b % beatsPerBar === 0;
        const isMajor = isBar && (b / beatsPerBar) % everyBars === 0;
        out.push({ t: b, major: isMajor, bar: isBar, label: isMajor ? String(b / beatsPerBar + 1) : '' });
      }
      return out;
    },
    snap(b, px = 6) {
      const q = Math.round(b);
      return Math.abs(q - b) * ppb <= px ? q : b;
    },
  };
}
