/**
 * menuFactory.js — the shared HUD-menu lifecycle.
 *
 * Every top-bar popover builds its content once, appends the popup to <body>
 * (the HUD pill clips overflow), unhides it under its chip, and closes on an
 * outside pointerdown or Escape. The menu owns that plumbing; the module owns
 * the content and any live-update timers through the `onOpen`/`onClose` hooks.
 *
 * Opening and closing move in the user's motion language (surfaceEnter /
 * surfaceExit, web/ui/motionStyle.js): the popup unrolls, stretches or tilts
 * out of its chip and goes back the same way.
 */

import { placeMenu, surfaceEnter, surfaceExit } from '../ui/index.js';
import { notifyMenuChange } from './menuState.js';

export function createMenu({
  build,
  align = 'left',
  minHeight,
  restartAnimation = true,
  onOpen,
  onClose,
}) {
  let popup = null;
  let trigger = null;
  let open = false;
  /** Bumped on every show/close, so a late exit never hides a reopened menu. */
  let generation = 0;

  function ensurePopup() {
    if (popup) return;
    popup = build();
    document.body.appendChild(popup);
  }

  function show(chip) {
    trigger = chip || trigger;
    ensurePopup();
    open = true;
    generation += 1;
    popup.classList.remove('hidden');
    // The CSS entrance some menus carry would fight the motion language.
    if (restartAnimation) popup.style.animation = 'none';
    onOpen?.();
    document.addEventListener('pointerdown', onOutside, true);
    document.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', reposition);
    trigger?.setAttribute('aria-expanded', 'true');
    trigger?.classList.add('is-open');
    reposition();
    void surfaceEnter(popup, trigger);
    notifyMenuChange();
  }

  function close() {
    if (!open) return;
    open = false;
    const ticket = ++generation;
    void surfaceExit(popup, { done: () => { if (ticket === generation) popup?.classList.add('hidden'); } });
    document.removeEventListener('pointerdown', onOutside, true);
    document.removeEventListener('keydown', onKey, true);
    window.removeEventListener('resize', reposition);
    trigger?.setAttribute('aria-expanded', 'false');
    trigger?.classList.remove('is-open');
    onClose?.();
    notifyMenuChange();
  }

  function toggle(chip) {
    if (open) close();
    else show(chip);
  }

  function onOutside(event) {
    if (event.target.closest?.('.ui-modal')) return;
    if (popup?.contains(event.target)) return;
    if (trigger?.contains(event.target)) return;
    close();
  }

  function onKey(event) {
    if (event.key === 'Escape' && !document.querySelector('.ui-modal:not(.hidden)')) {
      close();
    }
  }

  function reposition() {
    if (!popup || !trigger) return;
    placeMenu(popup, trigger, { align, minHeight });
  }

  return { show, close, toggle, reposition, isOpen: () => open, getPopup: () => popup };
}
