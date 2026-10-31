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
import { createMenu } from './menuFactory.js';
import { t } from './i18n.js';
import { button, emptyState, iconButton, modal, setIcon, spinner, toast, toggle } from '../ui/index.js';
import { el, menuFoot, menuHead, menuLabel, menuNote, menuPill, menuPopup, menuRow, menuValue } from './hudMenuShared.js';
import {
  batteryLabel, deviceIconName, signalLabel,
} from './bluetoothShared.js';

const POLL_MS = 3000;

let body = null;
let powerToggle = null;
let powerHint = null;
let scanBtn = null;
let headIcon = null;
let scanning = false;
let pollTimer = null;
let status = null;
/** id of the device with an action in flight, so its row can show progress. */
let busy = null;

/* ── Open / close ──────────────────────────────────────────── */

const menu = createMenu({
  build: buildPopup,
  align: 'right',
  onOpen: () => {
    render();
    void refresh();
    if (!pollTimer) pollTimer = setInterval(() => void refresh(), POLL_MS);
  },
  onClose: () => {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = null;
  },
});

export function openBluetoothMenu(chip) {
  menu.show(chip);
}

export function closeBluetoothMenu() {
  menu.close();
}

export function toggleBluetoothMenu(chip) {
  menu.toggle(chip);
}

/* ── Skeleton ──────────────────────────────────────────────── */

function buildPopup() {
  const { popup, body: bodyEl } = menuPopup('bluetooth-menu', t('hud.bluetooth'));
  body = bodyEl;

  const head = menuHead({ iconName: 'hud/bluetooth', title: t('bluetooth.label') });
  headIcon = head.iconEl;
  powerHint = head.hintEl;
  powerToggle = toggle({ checked: false, onChange: (next) => void setPower(next) });
  powerToggle.setAttribute('aria-label', t('bluetooth.label'));
  head.actions.appendChild(powerToggle);

  scanBtn = menuPill({
    label: t('bt.scan'),
    iconName: 'ui/refresh',
    arrow: false,
    onClick: () => void scan(),
  });
  const { foot } = menuFoot(scanBtn);

  popup.append(head.head, body, foot);
  return popup;
}

/* ── Data ──────────────────────────────────────────────────── */

async function refresh() {
  if (!menu.isOpen()) return;
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
    toast(enabled ? t('bt.on') : t('bluetooth.off'));
  } catch (err) {
    toast(err.message || t('bt.powerFailed'), { type: 'error' });
    powerToggle.setChecked(!enabled, { silent: true });
  } finally {
    await refresh();
  }
}

function scanAllowed() {
  return !!status?.available && !!status?.present && !!status?.powered;
}

function setScanning(on) {
  scanning = on;
  scanBtn?.classList.toggle('is-loading', on);
  if (scanBtn) scanBtn.disabled = on || !scanAllowed();
}

async function scan() {
  setScanning(true);
  try {
    await apiFetch('/api/bluetooth/scan', { method: 'POST', body: '{}' });
    setTimeout(() => { void refresh(); setScanning(false); }, 3000);
  } catch (err) {
    setScanning(false);
    toast(err.message || t('bt.scanFailed'), { type: 'error' });
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
    ok: t('bt.connectedTo', { name: device.name }),
    fail: t('bt.pairFailed', { name: device.name }),
  });
}

function connect(device) {
  void run('connect', device.id, {
    ok: t('bt.connectedTo', { name: device.name }),
    fail: t('bt.connectFailed', { name: device.name }),
  });
}

function disconnect(device) {
  void run('disconnect', device.id, {
    ok: t('bt.disconnectedFrom', { name: device.name }),
    fail: t('bt.disconnectFailed', { name: device.name }),
  });
}

function confirmForget(device) {
  const dialog = modal({
    title: t('bt.forgetConfirmTitle', { name: device.name }),
    body: [t('bt.forgetConfirmBody')],
  });
  const cancel = button({ label: t('common.cancel'), variant: 'quiet', onClick: () => dialog.close() });
  const confirm = button({
    label: t('common.forget'),
    variant: 'danger',
    onClick: () => {
      dialog.close();
      void run('forget', device.id, {
        ok: t('bt.forgot', { name: device.name }),
        fail: t('bt.forgetFailed', { name: device.name }),
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
  if (scanBtn) scanBtn.disabled = scanning || !scanAllowed();
  if (headIcon) {
    void setIcon(headIcon, s?.available && s?.present && s?.powered ? 'hud/bluetooth' : 'hud/bluetooth-off', { size: 18 });
  }
  if (powerHint) {
    powerHint.textContent = !s
      ? t('bt.reading')
      : !s.available
        ? (s.reason || t('bt.bluezUnavailable'))
        : !s.present
          ? t('bt.noHardware')
          : !s.powered
            ? t('bluetooth.off')
            : s.discovering
              ? t('bt.searching')
              : s.adapter || t('bt.ready');
  }

  if (!s) {
    body.appendChild(menuNote(t('bt.loading')));
    return;
  }
  if (!s.available) {
    body.appendChild(emptyState({
      icon: 'ui/warning',
      title: t('bt.unavailable'),
      body: s.reason || t('bt.bluezUnreachable'),
    }));
    return;
  }
  if (!s.present) {
    body.appendChild(menuNote(t('bt.noHardware')));
    return;
  }
  if (!s.powered) {
    body.appendChild(menuNote(t('bt.turnOnBody')));
    return;
  }

  const devices = s.devices || [];
  const connected = devices.filter((d) => d.connected);
  const paired = devices.filter((d) => d.paired && !d.connected);
  const nearby = devices.filter((d) => !d.paired && !d.connected && !d.blocked);

  body.appendChild(deviceSection(t('bt.connected'), connected, 'connected'));
  body.appendChild(deviceSection(t('bt.paired'), paired, 'connect'));
  body.appendChild(deviceSection(t('bt.nearby'), nearby, 'pair', s.discovering ? 'searching' : null));

  requestAnimationFrame(() => menu.reposition());
}

function deviceSection(title, devices, action, mode = null) {
  const wrap = el('div', 'hud-menu-section');
  wrap.appendChild(menuLabel(
    title,
    devices.length
      ? menuValue(String(devices.length))
      : mode === 'searching' ? spinner({ size: 12 }) : null,
  ));

  if (!devices.length) {
    wrap.appendChild(menuNote(mode === 'searching'
      ? t('bt.searching')
      : action === 'connected'
        ? t('bt.noneConnected')
        : action === 'connect'
          ? t('bt.nonePaired')
          : t('bt.noneFound')));
    if (mode === 'searching') wrap.classList.add('is-searching');
    return wrap;
  }

  const listEl = el('div', 'hud-menu-list');
  for (const device of devices) {
    const isBusy = busy === device.id;
    const bits = [
      device.connected ? t('bt.connected') : null,
      device.address,
      signalLabel(device),
    ].filter(Boolean);

    const trailing = [];
    if (isBusy) {
      trailing.push(spinner({ size: 13 }));
    } else if (action === 'connected') {
      if (batteryLabel(device)) trailing.push(menuValue(batteryLabel(device)));
      trailing.push(iconButton({
        icon: 'ui/close',
        label: t('bt.disconnectName', { name: device.name }),
        variant: 'quiet',
        size: 'sm',
        onClick: (event) => { event.stopPropagation(); disconnect(device); },
      }));
    } else if (action === 'connect') {
      if (batteryLabel(device)) trailing.push(menuValue(batteryLabel(device)));
      trailing.push(iconButton({
        icon: 'ui/trash',
        label: t('bt.forgetName', { name: device.name }),
        variant: 'quiet',
        size: 'sm',
        onClick: (event) => { event.stopPropagation(); confirmForget(device); },
      }));
    }

    listEl.appendChild(menuRow({
      iconName: deviceIconName(device.kind),
      title: device.name || device.address,
      sub: bits.join(' · '),
      trailing,
      selected: action === 'connected',
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
