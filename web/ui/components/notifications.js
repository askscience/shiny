/**
 * notifications — GNOME-style desktop notifications.
 *
 * A single unified banner renderer used by the core and every plugin. Banners
 * stack top-center just below the HUD, show an app/plugin icon, a title
 * (summary), a body, optional action buttons, and dismiss on click or after
 * their urgency-based timeout. Critical notifications stay until dismissed.
 *
 * Two entry points:
 *   - `notify({ title, body, urgency, icon, app, actions, timeout })`
 *   - the global `app:notify` CustomEvent (detail = the same options object)
 *
 * `icon` may be a theme icon path (e.g. `"ui/mail"`) or an already-built
 * element (the app layer passes a plugin icon element in).
 */

import { icon } from '../icon.js';

const URGENCY_TIMEOUT = { low: 4000, normal: 7000, critical: 0 };

let container = null;
let seq = 0;

function ensureContainer() {
  if (container?.parentElement) return container;
  container = document.createElement('div');
  container.className = 'ui-notification-container';
  container.setAttribute('aria-live', 'assertive');
  container.setAttribute('aria-label', 'Notifications');
  document.body.appendChild(container);
  return container;
}

function dismiss(el) {
  if (!el || el.classList.contains('is-leaving')) return;
  el.classList.add('is-leaving');
  setTimeout(() => el.remove(), 260);
}

function buildIcon(iconArg) {
  const wrap = document.createElement('span');
  wrap.className = 'ui-notification-icon';
  if (iconArg instanceof Node) {
    wrap.appendChild(iconArg);
  } else if (typeof iconArg === 'string' && iconArg) {
    wrap.appendChild(icon(iconArg, { size: 18 }));
  } else {
    wrap.classList.add('is-empty');
  }
  return wrap;
}

/**
 * Show a notification banner.
 * @param {object} opts
 * @param {string|null} opts.title     summary headline
 * @param {string}      opts.body      main text
 * @param {string}      opts.urgency   'low' | 'normal' | 'critical'
 * @param {string|Node} opts.icon      theme icon path or an icon element
 * @param {string|null} opts.app       app label shown above the title
 * @param {Array}       opts.actions   [{ label, action }]
 * @param {number|null} opts.timeout   ms before auto-dismiss (0 = sticky)
 */
export function notify(opts = {}) {
  const {
    title = null,
    body = '',
    urgency = 'normal',
    icon: iconArg = null,
    app = null,
    actions = [],
    timeout = null,
  } = opts;

  const level = ['low', 'normal', 'critical'].includes(urgency) ? urgency : 'normal';
  const el = document.createElement('article');
  el.className = `ui-notification ui-notification--${level}`;
  el.dataset.id = `notif-${Date.now()}-${++seq}`;

  if (iconArg) el.appendChild(buildIcon(iconArg));

  const text = document.createElement('div');
  text.className = 'ui-notification-text';
  if (app) {
    const appEl = document.createElement('span');
    appEl.className = 'ui-notification-app';
    appEl.textContent = app;
    text.appendChild(appEl);
  }
  if (title) {
    const t = document.createElement('p');
    t.className = 'ui-notification-title';
    t.textContent = title;
    text.appendChild(t);
  }
  if (body) {
    const b = document.createElement('p');
    b.className = 'ui-notification-body';
    b.textContent = body;
    text.appendChild(b);
  }
  el.appendChild(text);

  if (actions?.length) {
    const row = document.createElement('div');
    row.className = 'ui-notification-actions';
    for (const a of actions) {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'ui-notification-action';
      btn.textContent = a.label || a.action;
      btn.addEventListener('click', (e) => {
        e.stopPropagation();
        dismiss(el);
        window.dispatchEvent(new CustomEvent('app:notification-action', {
          detail: { id: el.dataset.id, action: a.action, notification: opts },
        }));
      });
      row.appendChild(btn);
    }
    el.appendChild(row);
  }

  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'ui-notification-close';
  close.setAttribute('aria-label', 'Dismiss notification');
  close.textContent = '\u00d7';
  close.addEventListener('click', (e) => {
    e.stopPropagation();
    dismiss(el);
  });
  el.appendChild(close);

  el.addEventListener('click', (e) => {
    if (e.target.closest('button')) return;
    dismiss(el);
  });

  ensureContainer().appendChild(el);
  requestAnimationFrame(() => requestAnimationFrame(() => el.classList.add('is-in')));

  const ttl = timeout != null ? Number(timeout) : URGENCY_TIMEOUT[level];
  if (ttl > 0) setTimeout(() => dismiss(el), ttl);

  return el;
}

let wired = false;
/** Route the `app:notify` / `app:notification` events into `notify()`. */
export function wireNotificationEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('app:notify', (e) => notify(e.detail || {}));
  window.addEventListener('app:notification', (e) => notify(e.detail || {}));
}
