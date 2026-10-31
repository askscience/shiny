/**
 * motionStyle.js — the user's motion language, shared by core and plugins.
 *
 * Settings → Appearance → "Motion" picks one of three languages; the choice is
 * published as `<html data-motion="…">` (preferences.js) and every surface
 * reads it at the moment it moves:
 *
 *   kinetic  (default) masks and curtains with a cinematic ease: windows rise
 *            behind a curtain, menus unroll, rows are wiped in, numbers roll
 *   fluid    real springs with a little bounce: windows grow out of their
 *            tray icon and fall back into it, menus stretch out of their chip
 *   depth    the z-axis: windows arrive out of depth while the desk steps
 *            back, menus tilt up into place, things settle out of a blur
 *
 * Each helper animates *towards* the element's own styles and returns a
 * promise; entrances leave nothing behind, so CSS stays the resting truth.
 * Exits hold their last frame until the caller has hidden or removed the
 * element (`done`), then let go. Reduced motion skips every one of them.
 *
 * Springs are damped springs sampled into CSS `linear()` easings, published
 * as `--spring-<name>` / `--spring-<name>-d` so CSS transitions can use the
 * same curves (motion.css).
 */
import { reducedMotion } from './motion.js';

export const MOTION_STYLES = ['kinetic', 'fluid', 'depth'];
export const DEFAULT_MOTION_STYLE = 'kinetic';

/** The active motion language. */
export function motionStyle() {
  const value = typeof document !== 'undefined' ? document.documentElement?.dataset?.motion : '';
  return MOTION_STYLES.includes(value) ? value : DEFAULT_MOTION_STYLE;
}

/* ── Springs ────────────────────────────────────────────────── */

/** A damped spring sampled into a CSS `linear()` easing and its settle time. */
export function spring({ stiffness, damping, mass = 1 }) {
  const w0 = Math.sqrt(stiffness / mass);
  const zeta = damping / (2 * Math.sqrt(stiffness * mass));
  const x = (t) => {
    if (zeta < 1) {
      const wd = w0 * Math.sqrt(1 - zeta * zeta);
      return 1 - Math.exp(-zeta * w0 * t) * (Math.cos(wd * t) + ((zeta * w0) / wd) * Math.sin(wd * t));
    }
    return 1 - Math.exp(-w0 * t) * (1 + w0 * t);
  };
  // Settled once it stays within 0.1% of the target for 50 ms.
  let end = 0;
  let still = 0;
  while (end < 3) {
    end += 1 / 240;
    if (Math.abs(1 - x(end)) < 0.001) {
      still += 1 / 240;
      if (still > 0.05) break;
    } else still = 0;
  }
  const n = 40;
  const points = Array.from({ length: n + 1 }, (_, i) => (i === n ? 1 : +x((end * i) / n).toFixed(4)));
  return { easing: `linear(${points.join(', ')})`, duration: Math.round(end * 1000) };
}

export const SPRINGS = {
  bouncy: spring({ stiffness: 260, damping: 23 }),
  snappy: spring({ stiffness: 480, damping: 44 }),
  soft: spring({ stiffness: 130, damping: 23 }),
};

export const EASE = {
  out: 'cubic-bezier(0.2, 0.8, 0.2, 1)',
  in: 'cubic-bezier(0.4, 0, 1, 1)',
  expo: 'cubic-bezier(0.16, 1, 0.3, 1)',
  cine: 'cubic-bezier(0.76, 0, 0.24, 1)',
};

/** Publish the springs as CSS custom properties on :root (once, at boot). */
export function installMotionTokens(root = document.documentElement) {
  for (const [name, sp] of Object.entries(SPRINGS)) {
    root.style.setProperty(`--spring-${name}`, sp.easing);
    root.style.setProperty(`--spring-${name}-d`, `${sp.duration}ms`);
  }
}

/* ── Plumbing ───────────────────────────────────────────────── */

/** One owner per element: a new motion on a node replaces the last one. */
const owned = new WeakMap();

function stop(node) {
  const list = owned.get(node);
  if (!list) return;
  owned.delete(node);
  for (const a of list) a.cancel();
}

/** Start keyframe animations on nodes; resolves when they all settle. */
function run(steps, { hold = false } = {}) {
  const live = steps.filter(([node]) => node?.animate && node.isConnected);
  if (reducedMotion() || !live.length) return Promise.resolve(() => {});
  const byNode = new Map();
  for (const [node, frames, options] of live) {
    if (!byNode.has(node)) { stop(node); byNode.set(node, []); }
    let animation;
    try {
      animation = node.animate(frames, { fill: hold ? 'forwards' : 'backwards', ...options });
    } catch { continue; }
    byNode.get(node).push(animation);
  }
  for (const [node, list] of byNode) owned.set(node, list);
  const all = [...byNode.values()].flat();
  const longest = Math.max(0, ...all.map((a) => {
    const timing = a.effect?.getComputedTiming?.();
    return Number(timing?.endTime) || 0;
  }));
  const release = () => { for (const [node, list] of byNode) if (owned.get(node) === list) stop(node); };
  return new Promise((resolve) => {
    let settled = false;
    const settle = () => {
      if (settled) return;
      settled = true;
      clearTimeout(watchdog);
      if (!hold) release();
      resolve(release);
    };
    // An occluded renderer may never paint the last frame; never wait on it.
    const watchdog = setTimeout(settle, longest + 150);
    Promise.all(all.map((a) => a.finished)).then(settle, settle);
  });
}

/** Run an exit: hold the last frame, let `done` hide the node, then let go. */
async function leave(steps, done) {
  const release = await run(steps, { hold: true });
  done?.();
  release();
}

/** Whether a node can play a motion at all right now. */
const canAnimate = (el) => !!el?.animate && el.isConnected !== false && !reducedMotion();

const radiusOf = (el) => (typeof getComputedStyle === 'function' ? getComputedStyle(el).borderTopLeftRadius : '0px') || '0px';
const rectOf = (el) => el?.getBoundingClientRect?.();

/** Shift that puts box r's centre on box from's centre. */
function toCentre(r, from) {
  return [from.left + from.width / 2 - (r.left + r.width / 2), from.top + from.height / 2 - (r.top + r.height / 2)];
}

/* ── Text: decoding labels and rolling numbers ──────────────── */

const GLYPHS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
const decoding = new WeakMap();

/**
 * Set a label's text by decoding it in place: unresolved letters cycle through
 * random glyphs and settle left to right. Plain textContent with reduced motion.
 */
export function decodeText(el, text, { duration = 520 } = {}) {
  if (!el) return;
  cancelAnimationFrame(decoding.get(el) || 0);
  if (reducedMotion() || typeof requestAnimationFrame !== 'function') { el.textContent = text; return; }
  const t0 = performance.now();
  const tick = (now) => {
    const k = Math.min(1, (now - t0) / duration);
    const fixed = Math.floor(k * text.length);
    el.textContent = [...text].map((ch, i) => (i < fixed || ch === ' '
      ? ch
      : GLYPHS[Math.floor(Math.random() * GLYPHS.length)])).join('');
    if (k < 1) decoding.set(el, requestAnimationFrame(tick));
    else { el.textContent = text; decoding.delete(el); }
  };
  decoding.set(el, requestAnimationFrame(tick));
}

/** Roll a number up to its value (the element's text), keeping its decimals. */
export function rollNumber(el, { duration = 1000 } = {}) {
  if (!el || reducedMotion() || typeof requestAnimationFrame !== 'function') return;
  const raw = el.textContent.trim();
  const match = raw.match(/^(-?)([\d.,]+)(.*)$/);
  if (!match) return;
  const sep = match[2].includes(',') && !match[2].includes('.') ? ',' : '.';
  const target = Number(match[2].replace(sep === ',' ? /\./g : /,/g, '').replace(',', '.'));
  if (!Number.isFinite(target)) return;
  const decimals = (match[2].split(sep)[1] || '').length;
  const t0 = performance.now();
  const tick = (now) => {
    const k = Math.min(1, (now - t0) / duration);
    const v = target * (1 - (1 - k) ** 4);
    el.textContent = k < 1 ? `${match[1]}${v.toFixed(decimals).replace('.', sep)}${match[3]}` : raw;
    if (k < 1) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

/* ── Windows ────────────────────────────────────────────────── */

/**
 * A window arriving. `from` is the box it comes out of (its tray icon), used
 * by fluid; `others` are the windows already on the desk, which depth sends
 * one step back while it lands.
 */
export function windowEnter(el, { from = null, others = [] } = {}) {
  const style = motionStyle();
  const r = rectOf(el);
  if (!r || !r.width) return Promise.resolve();
  const radius = radiusOf(el);
  // The window's content: not its chrome, glow, rim or resize grip.
  const body = [...el.children].find((c) => !c.matches('[data-window-chrome], .tile-glow, .tile-resize, [class^="tile-rim"]')) || null;
  if (style === 'fluid') {
    const [dx, dy] = from ? toCentre(r, from) : [0, 40];
    const s = from ? Math.max(from.width / r.width, 0.05) : 0.9;
    return run([
      [el, [
        { transform: `translate(${dx}px, ${dy}px) scale(${s})`, opacity: 0 },
        { opacity: 1, offset: 0.16 },
        { transform: 'none', opacity: 1 },
      ], SPRINGS.bouncy],
      [body, [{ opacity: 0 }, { opacity: 1 }], { duration: 260, delay: 150, easing: EASE.out }],
    ]);
  }
  if (style === 'depth') {
    return run([
      [el, [
        { opacity: 0, transform: 'scale(0.86)', filter: 'blur(14px)' },
        { opacity: 1, offset: 0.35 },
        { opacity: 1, transform: 'none', filter: 'blur(0px)' },
      ], SPRINGS.soft],
      ...others.map((o) => [o, [{ transform: 'none' }, { transform: 'scale(0.975)', offset: 0.35 }, { transform: 'none' }], { duration: 900, easing: 'ease-in-out' }]),
    ]);
  }
  const title = el.querySelector(':scope > [data-window-chrome] .tile-header-title');
  const letters = title ? splitLetters(title) : [];
  const done = run([
    [el, [
      { clipPath: `inset(100% 0% 0% 0% round ${radius})`, transform: 'translateY(40px)' },
      { clipPath: `inset(0% 0% 0% 0% round ${radius})`, transform: 'none' },
    ], { duration: 820, easing: EASE.cine }],
    [body, [{ transform: 'translateY(36px)' }, { transform: 'none' }], { duration: 980, easing: EASE.cine }],
    ...letters.map((ch, i) => [ch, [{ transform: 'translateY(110%)' }, { transform: 'none' }], { duration: 620, delay: 320 + i * 28, easing: EASE.expo }]),
  ]);
  return done.then(() => { if (title && letters.length) title.textContent = title.dataset.text ?? title.textContent; });
}

/** Wrap a title's letters for the kinetic rise; the caller restores the text. */
function splitLetters(title) {
  const text = title.textContent;
  title.dataset.text = text;
  title.textContent = '';
  return [...text].map((ch) => {
    const span = document.createElement('span');
    span.className = 'motion-letter';
    span.textContent = ch;
    title.appendChild(span);
    return span;
  });
}

/**
 * A window leaving. Resolves after `done` (which removes or hides it) ran;
 * `to` is the box it falls back into (its tray icon, fluid).
 */
export function windowExit(el, { to = null, done } = {}) {
  const style = motionStyle();
  const r = rectOf(el);
  if (!r || !r.width || !canAnimate(el)) { done?.(); return Promise.resolve(); }
  el.style.pointerEvents = 'none';
  const finish = () => { el.style.pointerEvents = ''; done?.(); };
  if (style === 'fluid') {
    const [dx, dy] = to ? toCentre(r, to) : [0, 40];
    const s = to ? Math.max(to.width / r.width, 0.05) : 0.9;
    return leave([[el, [
      { transform: 'none', opacity: 1 },
      { opacity: 1, offset: 0.6 },
      { transform: `translate(${dx}px, ${dy}px) scale(${s})`, opacity: 0 },
    ], { ...SPRINGS.snappy, duration: Math.round(SPRINGS.snappy.duration * 1.3) }]], finish);
  }
  if (style === 'depth') {
    return leave([[el, [
      { opacity: 1, transform: 'none', filter: 'blur(0px)' },
      { opacity: 0, transform: 'scale(0.9)', filter: 'blur(10px)' },
    ], { duration: 380, easing: 'cubic-bezier(0.4, 0, 0.2, 1)' }]], finish);
  }
  const radius = radiusOf(el);
  return leave([[el, [
    { clipPath: `inset(0% 0% 0% 0% round ${radius})`, transform: 'none' },
    { clipPath: `inset(0% 0% 100% 0% round ${radius})`, transform: 'translateY(-24px)' },
  ], { duration: 560, easing: EASE.cine }]], finish);
}

/* ── Menus and popovers ─────────────────────────────────────── */

/** Point a surface's transform origin at its anchor, and say which way it opened. */
function aimAt(el, anchor) {
  const r = rectOf(el);
  const a = anchor && (anchor.getBoundingClientRect ? anchor.getBoundingClientRect() : anchor);
  if (!r || !a) return { downwards: true };
  const downwards = r.top >= a.top + a.height / 2;
  const x = Math.max(0, Math.min(r.width, a.left + a.width / 2 - r.left));
  el.style.transformOrigin = `${x}px ${downwards ? '0%' : '100%'}`;
  return { downwards };
}

/** A menu or popover opening from `anchor` (an element or a DOMRect). */
export function surfaceEnter(el, anchor = null) {
  if (!canAnimate(el)) return Promise.resolve();
  const style = motionStyle();
  const { downwards } = aimAt(el, anchor);
  const kids = [...el.children].slice(0, 14);
  if (style === 'fluid') {
    return run([
      [el, [
        { transform: 'scale(0.32, 0.12)', opacity: 0 },
        { opacity: 1, offset: 0.3 },
        { transform: 'none', opacity: 1 },
      ], SPRINGS.bouncy],
      ...kids.map((k) => [k, [{ opacity: 0 }, { opacity: 1 }], { duration: 240, delay: 110, easing: EASE.out }]),
    ]);
  }
  if (style === 'depth') {
    const tilt = downwards ? -14 : 14;
    return run([[el, [
      { opacity: 0, transform: `perspective(800px) rotateX(${tilt}deg) translateY(${downwards ? -12 : 12}px) scale(0.94)`, filter: 'blur(8px)' },
      { opacity: 1, offset: 0.45 },
      { opacity: 1, transform: 'perspective(800px)', filter: 'blur(0px)' },
    ], SPRINGS.soft]]);
  }
  const radius = radiusOf(el);
  const from = downwards ? `inset(0% 0% 100% 0% round ${radius})` : `inset(100% 0% 0% 0% round ${radius})`;
  return run([
    [el, [{ clipPath: from }, { clipPath: `inset(0% 0% 0% 0% round ${radius})` }], { duration: 540, easing: EASE.cine }],
    ...kids.map((k, i) => [k, [{ opacity: 0, transform: `translateY(${downwards ? -12 : 12}px)` }, { opacity: 1, transform: 'none' }], { duration: 620, delay: 120 + i * 40, easing: EASE.expo }]),
  ]);
}

/** A menu or popover closing back towards where it came from. */
export function surfaceExit(el, { done } = {}) {
  if (!canAnimate(el) || el.hidden || el.classList?.contains('hidden')) { done?.(); return Promise.resolve(); }
  const style = motionStyle();
  const downwards = !String(el.style?.transformOrigin).endsWith('100%');
  if (style === 'fluid') {
    return leave([[el, [{ transform: 'none', opacity: 1 }, { transform: 'scale(0.5, 0.25)', opacity: 0 }], { duration: 220, easing: EASE.in }]], done);
  }
  if (style === 'depth') {
    return leave([[el, [
      { opacity: 1, transform: 'perspective(800px)', filter: 'blur(0px)' },
      { opacity: 0, transform: `perspective(800px) rotateX(${downwards ? -10 : 10}deg) scale(0.96)`, filter: 'blur(6px)' },
    ], { duration: 260, easing: EASE.in }]], done);
  }
  const radius = radiusOf(el);
  const to = downwards ? `inset(0% 0% 100% 0% round ${radius})` : `inset(100% 0% 0% 0% round ${radius})`;
  return leave([[el, [{ clipPath: `inset(0% 0% 0% 0% round ${radius})` }, { clipPath: to }], { duration: 380, easing: EASE.cine }]], done);
}

/* ── Content ────────────────────────────────────────────────── */

/** Rows arriving in a cascade (a list loaded, a folder opened). */
export function listReveal(nodes) {
  const list = [...nodes].filter(Boolean).slice(0, 24);
  if (!list.length) return Promise.resolve();
  const style = motionStyle();
  if (style === 'fluid') {
    return run(list.map((n, i) => [n, [{ transform: 'translateY(18px)', opacity: 0 }, { transform: 'none', opacity: 1 }], { ...SPRINGS.bouncy, delay: i * 35 }]));
  }
  if (style === 'depth') {
    return run(list.map((n, i) => [n, [
      { opacity: 0, transform: 'scale(0.95)', filter: 'blur(6px)' },
      { opacity: 1, transform: 'none', filter: 'blur(0px)' },
    ], { ...SPRINGS.soft, delay: i * 40 }]));
  }
  return run(list.map((n, i) => [n, [{ clipPath: 'inset(0% 100% 0% 0%)' }, { clipPath: 'inset(0% 0% 0% 0%)' }], { duration: 640, delay: i * 45, easing: EASE.cine }]));
}

/**
 * A card arriving (a tool card in the chat, a notification). Kinetic also
 * rolls the card's big numbers (`[data-roll]`) up to their value.
 */
export function cardEnter(el) {
  if (!canAnimate(el)) return Promise.resolve();
  const style = motionStyle();
  if (style === 'fluid') {
    return run([[el, [{ transform: 'translateY(18px) scale(0.96)', opacity: 0 }, { transform: 'none', opacity: 1 }], SPRINGS.bouncy]]);
  }
  if (style === 'depth') {
    return run([[el, [
      { opacity: 0, transform: 'translateY(24px) scale(0.92)', filter: 'blur(12px)' },
      { opacity: 1, offset: 0.4 },
      { opacity: 1, transform: 'none', filter: 'blur(0px)' },
    ], SPRINGS.soft]]);
  }
  el.querySelectorAll('[data-roll]').forEach((n) => rollNumber(n));
  const radius = radiusOf(el);
  return run([[el, [
    { clipPath: `inset(100% 0% 0% 0% round ${radius})` },
    { clipPath: `inset(0% 0% 0% 0% round ${radius})` },
  ], { duration: 720, easing: EASE.cine }]]);
}

/** A card leaving (a notification dismissed). */
export function cardExit(el, { done } = {}) {
  if (!canAnimate(el)) { done?.(); return Promise.resolve(); }
  const style = motionStyle();
  if (style === 'fluid') {
    return leave([[el, [{ transform: 'none', opacity: 1 }, { transform: 'scale(0.9)', opacity: 0 }], { duration: 220, easing: EASE.in }]], done);
  }
  if (style === 'depth') {
    return leave([[el, [
      { opacity: 1, transform: 'none', filter: 'blur(0px)' },
      { opacity: 0, transform: 'scale(0.94)', filter: 'blur(8px)' },
    ], { duration: 320, easing: EASE.in }]], done);
  }
  const radius = radiusOf(el);
  return leave([[el, [
    { clipPath: `inset(0% 0% 0% 0% round ${radius})` },
    { clipPath: `inset(0% 100% 0% 0% round ${radius})` },
  ], { duration: 440, easing: EASE.cine }]], done);
}

/* ── Layout ─────────────────────────────────────────────────── */

/**
 * A window whose box changed because the layout re-tiled (another window
 * closed or opened, fullscreen toggled). `from` is its box before the change;
 * the element already sits in its new box. A window that grew is revealed
 * from its old box outwards (clip-path, so its content never stretches); one
 * that shrank or moved slides and scales from where it was.
 */
export function layoutShift(el, from) {
  if (!canAnimate(el) || !from) return Promise.resolve();
  const to = rectOf(el);
  if (!to || !to.width || !to.height) return Promise.resolve();
  const moved = Math.abs(from.left - to.left) + Math.abs(from.top - to.top)
    + Math.abs(from.width - to.width) + Math.abs(from.height - to.height);
  if (moved < 3) return Promise.resolve();
  const style = motionStyle();
  const timing = style === 'fluid' ? SPRINGS.bouncy
    : style === 'depth' ? SPRINGS.soft
      : { duration: 640, easing: EASE.cine };
  const radius = radiusOf(el);
  const grew = from.left >= to.left - 1 && from.top >= to.top - 1
    && from.left + from.width <= to.left + to.width + 1 && from.top + from.height <= to.top + to.height + 1;
  if (grew) {
    const inset = [
      from.top - to.top,
      to.left + to.width - (from.left + from.width),
      to.top + to.height - (from.top + from.height),
      from.left - to.left,
    ].map((v) => `${Math.max(0, v).toFixed(1)}px`).join(' ');
    return run([[el, [
      { clipPath: `inset(${inset} round ${radius})` },
      { clipPath: `inset(0px 0px 0px 0px round ${radius})` },
    ], timing]]);
  }
  const sx = from.width / to.width;
  const sy = from.height / to.height;
  const dx = from.left - to.left;
  const dy = from.top - to.top;
  return run([[el, [
    { transformOrigin: '0 0', transform: `translate(${dx}px, ${dy}px) scale(${sx}, ${sy})` },
    { transformOrigin: '0 0', transform: 'none' },
  ], timing]]);
}
