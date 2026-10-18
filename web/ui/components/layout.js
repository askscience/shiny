/** Shared arrangement primitives. Content, translated labels and actions stay with the caller. */
export function toolbar(children = [], { className = '', label } = {}) {
  const node = document.createElement('div');
  node.className = `ui-toolbar${className ? ` ${className}` : ''}`;
  if (label) { node.setAttribute('role', 'group'); node.setAttribute('aria-label', label); }
  node.append(...children.filter(Boolean));
  return node;
}

export function controlGrid(children = []) {
  const node = document.createElement('div');
  node.className = 'ui-control-grid';
  node.append(...children.filter(Boolean));
  return node;
}

export function controlCard({ children = [], wide = false } = {}) {
  const node = document.createElement('div');
  node.className = `ui-control-card${wide ? ' ui-control-card--wide' : ''}`;
  node.append(...[].concat(children).filter(Boolean));
  return node;
}

export function controlGroup({ heading, children = [] } = {}) {
  const node = document.createElement('section');
  node.className = 'ui-control-group';
  if (heading) node.appendChild(heading);
  const grid = controlGrid(children);
  node.appendChild(grid);
  node.grid = grid;
  return node;
}
