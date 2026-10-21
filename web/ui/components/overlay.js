/** Overlays share finite motion, focus containment and reversible dismissal. */

import { iconButton, controlButton } from './button.js';
import { icon } from '../icon.js';
import { t } from '../../js/i18n.js';
import { transition, disposeMotion } from '../motion.js';
import { createResources } from '../../js/resources.js';

const overlays = [];
const inertBefore = new Map();
let nextTitleId = 0;
/** Last dragged corner per key, so a floating panel reopens where it was left. */
const floatingPositions = new Map();
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

/**
 * Small floating window: the desktop's own chrome (a slim title bar with a close
 * control, draggable) on a compact surface, with no backdrop and **no tiling** —
 * it sits above the desktop as a DOM surface, not as a plugin window. Use it for
 * transient, compact things (a picker, a quick form) that should not take over
 * the screen.
 *
 * Returns `{ el, body, open(), close(), isOpen(), destroy() }`. `key` remembers
 * where the user dragged it; Escape and a click outside close it.
 */
export function floating(o = {}) {
  const el = document.createElement('div');
  el.className = `ui-floating${o.wide ? ' ui-floating--wide' : ''} hidden`;
  el.setAttribute('role', 'dialog');
  if (o.label) el.setAttribute('aria-label', o.label);
  el.tabIndex = -1;

  // The desktop's own window chrome: slim title bar, only a close control.
  const head = document.createElement('header');
  head.className = 'tile-header ui-floating-head';
  const controls = document.createElement('span');
  controls.className = 'tile-header-controls';
  const closeBtn = controlButton();
  closeBtn.type = 'button';
  closeBtn.className = 'tile-header-btn tile-header-btn--close';
  closeBtn.title = t('common.close');
  closeBtn.setAttribute('aria-label', t('common.close'));
  closeBtn.appendChild(icon('ui/close', { size: 14 }));
  closeBtn.addEventListener('click', (event) => {
    event.stopPropagation();
    api.close();
  });
  controls.appendChild(closeBtn);
  const title = document.createElement('span');
  title.className = 'tile-header-title';
  title.textContent = o.title || '';
  const spacer = document.createElement('span');
  spacer.className = 'tile-header-spacer ui-floating-spacer';
  head.append(controls, title, spacer);

  const body = document.createElement('div');
  body.className = 'ui-floating-body';
  for (const node of [].concat(o.body || [])) {
    if (node) body.appendChild(node);
  }
  el.append(head, body);

  let opened = false;
  let trigger = null;
  let cleanup = null;

  const clamp = (x, y) => {
    const width = el.offsetWidth || 380;
    const height = el.offsetHeight || 420;
    const maxX = Math.max(8, (window.innerWidth || 1024) - width - 8);
    const maxY = Math.max(8, (window.innerHeight || 768) - height - 8);
    return {
      x: Math.min(Math.max(8, x), maxX),
      y: Math.min(Math.max(8, y), maxY),
    };
  };

  const place = () => {
    const saved = o.key ? floatingPositions.get(o.key) : null;
    const width = el.offsetWidth || 380;
    const height = el.offsetHeight || 420;
    const x = saved?.x ?? Math.round(((window.innerWidth || 1024) - width) / 2);
    const y = saved?.y ?? Math.round(((window.innerHeight || 768) - height) / 3);
    const at = clamp(x, y);
    el.style.left = `${at.x}px`;
    el.style.top = `${at.y}px`;
  };

  // Drag by the head; the buttons in it keep their own clicks.
  head.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || event.target.closest?.('button')) return;
    const rect = el.getBoundingClientRect();
    const dx = event.clientX - rect.left;
    const dy = event.clientY - rect.top;
    const move = (moveEvent) => {
      const at = clamp(moveEvent.clientX - dx, moveEvent.clientY - dy);
      el.style.left = `${at.x}px`;
      el.style.top = `${at.y}px`;
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      if (o.key) {
        floatingPositions.set(o.key, {
          x: parseFloat(el.style.left) || 0,
          y: parseFloat(el.style.top) || 0,
        });
      }
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    event.preventDefault();
  });

  const api = {
    el,
    body,
    isOpen: () => opened,
    open() {
      if (opened) return;
      opened = true;
      trigger = document.activeElement;
      if (!el.parentElement) document.body.appendChild(el);
      el.hidden = false;
      el.classList.remove('hidden');
      place();
      const onDown = (event) => { if (!el.contains(event.target)) api.close(); };
      const onKey = (event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          api.close();
        }
      };
      document.addEventListener('pointerdown', onDown, true);
      document.addEventListener('keydown', onKey, true);
      cleanup = () => {
        document.removeEventListener('pointerdown', onDown, true);
        document.removeEventListener('keydown', onKey, true);
      };
      el.focus({ preventScroll: true });
    },
    close() {
      if (!opened) return;
      opened = false;
      el.classList.add('hidden');
      el.hidden = true;
      cleanup?.();
      cleanup = null;
      if (trigger?.isConnected) trigger.focus({ preventScroll: true });
      if (o.removeOnClose) el.remove();
      o.onClose?.();
    },
    destroy() {
      api.close();
      el.remove();
    },
  };
  return api;
}

/** Attach a CSS tooltip to an element. */
export function tooltip(el, text) {
  el.dataset.tooltip = text;
  return el;
}
