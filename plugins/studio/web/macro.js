/**
 * macro.js — where a macro knob puts one of its targets.
 *
 * `base + (knob - 0.5) * 2 * amount * (max - min)`, clamped to the target's
 * range: the engine's `macro_target` (src/engine.rs), which applies the same
 * at render. The knob moves the device's own controls live, so the window
 * keeps this copy, pinned to the engine by the cases in macro.test.mjs and
 * engine.rs's `macro_targets_follow_the_documented_formula`.
 */
export function macroTarget(base, knob, amount, min, max) {
  const k = Math.min(1, Math.max(0, knob));
  const v = base + (k - 0.5) * 2 * amount * (max - min);
  return Math.min(Math.max(min, max), Math.max(Math.min(min, max), v));
}
