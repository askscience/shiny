/** card — surfaces: card, panel, section header, divider, stack, row. */

export function card(o = {}) {
  const el = document.createElement('div');
  el.className = 'ui-card';
  if (o.flat) el.classList.add('ui-card--flat');
  if (o.reveal != null) el.dataset.reveal = o.reveal === true ? '' : String(o.reveal);

  const body = document.createElement('div');
  body.className = 'ui-card-body';
  if (o.eyebrow || o.title || o.subtitle) body.appendChild(section(o));
  for (const node of [].concat(o.body || [])) {
    if (node == null) continue;
    body.appendChild(typeof node === 'string' ? text(node) : node);
  }
  el.appendChild(body);

  const actions = [].concat(o.actions || []).filter(Boolean);
  if (actions.length) {
    const foot = document.createElement('div');
    foot.className = 'ui-card-foot';
    foot.append(...actions);
    el.appendChild(foot);
  }
  el.body = body;
  return el;
}

function text(str) {
  const p = document.createElement('p');
  p.className = 'ui-subtitle';
  p.textContent = str;
  return p;
}

/** Glass surface container. */
export function panel(children = [], { className = '' } = {}) {
  const el = document.createElement('div');
  el.className = `ui-panel${className ? ` ${className}` : ''}`;
  for (const node of [].concat(children)) {
    if (node == null) continue;
    el.appendChild(typeof node === 'string' ? text(node) : node);
  }
  return el;
}

/** Section header: eyebrow + title + subtitle, optional trailing actions. */
export function section(o = {}) {
  const el = document.createElement('div');
  el.className = 'ui-section';
  const head = document.createElement('div');
  head.className = 'ui-section-head';
  if (o.eyebrow) {
    const e = document.createElement('span');
    e.className = 'ui-eyebrow';
    e.textContent = o.eyebrow;
    head.appendChild(e);
  }
  if (o.title) {
    const t = document.createElement('h3');
    t.className = `ui-title${o.large ? ' ui-title--lg' : ''}`;
    t.textContent = o.title;
    head.appendChild(t);
  }
  if (o.subtitle) {
    const s = document.createElement('p');
    s.className = 'ui-subtitle';
    s.textContent = o.subtitle;
    head.appendChild(s);
  }
  el.appendChild(head);
  const actions = [].concat(o.actions || []).filter(Boolean);
  if (actions.length) {
    const side = document.createElement('div');
    side.className = 'ui-row';
    side.style.gap = '8px';
    side.append(...actions);
    el.appendChild(side);
  }
  return el;
}

export function divider() {
  const el = document.createElement('hr');
  el.className = 'ui-divider';
  return el;
}

/** Vertical stack with even gap. */
export function stack(children = [], { gap = 12 } = {}) {
  const el = document.createElement('div');
  el.className = 'ui-stack';
  el.style.gap = `${gap}px`;
  for (const node of [].concat(children)) {
    if (node == null) continue;
    el.appendChild(typeof node === 'string' ? text(node) : node);
  }
  return el;
}

/** Horizontal row. */
export function row(children = [], { gap = 10, className = '' } = {}) {
  const el = document.createElement('div');
  el.className = `ui-row${className ? ` ${className}` : ''}`;
  el.style.gap = `${gap}px`;
  for (const node of [].concat(children)) {
    if (node == null) continue;
    el.appendChild(typeof node === 'string' ? text(node) : node);
  }
  return el;
}
