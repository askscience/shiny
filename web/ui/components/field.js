/** field — form controls: input, textarea, select, toggle, slider, checkbox, search. */

import { icon } from '../icon.js';
import { controlButton, iconButton } from './button.js';
import { t } from '../../js/i18n.js';
import { enterSurface, disposeMotion } from '../motion.js';
import { placePopover } from '../popover.js';

/** Label + control + hint wrapper. */
let fieldId = 0;
export function field({ label, hint, control, htmlFor } = {}) {
  const wrap = document.createElement('div');
  wrap.className = 'ui-field';
  if (label) {
    const l = document.createElement('label');
    l.className = 'ui-label';
    l.textContent = label;
    const target = control?.select || control;
    if (target && !target.id) target.id = `ui-field-${++fieldId}`;
    if (htmlFor || target?.id) l.htmlFor = htmlFor || target.id;
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
  for (const key of ['min', 'max', 'step', 'name', 'inputMode']) if (o[key] != null) el[key] = o[key];
  if (o.label) el.setAttribute('aria-label', o.label);
  if (o.className) el.classList.add(...o.className.split(/\s+/).filter(Boolean));
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
 * Styled select. options: [{value, label}] or [string].
 * Returns the wrapper; `wrap.select` is the `<select>` that holds the value and
 * supplies the closed field's look.
 *
 * The platform popup is never opened. QtWebEngine 6.8 on X11 segfaults the
 * whole shell when a native `<select>` dropdown opens (QTBUG-135036, fixed
 * upstream in Qt 6.9.2): the popup is a native window and Qt then handles an X
 * event for a window that is already gone. So the look stays the styled
 * `<select>`'s, while the open list is an in-page menu (`.ui-select-menu`).
 */
export function selectControl(o = {}) {
  const el = document.createElement('select');
  el.className = 'ui-select';
  if (o.id) el.id = o.id;
  if (o.label) el.setAttribute('aria-label', o.label);
  el.setOptions = (options = []) => {
    if (openSelect?.select === el) closeSelectMenu();
    el.textContent = '';
    for (const opt of options) {
      const node = document.createElement('option');
      if (typeof opt === 'string') { node.value = opt; node.textContent = opt; }
      else { node.value = opt.value; node.textContent = opt.label; node.disabled = !!opt.disabled; }
      el.appendChild(node);
    }
    if (o.value != null) el.value = o.value;
  };
  el.setOptions(o.options || []);
  if (o.onChange) el.addEventListener('change', (e) => o.onChange(e.target.value, e));
  useSelectMenu(el);
  return el;
}

export function select(o = {}) {
  const wrap = document.createElement('div');
  wrap.className = 'ui-select-wrap';
  const el = selectControl(o);
  wrap.setOptions = el.setOptions;
  wrap.appendChild(el);
  wrap.appendChild(icon('ui/chevron-down'));
  wrap.select = el;
  return wrap;
}

/**
 * Arm a `<select>` to open the in-page menu instead of the platform popup.
 * `select()` does this for the shared component; call it for raw selects a
 * plugin builds itself. Keeping the platform popup closed is what keeps the
 * shell alive (see the note on `select()`).
 */
export function useSelectMenu(el) {
  if (el.dataset.selectMenuArmed) return el;
  el.dataset.selectMenuArmed = '1';
  el.addEventListener('pointerdown', (e) => {
    if (e.button != null && e.button !== 0) return;
    e.preventDefault();
    openSelectMenu(el);
  });
  el.addEventListener('mousedown', (e) => e.preventDefault());
  el.addEventListener('click', (e) => e.preventDefault());
  el.addEventListener('keydown', (e) => {
    if (e.key === ' ' || e.key === 'Enter' || e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      openSelectMenu(el);
    }
  });
  return el;
}

/** The one open dropdown, if any. */
let openSelect = null;

/** Close the open dropdown (no-op when none is open). */
export function closeSelectMenu() {
  if (!openSelect) return;
  const menu = openSelect;
  openSelect = null;
  menu.dispose();
}

function openSelectMenu(el) {
  if (el.disabled) return;
  const wasOpenFor = openSelect?.select === el;
  closeSelectMenu();
  if (wasOpenFor) return; // second click on the same field toggles it shut
  const options = [...el.options];
  const enabled = options.map((opt, index) => opt.disabled ? -1 : index).filter(index => index >= 0);
  if (!enabled.length) return;

  let active = Math.max(0, options.findIndex((opt) => opt.value === el.value));
  if (options[active].disabled) active = enabled[0];
  const items = [];
  const menu = document.createElement('div');
  menu.className = 'ui-select-menu';
  menu.setAttribute('role', 'listbox');
  menu.tabIndex = -1;

  const syncActive = (scroll = false) => {
    items.forEach((item, i) => item.classList.toggle('is-active', i === active));
    if (scroll) items[active]?.scrollIntoView({ block: 'nearest' });
  };
  const choose = (index) => {
    if (options[index]?.disabled) return;
    const value = options[index]?.value;
    closeSelectMenu();
    if (value == null) return;
    if (el.value !== value) {
      el.value = value;
      el.dispatchEvent(new Event('change', { bubbles: true }));
    }
    el.focus();
  };

  options.forEach((opt, i) => {
    const item = document.createElement('div');
    item.className = 'ui-select-option';
    item.setAttribute('role', 'option');
    item.dataset.value = opt.value;
    item.setAttribute('aria-disabled', String(!!opt.disabled));
    item.setAttribute('aria-selected', String(opt.value === el.value));
    item.textContent = opt.label || opt.textContent || opt.value;
    item.addEventListener('click', (e) => { e.preventDefault(); e.stopPropagation(); choose(i); });
    items.push(item);
    menu.appendChild(item);
  });
  syncActive();

  const place = () => {
    placePopover(menu, el);
  };

  const onDocPointerDown = (e) => {
    if (!menu.contains(e.target) && !el.contains(e.target)) closeSelectMenu();
  };
  const anchorObserver = typeof MutationObserver !== 'undefined'
    ? new MutationObserver(() => {
      if (!el.isConnected || el.closest('[hidden],.hidden')) closeSelectMenu();
    }) : null;
  const onWheel = (e) => {
    if (!menu.contains(e.target)) closeSelectMenu();
  };
  const onKeyDown = (e) => {
    const position = enabled.indexOf(active);
    switch (e.key) {
      case 'ArrowDown': active = enabled[Math.min(enabled.length - 1, position + 1)]; break;
      case 'ArrowUp': active = enabled[Math.max(0, position - 1)]; break;
      case 'Home': active = enabled[0]; break;
      case 'End': active = enabled[enabled.length - 1]; break;
      case 'Enter': case ' ': choose(active); e.preventDefault(); return;
      case 'Escape': closeSelectMenu(); el.focus(); e.preventDefault(); return;
      case 'Tab': closeSelectMenu(); return;
      default: return;
    }
    e.preventDefault();
    syncActive(true);
  };
  const dispose = () => {
    disposeMotion(menu);
    anchorObserver?.disconnect();
    document.removeEventListener('pointerdown', onDocPointerDown, true);
    window.removeEventListener('wheel', onWheel, true);
    window.removeEventListener('resize', closeSelectMenu);
    window.removeEventListener('blur', closeSelectMenu);
    window.visualViewport?.removeEventListener('resize', closeSelectMenu);
    window.visualViewport?.removeEventListener('scroll', closeSelectMenu);
    menu.removeEventListener('keydown', onKeyDown);
    menu.remove();
  };

  document.body.appendChild(menu);
  place();
  enterSurface(menu);
  menu.focus();
  menu.addEventListener('keydown', onKeyDown);
  document.addEventListener('pointerdown', onDocPointerDown, true);
  window.addEventListener('wheel', onWheel, { capture: true, passive: true });
  window.addEventListener('resize', closeSelectMenu);
  window.addEventListener('blur', closeSelectMenu);
  window.visualViewport?.addEventListener('resize', closeSelectMenu);
  window.visualViewport?.addEventListener('scroll', closeSelectMenu);
  openSelect = { select: el, dispose };
  anchorObserver?.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'hidden'] });
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
  if (o.label) t.setAttribute('aria-label', o.label);
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

/** Search field: a pill with the magnifier as its round action at the end. */
export function searchBar(o = {}) {
  const wrap = document.createElement('div');
  wrap.className = 'ui-search';
  const label = o.label || o.placeholder || t('common.search');
  const el = input({ placeholder: o.placeholder || t('common.search'), label, value: o.value, onInput: o.onInput, autocomplete: 'off' });
  wrap.appendChild(el);
  if (o.onClear) {
    const clear = iconButton({ icon: 'ui/close', label: o.clearLabel || t('common.clear'), size: 'sm', onClick: () => {
      el.value = '';
      el.focus();
      o.onClear();
    } });
    clear.classList.add('ui-search-clear');
    wrap.classList.add('ui-search--clearable');
    wrap.appendChild(clear);
  }
  /* The glyph is the field's affordance, not a second action: it hands the
     caret back to the input and stays out of the tab order (the input carries
     the label), so keyboard users are not given a dead stop. */
  const action = controlButton();
  action.classList.add('ui-search-action');
  action.tabIndex = -1;
  action.setAttribute('aria-hidden', 'true');
  action.title = label;
  action.appendChild(icon('ui/search', { size: 15 }));
  action.addEventListener('click', () => el.focus());
  wrap.appendChild(action);
  wrap.input = el;
  return wrap;
}
