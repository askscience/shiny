import { element as el } from '../../ui/index.js';
import { controlCard, controlGroup, segmented } from '../../ui/index.js';
import { t } from '../i18n.js';

let nextControlId = 0;

export { el };

export function field(labelText, control, { hint, id, details } = {}) {
  const wrap = controlCard();
  wrap.classList.add('settings-field');
  const target = control?.select || (control?.tagName === 'INPUT' ? control : control?.querySelector?.('input'));
  if (target) target.id = id || target.id || `settings-control-${++nextControlId}`;
  if (labelText) {
    const l = el('label', 'settings-label', labelText);
    if (id || target?.id) l.htmlFor = id || target.id;
    wrap.appendChild(l);
  }
  if (control) wrap.appendChild(control);
  if (hint) wrap.appendChild(el('p', 'settings-hint', hint));
  if (details) wrap.appendChild(details);
  return wrap;
}

export function sliderField(labelText, valueText, control) {
  const wrap = controlCard();
  wrap.classList.add('settings-field');
  const l = el('label', 'settings-label');
  control.id ||= `settings-control-${++nextControlId}`;
  l.htmlFor = control.id;
  l.append(document.createTextNode(labelText + ' '));
  const value = el('span', 'settings-value', valueText);
  l.appendChild(value);
  wrap.append(l, control);
  return { wrap, value };
}

export function heading(text) {
  const h = el('h4', 'settings-subhead', text);
  return h;
}

export function panel(id, iconName, title, children, { index = 0, total = 0, description = '' } = {}) {
  const section = el('section', 'settings-section settings-panel');
  section.id = `panel-${id}`;
  section.dataset.panel = id;
  // Editorial head: a mono "02 / 10" kicker over a large display title.
  const head = el('header', 'settings-panel-head');
  if (index && total) {
    const kicker = el('span', 'settings-panel-kicker', `${String(index).padStart(2, '0')} / ${String(total).padStart(2, '0')}`);
    kicker.setAttribute('aria-label', t('settings.panel.kicker', { current: index, total }, 'Section {current} of {total}'));
    head.appendChild(kicker);
  }
  head.appendChild(el('h3', 'settings-panel-title', title));
  if (description) head.appendChild(el('p', 'settings-panel-subtitle', description));
  section.appendChild(head);
  section.append(...settingsGroups(children));
  return section;
}

/**
 * Explicit heading boundaries keep the existing section order and translations.
 * A `.settings-raw` child (a panel's own tab bar) is placed as is, between the
 * groups; a heading's `data-page` carries over to its group, so a panel can
 * show one page of groups at a time (see `pageTabs`).
 */
export function settingsGroups(children) {
  const groups = [];
  let group;
  let previous;
  for (const child of children.filter(Boolean)) {
    if (child.classList.contains('settings-raw')) {
      groups.push(child);
      group = null;
      previous = null;
      continue;
    }
    if (!group || child.classList.contains('settings-subhead')) {
      group = controlGroup({ heading: child.classList.contains('settings-subhead') ? child : null });
      if (child.dataset?.page) group.dataset.page = child.dataset.page;
      groups.push(group);
      previous = null;
      if (child.classList.contains('settings-subhead')) continue;
    }
    if (child.classList.contains('settings-hint') && previous) {
      previous.appendChild(child);
      continue;
    }
    const card = child.classList.contains('ui-control-card') ? child : controlCard({ children: child, wide: !child.classList.contains('ui-toggle-row') });
    group.grid.appendChild(card);
    previous = card;
  }
  return groups;
}

/** A heading that opens a group on page `page` of a paged panel. */
export function pageHeading(text, page) {
  const h = heading(text);
  h.dataset.page = page;
  return h;
}

/**
 * Tabs over a long panel: one page of groups at a time. Groups are matched by
 * their heading's `data-page`; the choice is remembered per panel.
 */
export function pageTabs({ id, label, pages }) {
  const key = `settings.page.${id}`;
  let current = pages[0].value;
  try {
    const saved = localStorage.getItem(key);
    if (pages.some((p) => p.value === saved)) current = saved;
  } catch (_) { /* storage unavailable: first page */ }
  const bar = el('div', 'settings-raw settings-page-tabs');
  const show = (value) => {
    const section = bar.closest('.settings-panel');
    if (!section) return;
    for (const group of section.querySelectorAll(':scope > .ui-control-group[data-page]')) {
      group.hidden = group.dataset.page !== value;
    }
  };
  const tabs = segmented({
    label,
    options: pages,
    value: current,
    onChange: (value) => {
      current = value;
      try { localStorage.setItem(key, value); } catch (_) { /* best effort */ }
      show(value);
    },
  });
  bar.appendChild(tabs);
  // The groups exist once the panel has been assembled around this bar.
  queueMicrotask(() => show(current));
  return bar;
}

/* ── Appearance ─────────────────────────────────────────────── */

