/**
 * hudDivider.js — the top bar's separator, in one place.
 *
 * Every hairline in the HUD bar is one of these elements: the clock /
 * weather / tray group separators, the one before the host status chips, the
 * one before the action icons, and the group separators inside the plugin
 * tray. `initHudDividers` mounts the ones that belong to the static layout;
 * anything built at runtime uses the same factory.
 */

/** Build the bar's separator element. */
export function hudDivider() {
  const el = document.createElement('div');
  el.className = 'hud-divider';
  el.setAttribute('aria-hidden', 'true');
  return el;
}

/** Mount the separators that belong to the static top-bar layout. */
export function initHudDividers() {
  const clock = document.querySelector('.hud-clock');
  const meteo = document.getElementById('hud-meteo');
  const top = document.getElementById('hud-top');
  const firstAction = document.getElementById('hud-clipboard');

  clock?.after(hudDivider());
  meteo?.after(hudDivider());
  firstAction?.before(hudDivider());

  if (top) {
    // The separator before the host status chips also absorbs whatever free
    // space the plugin tray leaves, so the chips + actions stay pinned to the
    // end edge of the bar.
    const push = hudDivider();
    push.classList.add('hud-divider--push');
    top.before(push);
  }
}
