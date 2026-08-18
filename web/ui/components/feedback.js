/** feedback — toast, spinner, skeleton, progress, empty state, badge. */

import { icon } from '../icon.js';

let toastContainer = null;

function ensureToastContainer() {
  if (toastContainer?.parentElement) return toastContainer;
  toastContainer = document.createElement('div');
  toastContainer.className = 'ui-toast-container';
  toastContainer.setAttribute('aria-live', 'polite');
  document.body.appendChild(toastContainer);
  return toastContainer;
}

/**
 * Show a toast. type: 'info' | 'error' | 'ok'.
 * Also listens globally for `app:toast` CustomEvents (legacy app contract).
 */
export function toast(message, { type = 'info', duration = 4000 } = {}) {
  const el = document.createElement('div');
  el.className = `ui-toast glass-blur ui-toast--${type}`;
  el.textContent = message;
  ensureToastContainer().appendChild(el);
  requestAnimationFrame(() => requestAnimationFrame(() => el.classList.add('is-in')));
  setTimeout(() => {
    el.classList.remove('is-in');
    setTimeout(() => el.remove(), 400);
  }, duration);
  return el;
}

let toastListenerWired = false;
export function wireToastEvents() {
  if (toastListenerWired) return;
  toastListenerWired = true;
  window.addEventListener('app:toast', (e) => {
    toast(e.detail?.message ?? '', { type: e.detail?.type || 'info' });
  });
}

export function spinner({ size = 18 } = {}) {
  const el = document.createElement('span');
  el.className = 'ui-spinner';
  el.style.width = el.style.height = `${size}px`;
  el.setAttribute('aria-label', 'Loading');
  return el;
}

export function skeleton({ lines = 3, height = 12, gap = 10 } = {}) {
  const wrap = document.createElement('div');
  wrap.className = 'ui-stack';
  wrap.style.gap = `${gap}px`;
  for (let i = 0; i < lines; i++) {
    const bar = document.createElement('div');
    bar.className = 'ui-skeleton';
    bar.style.height = `${height}px`;
    bar.style.width = i === lines - 1 ? '62%' : '100%';
    wrap.appendChild(bar);
  }
  return wrap;
}

export function progress({ value = 0 } = {}) {
  const el = document.createElement('div');
  el.className = 'ui-progress';
  const bar = document.createElement('div');
  bar.className = 'ui-progress-bar';
  el.appendChild(bar);
  el.set = (v) => { bar.style.width = `${Math.max(0, Math.min(100, v))}%`; };
  el.set(value);
  return el;
}

export function emptyState(o = {}) {
  const el = document.createElement('div');
  el.className = 'ui-empty';
  if (o.icon) el.appendChild(icon(o.icon, { size: 28 }));
  const title = document.createElement('p');
  title.className = 'ui-title';
  title.style.fontSize = '16px';
  title.textContent = o.title || 'Nothing here yet';
  el.appendChild(title);
  if (o.body) {
    const body = document.createElement('p');
    body.className = 'ui-subtitle';
    body.textContent = o.body;
    el.appendChild(body);
  }
  if (o.action) el.appendChild(o.action);
  return el;
}

/** Small status pill. tone: 'neutral' | 'accent' | 'ok' | 'warn' | 'error' */
export function badge(text, { tone = 'neutral' } = {}) {
  const el = document.createElement('span');
  el.className = `ui-badge ui-badge--${tone}`;
  el.textContent = text;
  return el;
}
