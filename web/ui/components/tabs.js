/**
 * tabs — the shared tab strip a window shows when it holds several documents
 * or sessions (Terminal, Browser).
 *
 * The strip is meant to be the window's top bar: it carries `data-window-bar`
 * (the core chrome merges it into the title row next to close/fullscreen) and
 * `data-compact="off"` (its children are tabs, not actions to fold). Tabs are
 * flat mono capitals; the active one is a pill; a tab's close shows on hover,
 * on the active tab and on keyboard focus.
 *
 * The owner keeps the tab list and re-renders the strip from it; domain
 * classes (`terminal-tab`, `browser-tab`, `is-incognito`, …) are added on top
 * of the `.ui-tab*` ones as layout hooks.
 */

import { icon } from '../icon.js';
import { controlButton } from './button.js';

/**
 * @param {object} o
 * @param {string} o.label             accessible name of the strip
 * @param {string} [o.className]       domain class (layout hook)
 * @param {boolean} [o.windowBar=true] merge into the window's title row
 */
export function tabStrip({ label, className = '', windowBar = true } = {}) {
  const strip = document.createElement('div');
  strip.className = `ui-tabstrip ${className}`.trim();
  strip.setAttribute('role', 'tablist');
  if (label) strip.setAttribute('aria-label', label);
  if (windowBar) strip.dataset.windowBar = '';
  strip.dataset.compact = 'off';
  return strip;
}

/**
 * One tab.
 *
 * @param {object} o
 * @param {string} o.label             the visible name
 * @param {string} [o.title]           tooltip (defaults to the label)
 * @param {boolean} [o.active]
 * @param {() => void} [o.onSelect]
 * @param {() => void} [o.onClose]     omit for a tab that cannot be closed
 * @param {string} [o.closeLabel]      accessible name of the close button
 * @param {Node} [o.leading]           a small glyph before the name (incognito mask)
 * @param {string} [o.className]       domain class (layout hook)
 * @returns {HTMLElement & { labelButton: HTMLButtonElement, closeButton: HTMLButtonElement|null }}
 */
export function tab({
  label, title, active = false, onSelect, onClose, closeLabel, leading, className = '',
} = {}) {
  const el = document.createElement('div');
  el.className = `ui-tab ${className}`.trim();
  el.classList.toggle('is-active', !!active);
  el.setAttribute('role', 'tab');
  el.setAttribute('aria-selected', String(!!active));

  const name = controlButton();
  name.className = 'ui-tab-label';
  if (leading) {
    leading.classList?.add('ui-tab-leading');
    name.appendChild(leading);
  }
  const text = document.createElement('span');
  text.className = 'ui-tab-text';
  text.textContent = label || '';
  name.appendChild(text);
  name.title = title || label || '';
  if (onSelect) name.addEventListener('click', () => onSelect());
  el.appendChild(name);
  el.labelButton = name;
  el.closeButton = null;

  if (onClose) {
    const close = controlButton();
    close.className = 'ui-tab-close';
    close.setAttribute('aria-label', closeLabel || label || '');
    close.appendChild(icon('ui/close', { size: 12 }));
    close.addEventListener('click', (e) => {
      e.stopPropagation();
      onClose();
    });
    el.appendChild(close);
    el.closeButton = close;
  }
  return el;
}
