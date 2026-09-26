/**
 * bootScreen — the startup splash.
 *
 * index.html paints `#boot-screen` (the skull mark with an accent glitch)
 * before any module runs. This module is the single place that takes it away,
 * so every path that commits to a screen — the app shell or the sign-in
 * overlay — leaves the splash behind. Idempotent, so callers never have to
 * track whether it already ran.
 */

let hidden = false;

export function hideBootScreen() {
  if (hidden) return;
  hidden = true;

  const el = document.getElementById('boot-screen');
  if (!el) return;

  el.classList.add('is-hidden');
  el.setAttribute('aria-busy', 'false');

  // Drop the node once it has faded — it stays fixed and full-viewport while
  // present, so leaving it in the DOM would keep a transparent overlay around.
  el.addEventListener('transitionend', () => el.remove(), { once: true });
  // Fallback for reduced motion (no transition → no `transitionend`) or a
  // backgrounded tab (transitions may not run).
  setTimeout(() => el.remove(), 1200);
}
