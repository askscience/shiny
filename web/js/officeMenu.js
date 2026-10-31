/**
 * officeMenu.js — shared furniture for the office suite's document menus.
 *
 * Calc, Impress and Word all append the same actions footer (new / import /
 * save / export / delete) to their document popover — the window bars keep
 * only their working tools, so the footer is the one home of the file
 * actions — and all four office windows render the same short "updated"
 * date. Keeping the structure here means a footer change lands everywhere
 * at once.
 */

import { menuItem, placePopover, setIcon, listReveal } from '../ui/index.js';

/**
 * Show a document menu under its button and install the outside/Escape
 * listeners. The caller owns the menu state flag and its content. The
 * menu's rows arrive with the shared list reveal.
 */
export function showDocumentMenu(popup, button, onOutside, onKey) {
  popup.classList.remove('hidden');
  placePopover(popup, button);
  popup.focusFirst();
  void listReveal(popup.querySelectorAll('[role="menuitem"]'));
  button.setAttribute('aria-expanded', 'true');
  document.addEventListener('pointerdown', onOutside, true);
  document.addEventListener('keydown', onKey, true);
}

/**
 * Parse a server timestamp. SQLite's `datetime('now')` is UTC without a zone
 * ("2026-10-31 12:49:05"), which `Date` would read as local time.
 */
export function parseServerTime(iso) {
  if (!iso) return null;
  const text = String(iso);
  const d = new Date(/^\d{4}-\d\d-\d\d[ T]\d\d:\d\d(:\d\d(\.\d+)?)?$/.test(text) ? `${text.replace(' ', 'T')}Z` : text);
  return Number.isNaN(d.getTime()) ? null : d;
}

/** Short month/day label for an ISO timestamp. */
export function formatMenuDate(iso) {
  const d = parseServerTime(iso);
  return d ? d.toLocaleDateString([], { month: 'short', day: 'numeric' }) : '';
}

/**
 * The time of a save for a window's mono meta: "14:02" today, else the short
 * date. An unparseable stamp (a just-created record says "now") reads as now.
 */
export function formatMenuTime(iso) {
  const d = parseServerTime(iso) || new Date();
  return d.toDateString() === new Date().toDateString()
    ? d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString([], { month: 'short', day: 'numeric' });
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
