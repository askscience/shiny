import { controlButton } from '../ui/components/button.js';
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
import { button, emptyState, icon, modal, setIcon, toast, placeMenu } from '../ui/index.js';
import { notifyMenuChange } from './menuState.js';
import { t } from './i18n.js';

const chipEl = document.getElementById('hud-power');
const iconEl = document.getElementById('hud-power-icon');

/**
 * The three actions, in menu order. `id` is both the menu key and the last
 * segment of the endpoint (`/api/power/<id>`); `allows` reads the status flag.
 * Labels are catalogue keys, resolved at render time — the table is built at
 * import, before the catalogs exist.
 */
const ACTIONS = [
  {
    id: 'reboot',
    labelKey: 'power.restart',
    detailKey: 'power.restartDetail',
    failKey: 'power.restartFailed',
    icon: 'ui/refresh',
    allows: (s) => s.can_reboot,
    confirm: {
      titleKey: 'power.restartConfirmTitle',
      bodyKey: 'power.restartConfirmBody',
      actionKey: 'power.restart',
    },
  },
  {
    id: 'off',
    labelKey: 'power.powerOff',
    detailKey: 'power.powerOffDetail',
    failKey: 'power.powerOffFailed',
    icon: 'ui/power',
    allows: (s) => s.can_power_off,
    danger: true,
    confirm: {
      titleKey: 'power.powerOffConfirmTitle',
      bodyKey: 'power.powerOffConfirmBody',
      actionKey: 'power.powerOff',
    },
  },
  {
    id: 'suspend',
    labelKey: 'power.suspend',
    detailKey: 'power.suspendDetail',
    failKey: 'power.suspendFailed',
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
  placeMenu(popup, trigger, { align: 'right', minHeight: 180 });
}

/* ── Skeleton ──────────────────────────────────────────────── */

function ensurePopup() {
  if (popup) return;
  popup = document.createElement('div');
  popup.className = 'ui-hud-menu-popup power-menu hidden';
  popup.setAttribute('role', 'menu');
  popup.setAttribute('aria-label', t('hud.power'));

  const head = document.createElement('div');
  head.className = 'power-menu-head';
  head.append(icon('ui/power', { size: 14 }), el('span', 'power-menu-title', t('hud.power')));

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
    status = { available: false, reason: t('power.readFailed') };
  }
  render();
}

async function run(action) {
  busy = action.id;
  try {
    await apiFetch(`/api/power/${action.id}`, { method: 'POST', body: '{}' });
    // A reboot or shutdown tears the session down; the toast is best-effort.
    toast(t('power.requested', { action: t(action.labelKey) }));
  } catch (err) {
    toast(err.message || t(action.failKey), { type: 'error' });
  } finally {
    busy = null;
  }
}

/* ── Rendering ─────────────────────────────────────────────── */

function render() {
  if (!body) return;
  body.textContent = '';

  if (!status) {
    body.appendChild(emptyState({ title: t('power.reading') }));
    return;
  }
  if (!status.available) {
    body.appendChild(emptyState({
      icon: 'ui/warning',
      title: t('power.unavailable'),
      body: status.reason || t('power.logindUnreachable'),
    }));
    return;
  }

  for (const action of ACTIONS) body.appendChild(actionRow(action));
  requestAnimationFrame(() => reposition());
}

function actionRow(action) {
  const allowed = action.allows(status);
  const disabled = busy !== null || !allowed;

  const row = controlButton();
  row.type = 'button';
  row.className = 'ui-hud-menu-item';
  row.disabled = disabled;
  row.title = allowed ? t(action.detailKey) : t('power.notAvailable');
  if (!allowed) row.setAttribute('aria-disabled', 'true');
  row.append(icon(action.icon, { size: 16 }), el('span', 'ui-hud-menu-item-label', t(action.labelKey)));
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
    title: t(action.confirm.titleKey),
    body: [el('p', 'ui-subtitle', t(action.confirm.bodyKey))],
  });
  const cancel = button({ label: t('common.cancel'), variant: 'quiet', onClick: () => dialog.close() });
  const confirm = button({
    label: t(action.confirm.actionKey),
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
