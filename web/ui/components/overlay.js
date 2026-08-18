/** overlay — modal, sheet, tooltip. Focus stays simple: Esc + backdrop close. */

import { iconButton } from './button.js';

function wireDismiss(root, close) {
  root.addEventListener('click', (e) => { if (e.target === root) close(); });
  root.addEventListener('keydown', (e) => { if (e.key === 'Escape') close(); });
}

/**
 * Modal dialog. Returns { el, open(), close(), isOpen(), card }.
 * The element is appended to document.body on first open.
 */
export function modal(o = {}) {
  const el = document.createElement('div');
  el.className = 'ui-modal hidden';
  el.setAttribute('role', 'dialog');
  el.setAttribute('aria-modal', 'true');

  const card = document.createElement('div');
  card.className = `ui-modal-card glass-blur${o.wide ? ' ui-modal-card--wide' : ''}`;
  card.tabIndex = -1;

  if (o.title) {
    const head = document.createElement('div');
    head.className = 'ui-row';
    head.style.justifyContent = 'space-between';
    const h = document.createElement('h2');
    h.className = 'ui-title ui-title--lg';
    h.textContent = o.title;
    head.appendChild(h);
    if (o.closable !== false) {
      head.appendChild(iconButton({
        icon: 'ui/close', label: 'Close', variant: 'quiet', size: 'sm',
        onClick: () => api.close(),
      }));
    }
    card.appendChild(head);
  }

  const body = document.createElement('div');
  body.className = 'ui-stack';
  body.style.gap = '20px';
  for (const node of [].concat(o.body || [])) {
    if (node) body.appendChild(node);
  }
  card.appendChild(body);
  el.appendChild(card);
  wireDismiss(el, () => api.close());

  const api = {
    el, card, body,
    isOpen: () => el.classList.contains('is-open'),
    open() {
      if (!el.parentElement) document.body.appendChild(el);
      el.classList.remove('hidden');
      requestAnimationFrame(() => requestAnimationFrame(() => el.classList.add('is-open')));
      card.focus({ preventScroll: true });
    },
    close() {
      el.classList.remove('is-open');
      setTimeout(() => el.classList.add('hidden'), 350);
      o.onClose?.();
    },
  };
  return api;
}

/** Bottom sheet (mobile-first surface). Same API as modal. */
export function sheet(o = {}) {
  const el = document.createElement('div');
  el.className = 'ui-sheet hidden';

  const card = document.createElement('div');
  card.className = 'ui-sheet-card glass-blur';
  card.tabIndex = -1;
  const grip = document.createElement('div');
  grip.className = 'ui-sheet-grip';
  card.appendChild(grip);
  for (const node of [].concat(o.body || [])) {
    if (node) card.appendChild(node);
  }
  el.appendChild(card);
  wireDismiss(el, () => api.close());

  const api = {
    el, card,
    isOpen: () => el.classList.contains('is-open'),
    open() {
      if (!el.parentElement) document.body.appendChild(el);
      el.classList.remove('hidden');
      requestAnimationFrame(() => requestAnimationFrame(() => el.classList.add('is-open')));
    },
    close() {
      el.classList.remove('is-open');
      setTimeout(() => el.classList.add('hidden'), 350);
      o.onClose?.();
    },
  };
  return api;
}

/** Attach a CSS tooltip to an element. */
export function tooltip(el, text) {
  el.dataset.tooltip = text;
  return el;
}
