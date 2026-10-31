/**
 * gestureTransition.js — interactive transitions driven by a progress value.
 *
 * The trackpad reports a swipe as begin → update(progress) → end(progress,
 * velocity) (crates/peakd/src/gestures.rs). A surface that follows the
 * fingers (the overview, the launcher, the workspace slide) renders one
 * progress value, 0 = where it started and 1 = the committed state; on
 * release this module decides whether it completes or springs back and
 * animates the rest of the way. Discrete callers (a key, the Touch Bar) run
 * the very same transition from 0 to 1, so both paths look alike.
 *
 * The decision and timing helpers are pure, so they are unit-tested in Node
 * (web/js/tests/gestureTransition.test.mjs); the animator takes its clock and
 * frame source as optional arguments for the same reason.
 */

/** Past this much progress a release completes the transition. */
export const COMMIT_PROGRESS = 0.5;
/** A flick (progress per second) counts as intent from this speed on... */
export const FLICK_VELOCITY = 1.2;
/** ...once at least this much of the way has been travelled. */
export const FLICK_MIN_PROGRESS = 0.12;
/** The remaining distance is animated in this window, scaled by distance. */
export const SETTLE_MIN_MS = 250;
export const SETTLE_MAX_MS = 450;

export const clamp01 = (v) => Math.min(1, Math.max(0, Number(v) || 0));

/**
 * Whether a released gesture commits. `progress` is how far the transition
 * got (0..1, may lie outside while rubber-banding); `velocity` is progress per
 * second at release, positive towards committing.
 *
 * - A flick backwards always rolls back, however far the fingers got.
 * - A flick forwards commits once it travelled a little.
 * - Otherwise the half-way mark decides.
 */
export function shouldCommit(progress, velocity = 0) {
  const p = Number(progress) || 0;
  const v = Number(velocity) || 0;
  if (v <= -FLICK_VELOCITY) return false;
  if (v >= FLICK_VELOCITY && p >= FLICK_MIN_PROGRESS) return true;
  return p >= COMMIT_PROGRESS;
}

/**
 * How long the rest of the transition takes: proportional to the distance
 * still to go, between SETTLE_MIN_MS and SETTLE_MAX_MS. A fast release in the
 * right direction shortens it so the motion does not visibly brake.
 */
export function settleDuration(from, to, velocity = 0) {
  const distance = Math.min(1, Math.abs((Number(to) || 0) - (Number(from) || 0)));
  let ms = SETTLE_MIN_MS + (SETTLE_MAX_MS - SETTLE_MIN_MS) * distance;
  const towards = Math.sign((Number(to) || 0) - (Number(from) || 0));
  const speed = (Number(velocity) || 0) * towards;
  if (speed > FLICK_VELOCITY) ms *= Math.max(0.6, FLICK_VELOCITY / speed);
  return Math.round(Math.min(SETTLE_MAX_MS, Math.max(SETTLE_MIN_MS * 0.6, ms)));
}

/** A soft spring-like ease: fast out of the release, settling gently. */
export function easeSettle(t) {
  const k = clamp01(t);
  return 1 - (1 - k) ** 3;
}

/** Rubber band past the ends: travel beyond 0..1 is damped, never blocked. */
export function rubberBand(p, limit = 0.12) {
  if (p >= 0 && p <= 1) return p;
  const over = p < 0 ? -p : p - 1;
  const damped = limit * (1 - 1 / (over / limit + 1));
  return p < 0 ? -damped : 1 + damped;
}

export function prefersReducedMotion() {
  try {
    return !!globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  } catch (_) {
    return false;
  }
}

const defaultRaf = (fn) => (globalThis.requestAnimationFrame
  ? globalThis.requestAnimationFrame(fn)
  : setTimeout(() => fn(Date.now()), 16));
const defaultCancel = (id) => (globalThis.cancelAnimationFrame
  ? globalThis.cancelAnimationFrame(id)
  : clearTimeout(id));
const defaultNow = () => (globalThis.performance?.now ? globalThis.performance.now() : Date.now());

/**
 * Animate a value from `from` to `to`, calling `onFrame(value)` each frame and
 * `onDone()` once at the end. Under reduced motion it jumps straight there.
 * Returns a function that stops the animation without calling `onDone`.
 */
export function animateValue({
  from, to, duration, onFrame, onDone,
  raf = defaultRaf, cancel = defaultCancel, now = defaultNow, reduced = prefersReducedMotion(),
}) {
  if (reduced || !(duration > 0) || from === to) {
    onFrame(to);
    onDone?.();
    return () => {};
  }
  const t0 = now();
  let id = 0;
  let stopped = false;
  const tick = () => {
    if (stopped) return;
    const k = clamp01((now() - t0) / duration);
    onFrame(from + (to - from) * easeSettle(k));
    if (k < 1) id = raf(tick);
    else onDone?.();
  };
  id = raf(tick);
  return () => {
    stopped = true;
    cancel(id);
  };
}

/**
 * One interactive transition over a target `{ apply(p), finish(committed) }`:
 * `apply` renders a progress value (0..1, slightly beyond while
 * rubber-banding), `finish` settles the real state once the animation has
 * landed on 0 (`false`) or 1 (`true`).
 *
 * `update()` is coalesced to one `apply` per animation frame; `end()` decides
 * (or is told) the outcome and animates the remainder; `play()` runs the
 * whole transition for a discrete trigger.
 */
export function createTransition(target, { raf = defaultRaf, cancel = defaultCancel, now = defaultNow } = {}) {
  let current = 0;
  let pending = null;
  let frame = 0;
  let stop = null;
  let settled = false;

  const flush = () => {
    frame = 0;
    if (pending === null) return;
    current = pending;
    pending = null;
    target.apply(current);
  };

  const settle = (committed, velocity = 0) => {
    if (settled) return;
    if (frame) {
      cancel(frame);
      frame = 0;
    }
    if (pending !== null) {
      current = pending;
      pending = null;
    }
    stop?.();
    const to = committed ? 1 : 0;
    stop = animateValue({
      from: current,
      to,
      duration: settleDuration(current, to, committed ? velocity : -velocity),
      onFrame: (v) => {
        current = v;
        target.apply(v);
      },
      onDone: () => {
        settled = true;
        stop = null;
        target.finish(committed);
      },
      raf, cancel, now,
    });
  };

  return {
    get progress() { return pending ?? current; },
    get settled() { return settled; },
    update(p) {
      if (settled || stop) return;
      pending = rubberBand(Number(p) || 0);
      if (!frame) frame = raf(flush);
    },
    /** Release: commit per `shouldCommit` unless `committed` is given. */
    end(velocity = 0, committed = undefined) {
      const p = pending ?? current;
      settle(committed ?? shouldCommit(p, velocity), velocity);
    },
    cancel() { settle(false); },
    play() { settle(true); },
    /** Jump to the end state at once (a newer transition needs the stage). */
    finishNow(committed = true) {
      if (settled) return;
      stop?.();
      stop = null;
      if (frame) cancel(frame);
      frame = 0;
      pending = null;
      current = committed ? 1 : 0;
      target.apply(current);
      settled = true;
      target.finish(committed);
    },
  };
}
