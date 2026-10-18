/** Flow: finite, interruptible motion shared by core and plugin surfaces. */
const running = new Map();
const visibility = new WeakMap();
let media;

export function reducedMotion() {
  if (!media && typeof window !== 'undefined' && window.matchMedia) {
    media = window.matchMedia('(prefers-reduced-motion: reduce)');
    media.addEventListener?.('change', () => {
      if (media.matches) for (const entry of [...running.values()]) entry.finish();
    });
  }
  return media?.matches === true;
}

/** Read theme tokens at the time of interaction, including after a theme swap. */
export function motionDuration(kind = 'reveal') {
  const defaults = { feedback: 120, reveal: 200, connect: 280 };
  const value = typeof getComputedStyle === 'function' && typeof document !== 'undefined'
    ? getComputedStyle(document.documentElement).getPropertyValue(`--motion-${kind}`).trim() : '';
  const number = parseFloat(value);
  return Number.isFinite(number) ? Math.max(0, Math.min(600, number * (value.endsWith('ms') ? 1 : 1000))) : (defaults[kind] ?? 200);
}

/** Final CSS/semantic state belongs to the caller, never to a filling animation. */
export function transition(node, frames, { kind = 'reveal', duration = motionDuration(kind), done = () => {} } = {}) {
  running.get(node)?.cancel();
  if (reducedMotion() || !node?.animate || !node.isConnected) { done(); return () => {}; }
  let animation;
  const easing = typeof getComputedStyle === 'function' && typeof document !== 'undefined'
    ? getComputedStyle(document.documentElement).getPropertyValue('--motion-ease').trim() : '';
  try { animation = node.animate(frames, { duration, easing: easing || 'cubic-bezier(.2,.8,.2,1)' }); }
  catch { done(); return () => {}; }
  let settled = false;
  let watchdog;
  const settle = complete => {
    if (settled) return;
    settled = true;
    clearTimeout(watchdog);
    if (running.get(node) === entry) running.delete(node);
    animation.cancel();
    if (complete) done();
  };
  const entry = { cancel: () => settle(false), finish: () => settle(true) };
  running.set(node, entry);
  animation.finished.then(entry.finish, () => settle(false));
  // An occluded embedded renderer must not leave a control waiting for a frame.
  watchdog = setTimeout(entry.finish, duration + 100);
  return entry.cancel;
}

export function disposeMotion(root) {
  for (const [node, entry] of [...running]) if (node === root || root?.contains?.(node)) entry.cancel();
}

export function revealRegion(node, visible, { animate = true } = {}) {
  const wasHidden = node.hidden || node.classList.contains('hidden');
  if (visibility.get(node) === visible && (running.has(node) || wasHidden !== visible)) return;
  visibility.set(node, visible);
  const previous = wasHidden ? 0 : node.getBoundingClientRect().height;
  running.get(node)?.cancel();
  node.classList.add('ui-expandable');
  node.classList.remove('hidden');
  const finish = () => { node.hidden = !visible; node.classList.toggle('hidden', !visible); };
  if (!animate || !node.isConnected || reducedMotion()) { finish(); return; }
  node.hidden = false;
  transition(node, [{ height: `${previous}px`, opacity: wasHidden ? 0 : 1 }, { height: `${visible ? node.scrollHeight : 0}px`, opacity: visible ? 1 : 0 }], { done: finish });
}

/** A small surface entrance. Call after placement so menus reveal from their anchor. */
export function enterSurface(node) {
  return transition(node, [{ opacity: 0, transform: 'translateY(-4px) scale(.985)' }, { opacity: 1, transform: 'none' }]);
}
