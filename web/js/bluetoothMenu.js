/**
 * bluetoothMenu.js — the Bluetooth menu: a popover anchored to the top-bar
 * chip with the adapter's state, the connected device and the devices BlueZ
 * knows about.
 *
 * Content is `/api/bluetooth/status` (the server caches the BlueZ snapshot),
 * re-read by a slow poll while the popover is open. Actions POST to
 * `/api/bluetooth/*` — loopback-only server-side, which is exactly this case.
 */

import { apiFetch } from './api.js';
import { notifyMenuChange } from './menuState.js';
import {
  badge, button, emptyState, icon, iconButton, listItem, modal, spinner, toast, toggle,
} from '../ui/index.js';
import {
  batteryLabel, deviceIconName, signalLabel,
} from './bluetoothShared.js';

const POLL_MS = 3000;

let popup = null;
let body = null;
let powerToggle = null;
let powerHint = null;
let scanBtn = null;
let pollTimer = null;
let status = null;
let open = false;
let trigger = null;
/** id of the device with an action in flight, so its row can show progress. */
let busy = null;

/* ── Open / close ──────────────────────────────────────────── */

export function isBluetoothMenuOpen() {
  return open;
}

export function openBluetoothMenu(chip) {
  trigger = chip || trigger;
  ensurePopup();
  open = true;
  // The popup element is reused between opens (so its listeners survive), and
  // only `ensurePopup` runs once — unhide it here.
  popup.classList.remove('hidden');
  popup.style.animation = 'none';
  void popup.offsetHeight;
  popup.style.animation = '';
  render();
  void refresh();
  if (!pollTimer) pollTimer = setInterval(() => void refresh(), POLL_MS);
  document.addEventListener('pointerdown', onOutside, true);
  document.addEventListener('keydown', onKey, true);
  window.addEventListener('resize', reposition);
  trigger?.setAttribute('aria-expanded', 'true');
  trigger?.classList.add('is-open');
  reposition();
  notifyMenuChange();
}

export function closeBluetoothMenu() {
  if (!open) return;
  open = false;
  popup?.classList.add('hidden');
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = null;
  document.removeEventListener('pointerdown', onOutside, true);
  document.removeEventListener('keydown', onKey, true);
  window.removeEventListener('resize', reposition);
  trigger?.setAttribute('aria-expanded', 'false');
  trigger?.classList.remove('is-open');
  notifyMenuChange();
}

export function toggleBluetoothMenu(chip) {
  if (open) closeBluetoothMenu();
  else openBluetoothMenu(chip);
}

function onOutside(event) {
  if (event.target.closest?.('.ui-modal')) return;
  if (popup?.contains(event.target)) return;
  if (trigger?.contains(event.target)) return;
  closeBluetoothMenu();
}

function onKey(event) {
  if (event.key === 'Escape' && !document.querySelector('.ui-modal:not(.hidden)')) {
    closeBluetoothMenu();
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
  popup.style.maxHeight = `${Math.max(220, window.innerHeight - top - 16)}px`;
}

/* ── Skeleton ──────────────────────────────────────────────── */

function ensurePopup() {
  if (popup) return;
  popup = document.createElement('div');
  popup.className = 'ui-hud-menu-popup bluetooth-menu hidden';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', 'Bluetooth');

  const head = document.createElement('div');
  head.className = 'bluetooth-head';

  const meta = document.createElement('div');
  meta.className = 'bluetooth-head-main';
  const title = document.createElement('div');
  title.className = 'bluetooth-head-title';
  title.appendChild(icon('hud/bluetooth', { size: 15 }));
  const label = document.createElement('span');
  label.textContent = 'Bluetooth';
  title.appendChild(label);
  powerHint = document.createElement('div');
  powerHint.className = 'bluetooth-head-hint';
  meta.append(title, powerHint);

  powerToggle = toggle({ checked: false, onChange: (next) => void setPower(next) });

  scanBtn = iconButton({
    icon: 'ui/refresh',
    label: 'Scan for devices',
    variant: 'quiet',
    size: 'sm',
    onClick: () => void scan(),
  });

  head.append(meta, powerToggle, scanBtn);

  body = document.createElement('div');
  body.className = 'bluetooth-body';

  popup.append(head, body);
  document.body.appendChild(popup);
}

/* ── Data ──────────────────────────────────────────────────── */

async function refresh() {
  if (!open) return;
  try {
    const res = await apiFetch('/api/bluetooth/status', { authRedirect: false });
    if (res?.data) {
      status = res.data;
      render();
    }
  } catch (_) {
    // Server restarting or offline: keep the last snapshot on screen.
  }
}

async function setPower(enabled) {
  powerToggle.disabled = true;
  try {
    await apiFetch('/api/bluetooth/power', {
      method: 'POST',
      body: JSON.stringify({ enabled }),
    });
    toast(enabled ? 'Bluetooth on' : 'Bluetooth off');
  } catch (err) {
    toast(err.message || 'Could not change Bluetooth', { type: 'error' });
    powerToggle.setChecked(!enabled, { silent: true });
  } finally {
    await refresh();
  }
}

async function scan() {
  scanBtn?.setLoading?.(true);
  try {
    await apiFetch('/api/bluetooth/scan', { method: 'POST', body: '{}' });
    setTimeout(() => { void refresh(); scanBtn?.setLoading?.(false); }, 3000);
  } catch (err) {
    scanBtn?.setLoading?.(false);
    toast(err.message || 'Scan failed', { type: 'error' });
  }
}

async function run(action, id, { ok, fail }) {
  busy = id;
  render();
  try {
    await apiFetch(`/api/bluetooth/${action}`, {
      method: 'POST',
      body: JSON.stringify({ id }),
    });
    toast(ok);
  } catch (err) {
    toast(err.message || fail, { type: 'error' });
  } finally {
    busy = null;
    await refresh();
  }
}

function pair(device) {
  void run('pair', device.id, {
    ok: `Connected to ${device.name}`,
    fail: `Could not pair with ${device.name}`,
  });
}

function connect(device) {
  void run('connect', device.id, {
    ok: `Connected to ${device.name}`,
    fail: `Could not connect to ${device.name}`,
  });
}

function disconnect(device) {
  void run('disconnect', device.id, {
    ok: `Disconnected from ${device.name}`,
    fail: `Could not disconnect ${device.name}`,
  });
}

function confirmForget(device) {
  const dialog = modal({
    title: `Forget ${device.name}?`,
    body: ['The pairing will be removed. You can pair it again later.'],
  });
  const cancel = button({ label: 'Cancel', variant: 'quiet', onClick: () => dialog.close() });
  const confirm = button({
    label: 'Forget',
    variant: 'danger',
    onClick: () => {
      dialog.close();
      void run('forget', device.id, {
        ok: `Forgot ${device.name}`,
        fail: `Could not forget ${device.name}`,
      });
    },
  });
  const row = document.createElement('div');
  row.className = 'ui-row bluetooth-actions';
  row.append(cancel, confirm);
  dialog.card.appendChild(row);
  dialog.open();
}

/* ── Rendering ─────────────────────────────────────────────── */

function render() {
  if (!body) return;
  body.textContent = '';

  const s = status;
  if (powerToggle) {
    powerToggle.setChecked(!!s?.powered, { silent: true });
    powerToggle.disabled = !s?.available || !s?.present || busy !== null;
  }
  if (scanBtn) scanBtn.disabled = !s?.available || !s?.present || !s?.powered;
  if (powerHint) {
    powerHint.textContent = !s
      ? 'Reading Bluetooth state…'
      : !s.available
        ? (s.reason || 'BlueZ is not available')
        : !s.present
          ? 'No Bluetooth hardware'
          : !s.powered
            ? 'Bluetooth is off'
            : s.discovering
              ? 'Searching for devices…'
              : s.adapter || 'Ready';
  }

  if (!s) {
    body.appendChild(emptyState({ title: 'Loading Bluetooth state…' }));
    return;
  }
  if (!s.available) {
    body.appendChild(emptyState({
      icon: 'ui/warning',
      title: 'Bluetooth panel unavailable',
      body: s.reason || 'BlueZ is not reachable.',
    }));
    return;
  }
  if (!s.present) {
    body.appendChild(emptyState({ title: 'No Bluetooth hardware' }));
    return;
  }
  if (!s.powered) {
    body.appendChild(emptyState({
      title: 'Bluetooth is off',
      body: 'Turn it on to pair headphones, keyboards and other devices.',
    }));
    return;
  }

  const devices = s.devices || [];
  const connected = devices.filter((d) => d.connected);
  const paired = devices.filter((d) => d.paired && !d.connected);
  const nearby = devices.filter((d) => !d.paired && !d.connected && !d.blocked);

  body.appendChild(deviceSection('Connected', connected, 'connected'));
  body.appendChild(deviceSection('Paired', paired, 'connect'));
  body.appendChild(deviceSection('Nearby', nearby, 'pair', s.discovering ? 'searching' : null));

  requestAnimationFrame(() => reposition());
}

function sectionTitle(text) {
  const el = document.createElement('div');
  el.className = 'bluetooth-section-title';
  el.textContent = text;
  return el;
}

function deviceSection(title, devices, action, mode = null) {
  const wrap = document.createElement('div');
  wrap.className = 'bluetooth-card';

  const header = document.createElement('div');
  header.className = 'bluetooth-section-head';
  header.appendChild(sectionTitle(title));
  if (devices.length) header.appendChild(badge(String(devices.length)));
  else if (mode === 'searching') header.appendChild(spinner({ size: 13 }));
  wrap.appendChild(header);

  if (!devices.length) {
    const empty = document.createElement('div');
    empty.className = 'bluetooth-idle';
    empty.textContent = mode === 'searching'
      ? 'Searching…'
      : title === 'Connected'
        ? 'No device connected'
        : title === 'Paired'
          ? 'No paired devices yet'
          : 'No devices found — scan again.';
    wrap.appendChild(empty);
    if (mode === 'searching') wrap.classList.add('is-searching');
    return wrap;
  }

  const listEl = document.createElement('div');
  listEl.className = 'ui-list';
  for (const device of devices) {
    const isBusy = busy === device.id;
    const bits = [
      device.connected ? 'Connected' : null,
      device.address,
      signalLabel(device),
    ].filter(Boolean);

    const trailing = document.createElement('span');
    trailing.className = 'bluetooth-trailing';
    if (isBusy) {
      trailing.appendChild(spinner({ size: 13 }));
    } else if (action === 'connected') {
      if (batteryLabel(device)) trailing.appendChild(batteryTag(device));
      trailing.appendChild(iconButton({
        icon: 'ui/close',
        label: `Disconnect ${device.name}`,
        variant: 'quiet',
        size: 'sm',
        onClick: (event) => { event.stopPropagation(); disconnect(device); },
      }));
    } else if (action === 'connect') {
      if (batteryLabel(device)) trailing.appendChild(batteryTag(device));
      trailing.appendChild(iconButton({
        icon: 'ui/trash',
        label: `Forget ${device.name}`,
        variant: 'quiet',
        size: 'sm',
        onClick: (event) => { event.stopPropagation(); confirmForget(device); },
      }));
    }

    listEl.appendChild(listItem({
      leading: icon(deviceIconName(device.kind), { size: 17, className: 'bluetooth-leading' }),
      title: device.name || device.address,
      subtitle: bits.join(' · '),
      trailing,
      onClick: isBusy
        ? undefined
        : action === 'pair'
          ? () => pair(device)
          : action === 'connect'
            ? () => connect(device)
            : undefined,
    }));
  }
  wrap.appendChild(listEl);
  return wrap;
}

function batteryTag(device) {
  const el = document.createElement('span');
  el.className = 'bluetooth-battery';
  el.textContent = batteryLabel(device);
  return el;
}
