/**
 * barShared.js — what the Touch Bar page and the desktop page agree on, and
 * the bar's arithmetic. No DOM here, so Node can test it
 * (`tests/touchbarSurface.test.mjs`).
 *
 * The protocol, over one BroadcastChannel:
 *
 *   bar → desktop   { t: 'hello' }                       ask for a snapshot
 *                   { t: 'tap', id }                     an item was tapped
 *                   { t: 'slide', id, value, final? }    a slider moved (0..1)
 *                   { t: 'orb', phase: 'down' | 'up' }   the assistant button, pressed / released
 *                   { t: 'space', dir | index | add }    workspaces
 *                   { t: 'fkey', n }                     F1..F12 (Fn layer)
 *                   { t: 'activity' }                    a finger touched the bar
 *   desktop → bar   { t: 'state', state }                the whole picture
 *                   { t: 'voice', state }                assistant state
 *                   { t: 'level', level }                voice loudness, 0..1
 *                   { t: 'sleep', on }                   the bar's light went out
 *                   { t: 'bye' }                         the desktop page is leaving
 */

export const CHANNEL = 'shiny.touchbar';

export const FN_KEYS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

export function clamp01(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return 0;
  return Math.max(0, Math.min(1, n));
}

/**
 * A slider's value after the finger moved `dx` px from where it landed. The
 * move is relative (the value does not jump to the finger), and `travel` px
 * cover the whole range.
 */
export function sliderValue(start, dx, travel) {
  if (!(travel > 0)) return clamp01(start);
  return clamp01(clamp01(start) + dx / travel);
}

/** How far the strip can scroll: nothing when its content fits. */
export function scrollLimit(contentWidth, viewWidth) {
  return Math.max(0, Math.round(contentWidth - viewWidth));
}

/**
 * One step of the fling after a swipe: `velocity` in px/ms, `dt` in ms.
 * Friction is exponential, so the result does not depend on the frame rate.
 */
export function flingStep(velocity, dt) {
  const step = Math.max(0, Math.min(64, dt));
  const next = velocity * Math.exp(-step / 325);
  return {
    velocity: next,
    distance: velocity * step,
    done: Math.abs(next) < 0.02,
  };
}

/** The bar's backlight for a given idle time: 2 on, 1 dim, 0 off. */
export function backlightFor(idleMs, { dimAfter = 60_000, offAfter = 240_000 } = {}) {
  if (idleMs >= offAfter) return 0;
  if (idleMs >= dimAfter) return 1;
  return 2;
}

/** The Touch Bar dock setting: the dock lives on the bar instead of the screen. */
export function normalizeTouchBarDock(value) {
  return value === '1' || value === true || value === 'on';
}

/**
 * Keep what a window may put on the bar to plain, bounded data: the entries
 * come from plugin code and travel to another page.
 */
export function normalizeContextItems(entries, limit = 12) {
  if (!Array.isArray(entries)) return [];
  const out = [];
  const seen = new Set();
  for (const entry of entries) {
    if (!entry || typeof entry !== 'object') continue;
    const label = typeof entry.label === 'string' ? entry.label.trim().slice(0, 40) : '';
    const id = String(entry.id ?? label).slice(0, 60);
    if (!id || !label || seen.has(id)) continue;
    if (typeof entry.onClick !== 'function') continue;
    seen.add(id);
    out.push({
      id,
      label,
      icon: typeof entry.icon === 'string' ? entry.icon : null,
      active: !!(entry.active || entry.checked),
      disabled: !!entry.disabled,
      showLabel: !!entry.showLabel,
      onClick: entry.onClick,
    });
    if (out.length >= limit) break;
  }
  return out;
}
