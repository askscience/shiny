/**
 * hudMenuShared.js — the anatomy shared by the top-bar status menus (Sound,
 * Network, Bluetooth, Battery), so all four are built from one set of parts
 * and one `.hud-menu-*` class family (web/css/app.css; the Dusk/Ink skin in
 * web/themes/editorial/app.css turns the panel into a dark glass island).
 *
 *   popup   `.ui-hud-menu-popup.hud-menu` — head, scrolling body, footer
 *   head    leading icon, title + hint, trailing controls (a power switch)
 *   label   small mono capitals ("OUTPUT"), optional trailing count/button
 *   row     40px: leading monochrome icon, title (+ optional sub line),
 *           trailing bits; the selected row ends in an accent check
 *   slider  a row with a leading icon/button, a thin range and a mono value
 *   foot    a mono hint on the left, a grey pill button on the right
 *
 * Motion is not decided here: the popup's open/close belongs to
 * menuFactory.js and the shared motion layer.
 */

import { controlButton, icon } from '../ui/index.js';

export function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/** The popup shell: `extraClass` names the menu (`audio-menu`, …). */
export function menuPopup(extraClass, ariaLabel) {
  const popup = el('div', `ui-hud-menu-popup hud-menu ${extraClass} hidden`);
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', ariaLabel);
  const body = el('div', 'hud-menu-body');
  return { popup, body };
}

/**
 * Head row: `{ head, iconEl, titleEl, hintEl, actions }`. `actions` is the
 * trailing slot for a switch or an icon button.
 */
export function menuHead({ iconName, title }) {
  const head = el('div', 'hud-menu-head');
  const iconEl = icon(iconName, { size: 18, className: 'hud-menu-icon' });
  const main = el('div', 'hud-menu-main');
  const titleEl = el('div', 'hud-menu-head-title', title);
  const hintEl = el('div', 'hud-menu-head-hint');
  main.append(titleEl, hintEl);
  const actions = el('div', 'hud-menu-head-actions');
  head.append(iconEl, main, actions);
  return { head, iconEl, titleEl, hintEl, actions };
}

/** Section label in mono capitals; `trailing` nodes sit at its right end. */
export function menuLabel(text, ...trailing) {
  const label = el('div', 'hud-menu-label');
  label.appendChild(el('span', 'hud-menu-label-text', text));
  for (const node of trailing) if (node) label.appendChild(node);
  return label;
}

/** A small mono value (a count, a percentage, a battery level). */
export function menuValue(text, className = '') {
  return el('span', `hud-menu-value${className ? ` ${className}` : ''}`, text);
}

/** A quiet line of text inside the body (empty list, unavailable, …). */
export function menuNote(text, className = '') {
  return el('p', `hud-menu-note${className ? ` ${className}` : ''}`, text);
}

/**
 * A 40px row. `iconName` (or a ready `leading` node), a title and an
 * optional sub line; `trailing` nodes at the right; `selected` adds the
 * accent check. With `onClick` the row is a keyboard-reachable button, and
 * clicks on a nested button (a trailing action) never reach it.
 */
export function menuRow({
  iconName, leading, title, sub, trailing = [], selected = false,
  onClick, disabled = false, tooltip, role,
} = {}) {
  const row = el('div', 'hud-menu-row');
  if (selected) row.classList.add('is-selected');
  if (sub) row.classList.add('has-sub');
  if (tooltip) row.title = tooltip;

  const lead = leading || (iconName ? icon(iconName, { size: 17, className: 'hud-menu-icon' }) : null);
  if (lead) row.appendChild(lead);

  const main = el('div', 'hud-menu-main');
  main.appendChild(el('div', 'hud-menu-title', title || ''));
  if (sub) main.appendChild(el('div', 'hud-menu-sub', sub));
  row.appendChild(main);

  const tail = (Array.isArray(trailing) ? trailing : [trailing]).filter(Boolean);
  if (tail.length || selected) {
    const end = el('span', 'hud-menu-trailing');
    for (const node of tail) end.appendChild(node);
    if (selected) end.appendChild(icon('ui/check', { size: 14, className: 'hud-menu-check' }));
    row.appendChild(end);
  }

  if (role) row.setAttribute('role', role);
  if (onClick && !disabled) {
    row.classList.add('is-action');
    if (!role) row.setAttribute('role', 'button');
    row.tabIndex = 0;
    const fire = (event) => {
      if (event.target !== row && event.target.closest?.('button, input, a')) return;
      onClick(event);
    };
    row.addEventListener('click', fire);
    row.addEventListener('keydown', (event) => {
      if (event.target !== row) return;
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        onClick(event);
      }
    });
  }
  if (disabled) row.classList.add('is-disabled');
  row.main = main;
  return row;
}

/** Keep the range's filled part (`--fill`) in step with its value. */
export function syncSliderFill(range) {
  const min = Number(range.min) || 0;
  const max = Number(range.max) || 100;
  const value = Number(range.value) || 0;
  const pct = max > min ? Math.min(100, Math.max(0, ((value - min) / (max - min)) * 100)) : 0;
  range.style.setProperty('--fill', `${pct}%`);
}

/**
 * Slider row: a leading node (an icon, or a mute button), the range, and a
 * mono value. The range's fill follows every input.
 */
export function menuSlider({ leading, range, valueText = '' }) {
  const row = el('div', 'hud-menu-row hud-menu-slider');
  range.classList.add('hud-menu-range');
  range.addEventListener('input', () => syncSliderFill(range));
  syncSliderFill(range);
  const value = menuValue(valueText, 'hud-menu-slider-value');
  if (leading) row.appendChild(leading);
  row.append(range, value);
  return { row, value };
}

/** Grey pill button; `arrow` adds the trailing → of a "go somewhere" action. */
export function menuPill({ label, iconName, onClick, arrow = true }) {
  const btn = controlButton();
  btn.className = `hud-menu-pill${arrow ? ' has-arrow' : ''}`;
  if (iconName) btn.appendChild(icon(iconName, { size: 13 }));
  btn.appendChild(el('span', 'hud-menu-pill-label', label));
  if (onClick) btn.addEventListener('click', onClick);
  return btn;
}

/** Footer: `{ foot, hintEl }` — a mono hint on the left, `actions` right. */
export function menuFoot(...actions) {
  const foot = el('div', 'hud-menu-foot');
  const hintEl = el('span', 'hud-menu-foot-hint');
  foot.appendChild(hintEl);
  for (const node of actions) if (node) foot.appendChild(node);
  return { foot, hintEl };
}
