import { controlButton } from './button.js';

/** Body-level menu surface. Owners supply items, placement and dismissal. */
export function menuSurface({ className = '', onEscape } = {}) {
  const menu = document.createElement('div');
  menu.className = `ui-menu hidden ${className}`.trim();
  menu.setAttribute('role', 'menu');
  let previousFocus;
  const enabledItems = () => [...menu.querySelectorAll('[role="menuitem"]')]
    .filter(item => !item.disabled && !item.hidden && item.getClientRects().length);
  menu.focusFirst = () => {
    previousFocus = document.activeElement;
    enabledItems()[0]?.focus({ preventScroll: true });
  };
  menu.addEventListener('keydown', event => {
    if (event.key === 'Escape') {
      onEscape?.();
      if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
      return;
    }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    const items = enabledItems();
    if (!items.length) return;
    const current = items.indexOf(document.activeElement);
    const index = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
      : (current + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
    event.preventDefault();
    items[index].focus();
  });
  return menu;
}

/** Shared menu action; callers may append domain title/metadata nodes. */
export function menuItem({ className = '', label, onClick, disabled = false } = {}) {
  const item = controlButton();
  item.className = `ui-menu-item ${className}`.trim();
  item.setAttribute('role', 'menuitem');
  item.disabled = disabled;
  if (label) item.textContent = label;
  if (onClick) item.addEventListener('click', onClick);
  return item;
}
