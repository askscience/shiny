/** Shared arrangement primitives. Content, translated labels and actions stay with the caller. */
import { compactToolbar } from '../compact.js';

/**
 * A row of tools. With `compact: true` the row folds like a window bar in a
 * narrow window or on a phone (ui/compact.js): icon actions drop their text
 * and what does not fit moves into one "More" menu. `fold` names the
 * foldable tools when they are not `.ui-btn`.
 */
export function toolbar(children = [], { className = '', label, compact = false, fold } = {}) {
  const node = document.createElement('div');
  node.className = `ui-toolbar${className ? ` ${className}` : ''}`;
  if (label) { node.setAttribute('role', 'group'); node.setAttribute('aria-label', label); }
  node.append(...children.filter(Boolean));
  if (compact) compactToolbar(node, fold ? { fold } : {});
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
