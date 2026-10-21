/**
 * menuFactory.js — the shared HUD-menu lifecycle.
 *
 * Every top-bar popover builds its content once, appends the popup to <body>
 * (the HUD pill clips overflow), unhides it under its chip, and closes on an
 * outside pointerdown or Escape. The menu owns that plumbing; the module owns
 * the content and any live-update timers through the `onOpen`/`onClose` hooks.
 */

import { placeMenu } from '../ui/index.js';
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

  function ensurePopup() {
    if (popup) return;
    popup = build();
    document.body.appendChild(popup);
  }

  function show(chip) {
    trigger = chip || trigger;
    ensurePopup();
    open = true;
    popup.classList.remove('hidden');
    if (restartAnimation) {
      popup.style.animation = 'none';
      void popup.offsetHeight;
      popup.style.animation = '';
    }
    onOpen?.();
    document.addEventListener('pointerdown', onOutside, true);
    document.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', reposition);
    trigger?.setAttribute('aria-expanded', 'true');
    trigger?.classList.add('is-open');
    reposition();
    notifyMenuChange();
  }

  function close() {
    if (!open) return;
    open = false;
    popup?.classList.add('hidden');
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
