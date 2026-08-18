/** button — .ui-btn factories. */

import { icon } from '../icon.js';

/**
 * @param {object} o
 * @param {string} [o.label]
 * @param {'primary'|'ghost'|'quiet'|'danger'} [o.variant]
 * @param {'sm'|'md'|'lg'} [o.size]
 * @param {string|Node} [o.icon]  theme icon name or ready node
 * @param {boolean} [o.block]     full width
 * @param {boolean} [o.loading]
 * @param {boolean} [o.disabled]
 * @param {string} [o.type]       button type attr
 * @param {(e: MouseEvent) => void} [o.onClick]
 */
export function button(o = {}) {
  const el = document.createElement(o.href ? 'a' : 'button');
  el.className = `ui-btn ui-btn--${o.variant || 'primary'}`;
  if (o.size && o.size !== 'md') el.classList.add(`ui-btn--${o.size}`);
  if (o.block) el.classList.add('ui-btn--block');
  if (o.href) { el.href = o.href; }
  else el.type = o.type || 'button';
  if (o.disabled) el.disabled = true;

  const setContent = () => {
    el.textContent = '';
    if (o.loading) {
      const spin = document.createElement('span');
      spin.className = 'ui-spinner';
      el.appendChild(spin);
    }
    if (o.icon) {
      el.appendChild(typeof o.icon === 'string' ? icon(o.icon, { size: 16 }) : o.icon);
    }
    if (o.label) {
      const span = document.createElement('span');
      span.textContent = o.label;
      el.appendChild(span);
    }
  };
  setContent();

  el.setLoading = (v) => { o.loading = v; el.classList.toggle('is-loading', v); el.disabled = v || !!o.disabled; setContent(); };
  el.setLabel = (label) => { o.label = label; setContent(); };
  if (o.onClick) el.addEventListener('click', o.onClick);
  return el;
}

/** Circular icon-only button. */
export function iconButton(o = {}) {
  const el = button({
    variant: o.variant || 'ghost',
    size: o.size,
    icon: o.icon || 'ui/info',
    onClick: o.onClick,
    disabled: o.disabled,
  });
  el.classList.add('ui-btn--icon');
  if (o.label) {
    el.setAttribute('aria-label', o.label);
    el.title = o.label;
  }
  return el;
}
