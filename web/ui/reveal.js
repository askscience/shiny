/**
 * reveal — quiet entrance choreography.
 *
 * Any element with [data-reveal] fades/rises into view once, the first
 * time it enters the viewport. Optional stagger via data-reveal="<ms>"
 * or the CSS var --reveal-delay. Disabled entirely for reduced motion.
 * Subtle by contract: one soft rise, no looping, no bouncing.
 */

let observer = null;

function ensureObserver() {
  if (observer) return observer;
  observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const el = entry.target;
      const delay = el.dataset.reveal;
      if (delay && /^\d+$/.test(delay)) el.style.setProperty('--reveal-delay', `${delay}ms`);
      el.classList.add('is-revealed');
      observer.unobserve(el);
    }
  }, { threshold: 0.12, rootMargin: '0px 0px -4% 0px' });
  return observer;
}

/** Scan a subtree for [data-reveal] elements and observe them. */
export function reveal(root = document) {
  if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
    root.querySelectorAll?.('[data-reveal]').forEach((el) => el.classList.add('is-revealed'));
    return;
  }
  const io = ensureObserver();
  root.querySelectorAll?.('[data-reveal]:not(.is-revealed)').forEach((el) => io.observe(el));
}

/** Immediately reveal an element (for content already in view on load). */
export function revealNow(el) {
  el?.classList.add('is-revealed');
}
