/**
 * pullRefresh — a phone's "pull down to refresh" for a scrolling list.
 *
 * On a touch screen, dragging a list down while it is at the top pulls a
 * small spinner out from under its top edge; letting go past the threshold
 * runs `onRefresh()` (a promise) and the spinner turns until it settles.
 * Mouse and keyboard users keep their refresh button; the helper never
 * takes the touch away from scrolling (passive listeners, no preventDefault).
 *
 *   const release = pullRefresh(listEl, () => reload());
 *   …
 *   release();   // when the list goes away
 */

import { spinner } from './components/feedback.js';

/** Pull (px) that triggers a refresh. */
const THRESHOLD = 64;
/** How far the indicator follows the finger at most. */
const MAX_PULL = 96;

export function pullRefresh(scroller, onRefresh, { threshold = THRESHOLD } = {}) {
  if (!scroller || typeof onRefresh !== 'function') return () => {};
  scroller.classList.add('ui-pull');

  const indicator = document.createElement('div');
  indicator.className = 'ui-pull-indicator';
  indicator.setAttribute('aria-hidden', 'true');
  indicator.appendChild(spinner());
  scroller.prepend(indicator);

  let start = null;
  let pull = 0;
  let busy = false;

  const paint = () => {
    indicator.style.setProperty('--pull', `${Math.min(MAX_PULL, pull)}px`);
    indicator.classList.toggle('is-ready', pull >= threshold);
  };

  const onStart = (e) => {
    if (busy || scroller.scrollTop > 0 || e.touches.length !== 1) { start = null; return; }
    start = { y: e.touches[0].clientY };
    pull = 0;
  };
  const onMove = (e) => {
    if (!start || busy) return;
    const dy = e.touches[0].clientY - start.y;
    if (dy <= 0 || scroller.scrollTop > 0) { pull = 0; paint(); indicator.classList.remove('is-pulling'); return; }
    pull = dy * 0.6;
    indicator.classList.add('is-pulling');
    paint();
  };
  const onEnd = async () => {
    if (!start) return;
    start = null;
    const go = pull >= threshold && !busy;
    indicator.classList.remove('is-pulling');
    if (!go) { pull = 0; paint(); return; }
    busy = true;
    pull = threshold;
    indicator.classList.add('is-busy');
    paint();
    try { await onRefresh(); } catch (_) { /* the owner reports its own errors */ }
    busy = false;
    pull = 0;
    indicator.classList.remove('is-busy', 'is-ready');
    paint();
  };

  scroller.addEventListener('touchstart', onStart, { passive: true });
  scroller.addEventListener('touchmove', onMove, { passive: true });
  scroller.addEventListener('touchend', onEnd, { passive: true });
  scroller.addEventListener('touchcancel', onEnd, { passive: true });

  return () => {
    scroller.removeEventListener('touchstart', onStart);
    scroller.removeEventListener('touchmove', onMove);
    scroller.removeEventListener('touchend', onEnd);
    scroller.removeEventListener('touchcancel', onEnd);
    indicator.remove();
    scroller.classList.remove('ui-pull');
  };
}
