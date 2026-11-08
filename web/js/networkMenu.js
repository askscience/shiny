/**
 * networkMenu.js — the Network menu: a popover anchored to the dock chip
 * with the host's Wi-Fi state, the visible networks and the wired links.
 *
 * Content is `/api/network/status` (the server caches the NetworkManager
 * snapshot), re-read by a slow poll while the popover is open. Actions POST to
 * `/api/network/*` — loopback-only server-side, which is exactly this case.
 * Signal strength arrives pre-bucketed (`level` 0–4) so the icons here and in
 * the chip can never disagree: `hud/wifi-<level>`.
 */

import { apiFetch } from './api.js';
import { createMenu } from './menuFactory.js';
import { t } from './i18n.js';
import { button, emptyState, icon, iconButton, input, modal, setIcon, spinner, toast, toggle } from '../ui/index.js';
import {
  el, menuHero, menuLabel, menuNote, menuPopup, menuRow, menuValue,
} from './hudMenuShared.js';

const POLL_MS = 3000;

let body = null;
let wifiToggle = null;
let wifiHint = null;
let headIcon = null;
let hero = null;
let scanBtn = null;
let scanning = false;
let pollTimer = null;
let status = null;
/** SSID of the in-flight connect, so its row can show progress. */
let connecting = null;

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

export function openNetworkMenu(chip) {
  menu.show(chip);
}

export function closeNetworkMenu() {
  menu.close();
}

export function toggleNetworkMenu(chip) {
  menu.toggle(chip);
}

/* ── Skeleton ──────────────────────────────────────────────── */

function buildPopup() {
  const { popup, body: bodyEl } = menuPopup('network-menu', t('hud.network'));
  body = bodyEl;

  // The hero: the signal as the big value, the network and its details under it.
  hero = menuHero({ iconName: 'hud/wifi-4' });
  headIcon = hero.iconEl;
  wifiHint = hero.hintEl;
  wifiToggle = toggle({ checked: false, onChange: (next) => void setWifiPower(next) });
  wifiToggle.setAttribute('aria-label', t('net.wifi'));
  hero.actions.appendChild(wifiToggle);

  // Scanning refreshes the list it ends the label of.
  scanBtn = iconButton({
    icon: 'ui/refresh',
    label: t('net.scan'),
    variant: 'quiet',
    size: 'sm',
    onClick: () => void scan(),
  });
  scanBtn.classList.add('hud-menu-scan');

  popup.append(hero.hero, body);
  return popup;
}

/* ── Data ──────────────────────────────────────────────────── */

async function refresh() {
  if (!menu.isOpen()) return;
  try {
    const res = await apiFetch('/api/network/status', { authRedirect: false });
    if (res?.data) {
      status = res.data;
      render();
    }
  } catch (_) {
    // Server restarting or offline: keep the last snapshot on screen.
  }
}

function setScanning(on) {
  if (!scanBtn) return;
  scanning = on;
  scanBtn.classList.toggle('is-loading', on);
  scanBtn.disabled = on || !scanAllowed();
}

/** Scanning needs NetworkManager and a Wi-Fi device. */
function scanAllowed() {
  return !!status?.available && !!status?.wifi?.present;
}

async function scan() {
  setScanning(true);
  try {
    await apiFetch('/api/network/scan', { method: 'POST', body: '{}' });
    // NetworkManager scans asynchronously; give it a moment, then re-read.
    setTimeout(() => { void refresh(); setScanning(false); }, 2500);
  } catch (err) {
    setScanning(false);
    toast(err.message || t('net.scanFailed'), { type: 'error' });
  }
}

async function setWifiPower(enabled) {
  wifiToggle.disabled = true;
  try {
    await apiFetch('/api/network/wifi-power', { method: 'POST', body: JSON.stringify({ enabled }) });
    toast(enabled ? t('net.wifiOn') : t('net.wifiOff'));
  } catch (err) {
    toast(err.message || t('net.wifiPowerFailed'), { type: 'error' });
    wifiToggle.setChecked(!enabled, { silent: true });
  } finally {
    await refresh();
  }
}

async function connect(net, password) {
  connecting = net.ssid;
  render();
  try {
    await apiFetch('/api/network/connect', {
      method: 'POST',
      body: JSON.stringify({
        ssid: net.ssid,
        interface: net.interface || undefined,
        password: password || undefined,
      }),
    });
    toast(t('net.connectedTo', { ssid: net.ssid }));
  } catch (err) {
    toast(err.message || t('net.connectFailedTo', { ssid: net.ssid }), { type: 'error' });
  } finally {
    connecting = null;
    await refresh();
  }
}

async function disconnect() {
  try {
    await apiFetch('/api/network/disconnect', { method: 'POST', body: '{}' });
    toast(t('net.disconnected'));
  } catch (err) {
    toast(err.message || t('net.disconnectFailed'), { type: 'error' });
  } finally {
    await refresh();
  }
}

async function forget(profile) {
  try {
    await apiFetch('/api/network/forget', {
      method: 'POST',
      body: JSON.stringify({ uuid: profile.uuid }),
    });
    toast(t('net.forgot', { ssid: profile.ssid || profile.id }));
  } catch (err) {
    toast(err.message || t('net.forgetFailed'), { type: 'error' });
  } finally {
    await refresh();
  }
}

/* ── Interaction ───────────────────────────────────────────── */

function enterprise(net) {
  return /802\.1X|Enterprise/.test(net.security || '');
}

function onNetworkClick(net) {
  if (connecting) return;
  if (enterprise(net)) {
    toast(t('net.enterprise'));
    return;
  }
  if (net.secured && !net.known) {
    askPassword(net);
    return;
  }
  void connect(net, null);
}

/** Small prompt for the PSK of a network we have no profile for. */
function askPassword(net) {
  const field = input({
    type: 'password',
    placeholder: t('common.password'),
    autocomplete: 'off',
    autocapitalize: 'off',
  });
  const error = document.createElement('p');
  error.className = 'ui-subtitle network-error hidden';

  const dialog = modal({ title: t('net.connectTo', { ssid: net.ssid }), body: [field, error] });

  const cancel = button({ label: t('common.cancel'), variant: 'quiet', onClick: () => dialog.close() });
  const join = button({
    label: t('net.connect'),
    icon: 'ui/network',
    onClick: async () => {
      const password = field.value;
      if (!password) {
        error.textContent = t('net.enterPassword');
        error.classList.remove('hidden');
        return;
      }
      dialog.close();
      await connect(net, password);
    },
  });

  const row = document.createElement('div');
  row.className = 'ui-row network-actions';
  row.append(cancel, join);
  dialog.card.appendChild(row);

  field.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') join.click();
  });

  dialog.open();
  setTimeout(() => field.focus(), 60);
}

function confirmForget(profile) {
  const dialog = modal({
    title: t('net.forgetConfirmTitle', { name: profile.ssid || profile.id }),
    body: [t('net.forgetConfirmBody')],
  });
  const cancel = button({ label: t('common.cancel'), variant: 'quiet', onClick: () => dialog.close() });
  const confirm = button({
    label: t('common.forget'),
    variant: 'danger',
    onClick: () => { dialog.close(); void forget(profile); },
  });
  const row = document.createElement('div');
  row.className = 'ui-row network-actions';
  row.append(cancel, confirm);
  dialog.card.appendChild(row);
  dialog.open();
}

/* ── Rendering ─────────────────────────────────────────────── */

function render() {
  if (!body) return;
  body.textContent = '';

  if (!status) {
    hero.setValue(t('net.wifi'), { text: true });
    body.appendChild(menuNote(t('net.loading')));
    return;
  }

  const s = status;
  const wifi = s.wifi || {};

  if (wifiToggle) {
    wifiToggle.setChecked(!!wifi.enabled, { silent: true });
    wifiToggle.disabled = !s.available || !wifi.present || wifi.hardware_enabled === false
      || connecting !== null;
  }
  renderHero(s);
  if (scanBtn) scanBtn.disabled = scanning || !scanAllowed();

  if (!s.available) {
    body.appendChild(emptyState({
      icon: 'ui/warning',
      title: t('net.unavailable'),
      body: s.reason || t('net.nmUnreachable'),
    }));
    return;
  }

  body.appendChild(networksSection(s));

  const saved = (s.saved || []).filter((p) => p.ssid && p.ssid !== s.connection?.ssid);
  if (saved.length) body.appendChild(savedSection(saved, s));

  if ((s.links || []).length) body.appendChild(linksSection(s));

  // Keep the popover height honest: it can grow as networks appear.
  requestAnimationFrame(() => menu.reposition());
}

function section(label, ...children) {
  const wrap = el('div', 'hud-menu-section');
  wrap.appendChild(label);
  for (const child of children) if (child) wrap.appendChild(child);
  return wrap;
}

function strengthTrailing(net) {
  const bits = [];
  if (connecting === net.ssid) bits.push(spinner({ size: 13 }));
  if (net.secured) bits.push(icon('ui/lock', { size: 12, className: 'hud-menu-dim' }));
  bits.push(menuValue(`${net.strength}%`));
  return bits;
}

/**
 * The hero: the signal of the network in use as the big value (Ethernet by
 * name), the signal glyph and the Wi-Fi switch at the right, the SSID and
 * link details or the reason there is no link under it.
 */
function renderHero(s) {
  const wifi = s.wifi || {};
  const link = s.available ? s.connection : null;
  const wifiOn = s.available && wifi.present && wifi.enabled;
  const isWifi = link?.kind === 'wifi';

  // Like the charge and the volume, Wi-Fi shows a number: the signal.
  if (isWifi && link.strength != null) hero.setValue(`${link.strength}%`);
  else if (link) hero.setValue(isWifi ? (link.ssid || t('net.wifi')) : t('net.ethernet'), { text: true });
  else hero.setValue(wifiOn ? t('net.notConnected') : t('net.wifi'), { text: true });
  hero.valueEl.classList.toggle('is-off', !link);

  void setIcon(headIcon, link
    ? (isWifi ? `hud/wifi-${link.level ?? 4}` : 'hud/ethernet')
    : wifiOn
      ? 'hud/wifi-0'
      : wifi.hardware_enabled === false ? 'hud/wifi-blocked' : 'hud/wifi-off', { size: 22 });
  headIcon.classList.toggle('is-on', !!link);

  wifiHint.textContent = !s.available
    ? (s.reason || t('net.nmUnavailable'))
    : link
      ? [
        isWifi ? (link.ssid || t('net.wifi')) : null,
        isWifi ? link.security : null,
        link.band,
        link.ip4,
        link.speed_mbps ? `${link.speed_mbps} Mb/s` : null,
        link.managed === false ? t('net.systemManaged') : null,
      ].filter(Boolean).join(' · ')
      : !wifi.present
        ? t('net.noWifiHardware')
        : wifi.hardware_enabled === false
          ? t('net.hardwareSwitch')
          : wifi.enabled
            ? (wifi.interfaces || []).join(', ')
            : t('net.wifiOffHint');
  wifiHint.title = wifiHint.textContent;
}

/** Disconnect and forget, at the end of the network in use. */
function activeActions(s, net) {
  const bits = [
    iconButton({
      icon: 'ui/close',
      label: t('net.disconnect'),
      variant: 'quiet',
      size: 'sm',
      onClick: (event) => { event.stopPropagation(); void disconnect(); },
    }),
  ];
  const profile = (s.saved || []).find((p) => p.ssid === net.ssid);
  if (profile) {
    bits.push(iconButton({
      icon: 'ui/trash',
      label: t('net.forgetName', { name: net.ssid }),
      variant: 'quiet',
      size: 'sm',
      onClick: (event) => { event.stopPropagation(); confirmForget(profile); },
    }));
  }
  return bits;
}

function networksSection(s) {
  const networks = s.networks || [];
  const label = menuLabel(t('net.available'), networks.length ? menuValue(String(networks.length)) : null, scanBtn);

  if (!networks.length) return section(label, menuNote(t('net.noneFound')));

  const listEl = el('div', 'hud-menu-list');
  for (const net of networks) {
    const bits = [net.security || t('net.open'), net.band].filter(Boolean);
    if (!net.active && net.known) bits.unshift(t('net.known'));
    if (net.hidden || !net.ssid) bits.unshift(t('net.hidden'));

    listEl.appendChild(menuRow({
      iconName: `hud/wifi-${net.level}`,
      title: net.ssid || t('net.hiddenNetwork'),
      sub: bits.join(' · '),
      // The hero already shows the network in use: its row keeps the actions.
      trailing: net.active ? activeActions(s, net) : strengthTrailing(net),
      selected: !!net.active,
      onClick: net.active ? undefined : () => onNetworkClick(net),
    }));
  }
  return section(label, listEl);
}

function savedSection(saved, s) {
  const listEl = el('div', 'hud-menu-list');
  for (const profile of saved) {
    const forgetBtn = iconButton({
      icon: 'ui/trash',
      label: t('net.forgetName', { name: profile.ssid }),
      variant: 'quiet',
      size: 'sm',
      onClick: () => confirmForget(profile),
    });
    listEl.appendChild(menuRow({
      iconName: 'ui/lock',
      title: profile.ssid || profile.id,
      sub: profile.autoconnect ? t('net.autoconnect') : t('net.manual'),
      trailing: [forgetBtn],
      onClick: () => {
        const net = (s.networks || []).find((n) => n.ssid === profile.ssid)
          || { ssid: profile.ssid, interface: profile.interface, secured: true, known: true };
        void connect(net, null);
      },
    }));
  }
  return section(menuLabel(t('net.saved')), listEl);
}

/** Wired interfaces + any kernel link NetworkManager does not list. */
function linksSection(s) {
  const listEl = el('div', 'hud-menu-list');

  const seen = new Set();
  for (const wired of s.wired || []) {
    seen.add(wired.interface);
    const bits = [
      wired.state,
      wired.ip4,
      wired.speed_mbps ? `${wired.speed_mbps} Mb/s` : null,
      wired.managed === false ? t('net.systemManaged') : null,
    ].filter(Boolean);
    listEl.appendChild(menuRow({
      iconName: 'hud/ethernet',
      title: wired.interface,
      sub: bits.join(' · '),
    }));
  }
  for (const link of s.links || []) {
    if (seen.has(link.interface) || link.wireless) continue;
    const bits = [
      link.up ? t('net.up') : t('net.down'),
      link.ip4,
      link.default_route ? t('net.defaultRoute') : null,
      link.speed_mbps ? `${link.speed_mbps} Mb/s` : null,
      t('net.systemManaged'),
    ].filter(Boolean);
    listEl.appendChild(menuRow({
      iconName: 'hud/ethernet',
      title: link.interface,
      sub: bits.join(' · '),
    }));
  }
  return section(menuLabel(t('net.wired')), listEl);
}
