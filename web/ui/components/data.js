/** data — list, list item, stat, chip, avatar, key-value rows. */

import { icon } from '../icon.js';

export function list(items = [], renderItem) {
  const el = document.createElement('div');
  el.className = 'ui-list';
  el.replace = (next = []) => {
    el.textContent = '';
    for (const item of next) {
      const node = renderItem(item);
      if (node) el.appendChild(node);
    }
  };
  el.replace(items);
  return el;
}

/** Row with leading icon/avatar, title+subtitle, trailing node. */
export function listItem(o = {}) {
  const el = document.createElement(o.onClick ? 'button' : 'div');
  el.className = 'ui-list-item';
  if (o.onClick) { el.type = 'button'; el.addEventListener('click', o.onClick); }

  if (o.leading) {
    el.appendChild(typeof o.leading === 'string' ? icon(o.leading, { size: 18 }) : o.leading);
  }
  const main = document.createElement('div');
  main.className = 'ui-list-item-main';
  const title = document.createElement('div');
  title.className = 'ui-list-item-title';
  title.textContent = o.title || '';
  main.appendChild(title);
  if (o.subtitle) {
    const sub = document.createElement('div');
    sub.className = 'ui-list-item-sub';
    sub.textContent = o.subtitle;
    main.appendChild(sub);
  }
  el.appendChild(main);
  if (o.trailing) {
    el.appendChild(typeof o.trailing === 'string' ? icon(o.trailing, { size: 14 }) : o.trailing);
  }
  el.main = main;
  return el;
}

/** Big number + unit + label. */
export function stat(o = {}) {
  const el = document.createElement('div');
  el.className = 'ui-stat';
  const value = document.createElement('div');
  value.className = 'ui-stat-value';
  const num = document.createElement('span');
  num.textContent = o.value ?? '—';
  value.appendChild(num);
  if (o.unit) {
    const unit = document.createElement('span');
    unit.className = 'ui-stat-unit';
    unit.textContent = o.unit;
    value.appendChild(unit);
  }
  el.appendChild(value);
  if (o.label) {
    const label = document.createElement('div');
    label.className = 'ui-stat-label';
    label.textContent = o.label;
    el.appendChild(label);
  }
  el.setValue = (v) => { num.textContent = v; };
  return el;
}

/** Rounded interactive chip. */
export function chip(o = {}) {
  const el = document.createElement('button');
  el.type = 'button';
  el.className = 'ui-chip';
  if (o.active) el.classList.add('is-active');
  if (o.icon) el.appendChild(icon(o.icon, { size: 15 }));
  const label = document.createElement('span');
  label.textContent = o.label || '';
  el.appendChild(label);
  el.setActive = (v) => el.classList.toggle('is-active', v);
  if (o.onClick) el.addEventListener('click', o.onClick);
  return el;
}

/** Circular avatar with image or initials fallback. */
export function avatar(o = {}) {
  const el = document.createElement('span');
  el.className = `ui-avatar${o.size === 'lg' ? ' ui-avatar--lg' : ''}`;
  if (o.size && o.size !== 'lg' && Number.isFinite(o.size)) {
    el.style.width = el.style.height = `${o.size}px`;
  }
  el.set = ({ name, src } = {}) => {
    el.textContent = '';
    if (src) {
      const img = document.createElement('img');
      img.src = src;
      img.alt = name || '';
      el.appendChild(img);
    } else {
      el.textContent = (name || '?').trim().charAt(0).toUpperCase() || '?';
    }
  };
  el.set({ name: o.name, src: o.src });
  return el;
}

/** Label/value rows (artifact "sections"). rows: [{label, value}] or [[k, v]]. */
export function keyValue(rows = []) {
  const el = document.createElement('div');
  el.className = 'ui-kv';
  el.replace = (next = []) => {
    el.textContent = '';
    for (const row of next) {
      const [k, v] = Array.isArray(row) ? row : [row.label, row.value];
      const line = document.createElement('div');
      line.className = 'ui-kv-row';
      const key = document.createElement('span');
      key.className = 'ui-kv-key';
      key.textContent = k ?? '';
      const val = document.createElement('span');
      val.className = 'ui-kv-val';
      val.textContent = v ?? '';
      line.append(key, val);
      el.appendChild(line);
    }
  };
  el.replace(rows);
  return el;
}
