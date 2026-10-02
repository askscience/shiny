/**
 * powerMenu.js — the top-bar power menu: reboot, power off or suspend the
 * machine. A popover anchored to the power button, backed by the server's
 * freedesktop **logind** endpoints under `/api/power/*` (loopback-only).
 *
 * `/api/power/status` reports which of the three actions the current session
 * allows, so an entry logind would refuse is shown disabled rather than failing
 * on click. Reboot and power off ask for confirmation first; suspend does not
 * (it is easily reversed by waking the machine).
 *
 * The popover is appended to <body> (the HUD pill clips overflow), reuses the
 * shared `.ui-hud-menu-popup` glass, and is positioned under its button.
 */

import { apiFetch } from './api.js';
import { button, emptyState, icon, modal, setIcon, toast } from '../ui/index.js';
import { notifyMenuChange } from './menuState.js';

const chipEl = document.getElementById('hud-power');
const iconEl = document.getElementById('hud-power-icon');

/**
 * The three actions, in menu order. `id` is both the menu key and the last
 * segment of the endpoint (`/api/power/<id>`); `allows` reads the status flag.
 */
const ACTIONS = [
  {
    id: 'reboot',
    label: 'Restart',
    detail: 'Reboot the machine',
    icon: 'ui/refresh',
    allows: (s) => s.can_reboot,
    confirm: {
      title: 'Restart this machine?',
      body: 'It will reboot now — save any open work first.',
      action: 'Restart',
    },
  },
  {
    id: 'off',
    label: 'Power off',
    detail: 'Shut the machine down',
    icon: 'ui/power',
    allows: (s) => s.can_power_off,
    danger: true,
    confirm: {
      title: 'Power off this machine?',
      body: 'It will shut down now — save any open work first.',
      action: 'Power off',
    },
  },
  {
    id: 'suspend',
    label: 'Suspend',
    detail: 'Sleep until you wake the machine',
    icon: 'ui/suspend',
    allows: (s) => s.can_suspend,
  },
];

let popup = null;
let body = null;
let status = null;
let open = false;
let trigger = null;
/** id of the action in flight, so the menu can lock while it runs. */
let busy = null;

/* ── Open / close ──────────────────────────────────────────── */

export function isPowerMenuOpen() {
  return open;
}

export function openPowerMenu(chip) {
  trigger = chip || trigger;
  ensurePopup();
  open = true;
  popup.classList.remove('hidden');
  popup.style.animation = 'none';
  void popup.offsetHeight;
  popup.style.animation = '';
  render();
  void refresh();
  document.addEventListener('pointerdown', onOutside, true);
  document.addEventListener('keydown', onKey, true);
  window.addEventListener('resize', reposition);
  trigger?.setAttribute('aria-expanded', 'true');
  trigger?.classList.add('is-open');
  reposition();
  notifyMenuChange();
}

export function closePowerMenu() {
  if (!open) return;
  open = false;
  popup?.classList.add('hidden');
  document.removeEventListener('pointerdown', onOutside, true);
  document.removeEventListener('keydown', onKey, true);
  window.removeEventListener('resize', reposition);
  trigger?.setAttribute('aria-expanded', 'false');
  trigger?.classList.remove('is-open');
  notifyMenuChange();
}

export function togglePowerMenu(chip) {
  if (open) closePowerMenu();
  else openPowerMenu(chip);
}

function onOutside(event) {
  if (event.target.closest?.('.ui-modal')) return;
  if (popup?.contains(event.target)) return;
  if (trigger?.contains(event.target)) return;
  closePowerMenu();
}

function onKey(event) {
  if (event.key === 'Escape' && !document.querySelector('.ui-modal:not(.hidden)')) {
    closePowerMenu();
  }
}

function reposition() {
  if (!popup || !trigger) return;
  const rect = trigger.getBoundingClientRect();
  const width = popup.offsetWidth;
  const left = Math.max(12, Math.min(rect.right - width, window.innerWidth - width - 12));
  const top = rect.bottom + 8;
  popup.style.left = `${left}px`;
  popup.style.top = `${top}px`;
  popup.style.maxHeight = `${Math.max(180, window.innerHeight - top - 16)}px`;
}

/* ── Skeleton ──────────────────────────────────────────────── */

function ensurePopup() {
  if (popup) return;
  popup = document.createElement('div');
  popup.className = 'ui-hud-menu-popup power-menu hidden';
  popup.setAttribute('role', 'menu');
  popup.setAttribute('aria-label', 'Power');

  const head = document.createElement('div');
  head.className = 'power-menu-head';
  head.append(icon('ui/power', { size: 14 }), el('span', 'power-menu-title', 'Power'));

  body = document.createElement('div');
  body.className = 'power-menu-body';

  popup.append(head, body);
  document.body.appendChild(popup);
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/* ── Data ──────────────────────────────────────────────────── */

async function refresh() {
  if (!open) return;
  try {
    const res = await apiFetch('/api/power/status', { authRedirect: false });
    status = res?.data || null;
  } catch (_) {
    status = { available: false, reason: 'Could not read the power options.' };
  }
  render();
}

async function run(action) {
  busy = action.id;
  try {
    await apiFetch(`/api/power/${action.id}`, { method: 'POST', body: '{}' });
    // A reboot or shutdown tears the session down; the toast is best-effort.
    toast(`${action.label} requested`);
  } catch (err) {
    toast(err.message || `Could not ${action.label.toLowerCase()} the machine`, { type: 'error' });
  } finally {
    busy = null;
  }
}

/* ── Rendering ─────────────────────────────────────────────── */

function render() {
  if (!body) return;
  body.textContent = '';

  if (!status) {
    body.appendChild(emptyState({ title: 'Reading power options…' }));
    return;
  }
  if (!status.available) {
    body.appendChild(emptyState({
      icon: 'ui/warning',
      title: 'Power menu unavailable',
      body: status.reason || 'logind is not reachable.',
    }));
    return;
  }

  for (const action of ACTIONS) body.appendChild(actionRow(action));
  requestAnimationFrame(() => reposition());
}

function actionRow(action) {
  const allowed = action.allows(status);
  const disabled = busy !== null || !allowed;

  const row = document.createElement('button');
  row.type = 'button';
  row.className = 'ui-hud-menu-item';
  row.disabled = disabled;
  row.title = allowed ? action.detail : 'Not available on this machine';
  if (!allowed) row.setAttribute('aria-disabled', 'true');
  row.append(icon(action.icon, { size: 16 }), el('span', 'ui-hud-menu-item-label', action.label));
  row.addEventListener('click', () => onAction(action));
  return row;
}

function onAction(action) {
  closePowerMenu();
  if (action.confirm) confirmAction(action);
  else void run(action);
}

function confirmAction(action) {
  const dialog = modal({
    title: action.confirm.title,
    body: [el('p', 'ui-subtitle', action.confirm.body)],
  });
  const cancel = button({ label: 'Cancel', variant: 'quiet', onClick: () => dialog.close() });
  const confirm = button({
    label: action.confirm.action,
    variant: action.danger ? 'danger' : 'primary',
    onClick: () => {
      dialog.close();
      void run(action);
    },
  });
  const row = el('div', 'ui-row power-confirm-actions');
  row.append(cancel, confirm);
  dialog.card.appendChild(row);
  dialog.open();
}

/* ── Boot ──────────────────────────────────────────────────── */

export function initHudPower() {
  if (!chipEl) return;
  if (iconEl) void setIcon(iconEl, 'ui/power', { size: 20 });
  chipEl.addEventListener('click', () => togglePowerMenu(chipEl));
}
