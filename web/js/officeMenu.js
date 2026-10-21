/**
 * officeMenu.js — shared furniture for the office suite's document menus.
 *
 * Calc, Impress and Word all append the same actions footer (new / import /
 * export / delete) to their document popover, and all four office windows
 * render the same short "updated" date. Keeping the structure here means a
 * footer change lands everywhere at once.
 */

import { menuItem, placePopover, setIcon } from '../ui/index.js';

/**
 * Show a document menu under its button and install the outside/Escape
 * listeners. The caller owns the menu state flag and its content.
 */
export function showDocumentMenu(popup, button, onOutside, onKey) {
  popup.classList.remove('hidden');
  placePopover(popup, button);
  popup.focusFirst();
  button.setAttribute('aria-expanded', 'true');
  document.addEventListener('pointerdown', onOutside, true);
  document.addEventListener('keydown', onKey, true);
}

/** Short month/day label for an ISO timestamp. */
export function formatMenuDate(iso) {
  if (!iso) return '';
  try {
    return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric' });
  } catch (_) {
    return '';
  }
}

/**
 * Append the actions footer to a document menu.
 *
 * `prefix` is the plugin's CSS prefix (`word-doc`, `calc-sheet`,
 * `impress-deck`); `items` carries `{ icon, label, danger?, onClick }`.
 */
export function appendMenuFooter(popup, { prefix, close, items, buttonType = false }) {
  const foot = document.createElement('div');
  foot.className = `${prefix}-menu-foot`;
  for (const { icon, label, danger, onClick } of items) {
    const item = menuItem({ className: `${prefix}-menu-item` });
    if (buttonType) item.type = 'button';
    if (danger) item.classList.add(`${prefix}-menu-item--danger`);
    item.setAttribute('role', 'menuitem');
    const ic = document.createElement('span');
    ic.className = `${prefix}-menu-foot-icon`;
    item.appendChild(ic);
    void setIcon(ic, icon, { size: 14 });
    const labelEl = document.createElement('span');
    labelEl.className = `${prefix}-menu-title`;
    labelEl.textContent = label;
    item.appendChild(labelEl);
    item.addEventListener('click', () => {
      close();
      onClick();
    });
    foot.appendChild(item);
  }
  popup.appendChild(foot);
  return foot;
}
