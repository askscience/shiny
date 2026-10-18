import { element as el } from '../../ui/index.js';
import { icon, controlCard, controlGroup } from '../../ui/index.js';

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

export function panel(id, iconName, title, children) {
  const section = el('section', 'settings-section settings-panel');
  section.id = `panel-${id}`;
  section.dataset.panel = id;
  const head = el('div', 'settings-panel-head');
  const ring = el('span', 'settings-panel-icon');
  ring.appendChild(icon(iconName, { size: 22 }));
  head.append(ring, el('h3', null, title));
  section.appendChild(head);
  section.append(...settingsGroups(children));
  return section;
}

/** Explicit heading boundaries keep the existing section order and translations. */
export function settingsGroups(children) {
  const groups = [];
  let group;
  let previous;
  for (const child of children.filter(Boolean)) {
    if (!group || child.classList.contains('settings-subhead')) {
      group = controlGroup({ heading: child.classList.contains('settings-subhead') ? child : null });
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

/* ── Appearance ─────────────────────────────────────────────── */

