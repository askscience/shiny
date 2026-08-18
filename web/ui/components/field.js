/** field — form controls: input, textarea, select, toggle, slider, checkbox, search. */

import { icon } from '../icon.js';

/** Label + control + hint wrapper. */
export function field({ label, hint, control, htmlFor } = {}) {
  const wrap = document.createElement('div');
  wrap.className = 'ui-field';
  if (label) {
    const l = document.createElement('label');
    l.className = 'ui-label';
    l.textContent = label;
    if (htmlFor) l.htmlFor = htmlFor;
    wrap.appendChild(l);
  }
  if (control) wrap.appendChild(control);
  if (hint) {
    const p = document.createElement('p');
    p.className = 'ui-hint';
    p.textContent = hint;
    wrap.appendChild(p);
  }
  return wrap;
}

export function input(o = {}) {
  const el = document.createElement('input');
  el.className = 'ui-input';
  el.type = o.type || 'text';
  if (o.id) el.id = o.id;
  if (o.value != null) el.value = o.value;
  if (o.placeholder) el.placeholder = o.placeholder;
  if (o.maxlength) el.maxLength = o.maxlength;
  if (o.autocomplete) el.autocomplete = o.autocomplete;
  if (o.autocapitalize) el.autocapitalize = o.autocapitalize;
  if (o.onInput) el.addEventListener('input', (e) => o.onInput(e.target.value, e));
  return el;
}

export function textarea(o = {}) {
  const el = document.createElement('textarea');
  el.className = 'ui-textarea';
  if (o.id) el.id = o.id;
  el.rows = o.rows || 3;
  if (o.value != null) el.value = o.value;
  if (o.placeholder) el.placeholder = o.placeholder;
  if (o.onInput) el.addEventListener('input', (e) => o.onInput(e.target.value, e));
  return el;
}

/**
 * Styled native select. options: [{value, label}] or [string].
 * Returns the wrapper; `wrap.select` is the raw <select>.
 */
export function select(o = {}) {
  const wrap = document.createElement('div');
  wrap.className = 'ui-select-wrap';
  const el = document.createElement('select');
  el.className = 'ui-select';
  if (o.id) el.id = o.id;
  wrap.setOptions = (options = []) => {
    el.textContent = '';
    for (const opt of options) {
      const node = document.createElement('option');
      if (typeof opt === 'string') { node.value = opt; node.textContent = opt; }
      else { node.value = opt.value; node.textContent = opt.label; }
      el.appendChild(node);
    }
    if (o.value != null) el.value = o.value;
  };
  wrap.setOptions(o.options || []);
  if (o.onChange) el.addEventListener('change', (e) => o.onChange(e.target.value, e));
  wrap.appendChild(el);
  wrap.appendChild(icon('ui/chevron-down'));
  wrap.select = el;
  return wrap;
}

/** Switch. Returns button[role=switch]; `.setChecked(v)`, change via onChange. */
export function toggle(o = {}) {
  const el = document.createElement('button');
  el.type = 'button';
  el.className = 'ui-toggle';
  el.setAttribute('role', 'switch');
  let checked = !!o.checked;
  const sync = () => el.setAttribute('aria-checked', String(checked));
  el.setChecked = (v, { silent } = {}) => {
    checked = !!v;
    sync();
    if (!silent) o.onChange?.(checked);
  };
  el.isChecked = () => checked;
  el.addEventListener('click', () => el.setChecked(!checked));
  sync();
  return el;
}

/** Labelled toggle row (label left, switch right). */
export function toggleRow(o = {}) {
  const row = document.createElement('div');
  row.className = 'ui-row ui-toggle-row';
  const main = document.createElement('div');
  main.className = 'ui-list-item-main';
  const title = document.createElement('div');
  title.className = 'ui-list-item-title';
  title.textContent = o.label || '';
  main.appendChild(title);
  if (o.hint) {
    const sub = document.createElement('div');
    sub.className = 'ui-list-item-sub';
    sub.textContent = o.hint;
    main.appendChild(sub);
  }
  const t = toggle(o);
  row.append(main, t);
  row.toggle = t;
  return row;
}

export function slider(o = {}) {
  const el = document.createElement('input');
  el.type = 'range';
  el.className = 'ui-slider';
  el.min = o.min ?? 0;
  el.max = o.max ?? 100;
  el.step = o.step ?? 1;
  el.value = o.value ?? 0;
  if (o.onInput) el.addEventListener('input', (e) => o.onInput(Number(e.target.value), e));
  return el;
}

export function checkbox(o = {}) {
  const el = document.createElement('button');
  el.type = 'button';
  el.className = 'ui-checkbox';
  const box = document.createElement('span');
  box.className = 'ui-checkbox-box';
  box.appendChild(icon('ui/check'));
  const text = document.createElement('span');
  text.textContent = o.label || '';
  el.append(box, text);
  let checked = !!o.checked;
  const sync = () => el.classList.toggle('is-checked', checked);
  el.setChecked = (v, { silent } = {}) => {
    checked = !!v;
    sync();
    if (!silent) o.onChange?.(checked);
  };
  el.isChecked = () => checked;
  el.addEventListener('click', () => el.setChecked(!checked));
  sync();
  return el;
}

/** Search field with leading icon. */
export function searchBar(o = {}) {
  const wrap = document.createElement('div');
  wrap.className = 'ui-search';
  wrap.appendChild(icon('ui/search'));
  const el = input({ placeholder: o.placeholder || 'Search…', value: o.value, onInput: o.onInput });
  wrap.appendChild(el);
  wrap.input = el;
  return wrap;
}
