/** button — .ui-btn factories. */

import { icon } from '../icon.js';

/** Unskinned domain button: shared interaction rules, caller-owned layout. */
export function controlButton() {
  const el = document.createElement('button');
  el.type = 'button';
  el.dataset.uiControl = 'button';
  return el;
}

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
  const el = o.href ? document.createElement('a') : controlButton();
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
      el.appendChild(typeof o.icon === 'string' ? icon(o.icon, { size: o.iconSize || 16 }) : o.icon);
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
  if (o.className) el.classList.add(...o.className.split(/\s+/).filter(Boolean));
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
    iconSize: o.iconSize,
    className: o.className,
  });
  el.classList.add('ui-btn--icon');
  if (o.label) {
    el.setAttribute('aria-label', o.label);
    el.title = o.label;
  }
  return el;
}

/** Compact toolbar action, with the same accessible name and styling everywhere. */
export function toolButton(icon, label, onClick, options = {}) {
  return iconButton({ icon, label, onClick, size: 'sm', ...options,
    className: `ui-tool${options.danger ? ' ui-tool--danger' : ''}${options.className ? ` ${options.className}` : ''}` });
}

/** Consistent document/account menu trigger. The owner controls menu state. */
export function menuButton({ icon: name, label, onClick, className = '', text = '' } = {}) {
  const el = button({ variant: 'ghost', icon: name, iconSize: 15, onClick,
    className: `ui-menu-trigger ${className}` });
  el.title = label;
  el.setAttribute('aria-label', label);
  el.setAttribute('aria-haspopup', 'menu');
  el.setAttribute('aria-expanded', 'false');
  const caption = document.createElement('span');
  caption.className = 'ui-menu-trigger-label';
  caption.textContent = text;
  caption.hidden = !text;
  el.appendChild(caption);
  el.appendChild(icon('ui/chevron-down', { size: 12 }));
  el.caption = caption;
  return el;
}
