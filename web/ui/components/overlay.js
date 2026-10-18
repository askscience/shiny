/** Overlays share finite motion, focus containment and reversible dismissal. */

import { iconButton } from './button.js';
import { t } from '../../js/i18n.js';
import { transition, disposeMotion } from '../motion.js';
import { createResources } from '../../js/resources.js';

const overlays = [];
const inertBefore = new Map();
let nextTitleId = 0;
function syncBackground() {
  const top = overlays[overlays.length - 1];
  if (top) {
    for (const node of document.body.children) {
      if (!(node instanceof HTMLElement)) continue;
      if (!inertBefore.has(node)) inertBefore.set(node, node.inert);
      node.inert = node !== top;
    }
  } else {
    for (const [node, inert] of inertBefore) node.inert = inert;
    inertBefore.clear();
  }
}

function lifecycle(el, card, o) {
  let opened = false;
  let trigger;
  let resources;
  const focusable = () => [...card.querySelectorAll('button,a[href],input,select,textarea,[tabindex]')]
    .filter(node => !node.disabled && node.tabIndex !== -1 && node.getClientRects().length && !node.closest('[hidden],.hidden'));
  const restoreBackground = () => {
    const index = overlays.indexOf(el);
    if (index !== -1) overlays.splice(index, 1);
    syncBackground();
  };
  const api = {
    el, card,
    isOpen: () => opened,
    open() {
      if (opened) return;
      opened = true;
      trigger = document.activeElement;
      if (!el.parentElement) document.body.appendChild(el);
      el.hidden = false;
      el.classList.remove('hidden');
      el.classList.remove('is-closing');
      el.classList.add('is-open');
      el.removeAttribute('aria-hidden');
      overlays.push(el); syncBackground();
      resources = createResources();
      resources.on(el, 'click', e => { if (e.target === el && o.closable !== false) api.close(); });
      resources.on(el, 'keydown', e => {
        if (e.key === 'Escape' && o.closable !== false) { e.preventDefault(); e.stopPropagation(); api.close(); }
        if (e.key === 'Tab') {
          const nodes = focusable();
          if (!nodes.length) { e.preventDefault(); card.focus(); return; }
          const first = nodes[0], last = nodes[nodes.length - 1];
          if (e.shiftKey && (document.activeElement === first || document.activeElement === card)) { e.preventDefault(); last.focus(); }
          else if (!e.shiftKey && (document.activeElement === last || document.activeElement === card)) { e.preventDefault(); first.focus(); }
        }
      });
      (focusable()[0] || card).focus({ preventScroll: true });
      transition(card, [{ opacity: 0, transform: 'translateY(8px) scale(.985)' }, { opacity: 1, transform: 'none' }], { kind: 'connect' });
    },
    close() {
      if (!opened) return;
      opened = false;
      resources?.dispose(); resources = null;
      restoreBackground();
      el.classList.add('is-closing');
      el.classList.remove('is-open');
      el.setAttribute('aria-hidden', 'true');
      // The exit remains visible, but never intercepts input or accessibility focus.
      transition(card, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateY(6px) scale(.99)' }], { done: () => {
        el.classList.add('hidden'); el.classList.remove('is-closing'); el.hidden = true;
        if (o.removeOnClose) el.remove();
      } });
      if (trigger?.isConnected && !trigger.closest('[inert]')) trigger.focus({ preventScroll: true });
      o.onClose?.();
    },
    destroy() {
      api.close();
      resources?.dispose(); restoreBackground(); disposeMotion(el); el.remove();
    },
  };
  return api;
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
  if (o.label) el.setAttribute('aria-label', o.label);

  const card = document.createElement('div');
  card.className = `ui-modal-card glass-blur${o.wide ? ' ui-modal-card--wide' : ''}`;
  card.tabIndex = -1;

  if (o.title) {
    const head = document.createElement('div');
    head.className = 'ui-row';
    head.style.justifyContent = 'space-between';
    const h = document.createElement('h2');
    h.id = `ui-dialog-title-${++nextTitleId}`;
    if (!o.label) el.setAttribute('aria-labelledby', h.id);
    h.className = 'ui-title ui-title--lg';
    h.textContent = o.title;
    head.appendChild(h);
    if (o.closable !== false) {
      head.appendChild(iconButton({
        icon: 'ui/close', label: t('common.close'), variant: 'quiet', size: 'sm',
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
  const api = { ...lifecycle(el, card, o), body };
  return api;
}

/** Bottom sheet (mobile-first surface). Same API as modal. */
export function sheet(o = {}) {
  const el = document.createElement('div');
  el.className = 'ui-sheet hidden';
  el.setAttribute('role', 'dialog');
  el.setAttribute('aria-modal', 'true');
  if (o.label || o.title) el.setAttribute('aria-label', o.label || o.title);

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
  return lifecycle(el, card, o);
}

/** Attach a CSS tooltip to an element. */
export function tooltip(el, text) {
  el.dataset.tooltip = text;
  return el;
}
