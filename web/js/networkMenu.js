/**
 * networkMenu.js — the Network menu: a popover anchored to the top-bar chip
 * with the host's Wi-Fi state, the visible networks and the wired links.
 *
 * Content is `/api/network/status` (the server caches the NetworkManager
 * snapshot), re-read by a slow poll while the popover is open. Actions POST to
 * `/api/network/*` — loopback-only server-side, which is exactly this case.
 * Signal strength arrives pre-bucketed (`level` 0–4) so the icons here and in
 * the chip can never disagree: `hud/wifi-<level>`.
 */

import { apiFetch } from './api.js';
import { notifyMenuChange } from './menuState.js';
import { t } from './i18n.js';
import {
  badge, button, emptyState, icon, iconButton, input, listItem, modal,
  spinner, toast, toggle,
} from '../ui/index.js';

const POLL_MS = 3000;

let popup = null;
let body = null;
let wifiToggle = null;
let wifiHint = null;
let pollTimer = null;
let status = null;
let open = false;
let trigger = null;
/** SSID of the in-flight connect, so its row can show progress. */
let connecting = null;

/* ── Open / close ──────────────────────────────────────────── */

export function isNetworkMenuOpen() {
  return open;
}

export function openNetworkMenu(chip) {
  trigger = chip || trigger;
  ensurePopup();
  open = true;
  // The popup element is reused between opens (so its scroll position and
  // listeners survive), and only `ensurePopup` runs once — unhide it here,
  // otherwise the second open would render into a hidden element.
  popup.classList.remove('hidden');
  // Restart the entry animation: it played on the first open and a reused
  // element would otherwise just pop in.
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

export function closeNetworkMenu() {
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

export function toggleNetworkMenu(chip) {
  if (open) closeNetworkMenu();
  else openNetworkMenu(chip);
}

function onOutside(event) {
  // The password dialog lives in document.body, not in the popover: a click
  // inside it must not dismiss the menu underneath.
  if (event.target.closest?.('.ui-modal')) return;
  if (popup?.contains(event.target)) return;
  if (trigger?.contains(event.target)) return;
  closeNetworkMenu();
}

function onKey(event) {
  if (event.key === 'Escape' && !document.querySelector('.ui-modal:not(.hidden)')) {
    closeNetworkMenu();
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
  popup.className = 'ui-hud-menu-popup network-menu hidden';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', t('hud.network'));

  const head = document.createElement('div');
  head.className = 'network-head';

  const meta = document.createElement('div');
  meta.className = 'network-head-main';
  const title = document.createElement('div');
  title.className = 'network-head-title';
  title.appendChild(icon('ui/network', { size: 15 }));
  const label = document.createElement('span');
  label.textContent = t('net.wifi');
  title.appendChild(label);
  wifiHint = document.createElement('div');
  wifiHint.className = 'network-head-hint';
  meta.append(title, wifiHint);

  wifiToggle = toggle({ checked: false, onChange: (next) => void setWifiPower(next) });

  let scanBtn;
  scanBtn = iconButton({
    icon: 'ui/refresh',
    label: t('net.scan'),
    variant: 'quiet',
    size: 'sm',
    onClick: () => void scan(scanBtn),
  });

  head.append(meta, wifiToggle, scanBtn);

  body = document.createElement('div');
  body.className = 'network-body';

  popup.append(head, body);
  document.body.appendChild(popup);
}

/* ── Data ──────────────────────────────────────────────────── */

async function refresh() {
  if (!open) return;
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

async function scan(buttonEl) {
  buttonEl?.setLoading?.(true);
  try {
    await apiFetch('/api/network/scan', { method: 'POST', body: '{}' });
    // NetworkManager scans asynchronously; give it a moment, then re-read.
    setTimeout(() => { void refresh(); buttonEl?.setLoading?.(false); }, 2500);
  } catch (err) {
    buttonEl?.setLoading?.(false);
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
    body.appendChild(emptyState({ title: t('net.loading') }));
    return;
  }

  const s = status;
  const wifi = s.wifi || {};

  if (wifiToggle) {
    wifiToggle.setChecked(!!wifi.enabled, { silent: true });
    wifiToggle.disabled = !s.available || !wifi.present || wifi.hardware_enabled === false
      || connecting !== null;
  }
  if (wifiHint) {
    wifiHint.textContent = !s.available
      ? (s.reason || t('net.nmUnavailable'))
      : !wifi.present
        ? t('net.noWifiHardware')
        : wifi.hardware_enabled === false
          ? t('net.hardwareSwitch')
          : wifi.enabled
            ? (wifi.interfaces || []).join(', ')
            : t('net.wifiOffHint');
  }

  if (!s.available) {
    body.appendChild(emptyState({
      icon: 'ui/warning',
      title: t('net.unavailable'),
      body: s.reason || t('net.nmUnreachable'),
    }));
    return;
  }

  body.appendChild(connectionSection(s));
  body.appendChild(networksSection(s));

  const saved = (s.saved || []).filter((p) => p.ssid && p.ssid !== s.connection?.ssid);
  if (saved.length) body.appendChild(savedSection(saved, s));

  if ((s.links || []).length) body.appendChild(linksSection(s));

  // Keep the popover height honest: it can grow as networks appear.
  requestAnimationFrame(() => reposition());
}

function sectionTitle(text) {
  const el = document.createElement('div');
  el.className = 'network-section-title';
  el.textContent = text;
  return el;
}

function strengthTrailing(net) {
  const wrap = document.createElement('span');
  wrap.className = 'network-trailing';
  if (connecting === net.ssid) wrap.appendChild(spinner({ size: 13 }));
  if (net.secured) wrap.appendChild(icon('ui/lock', { size: 12, className: 'network-lock' }));
  const pct = document.createElement('span');
  pct.className = 'network-pct';
  pct.textContent = `${net.strength}%`;
  wrap.appendChild(pct);
  return wrap;
}

function connectionSection(s) {
  const link = s.connection;
  const wrap = document.createElement('div');
  wrap.className = 'network-card';
  wrap.appendChild(sectionTitle(t('net.connected')));

  if (!link) {
    const idle = document.createElement('div');
    idle.className = 'network-idle';
    idle.textContent = t('net.notConnected');
    wrap.appendChild(idle);
    return wrap;
  }

  const isWifi = link.kind === 'wifi';
  const subtitleBits = [
    isWifi ? link.security : null,
    link.band,
    link.ip4,
    link.speed_mbps ? `${link.speed_mbps} Mb/s` : null,
    link.managed === false ? t('net.systemManaged') : null,
  ].filter(Boolean);

  const trailing = document.createElement('span');
  trailing.className = 'network-trailing';
  if (isWifi && link.strength != null) {
    const pct = document.createElement('span');
    pct.className = 'network-pct';
    pct.textContent = `${link.strength}%`;
    trailing.appendChild(pct);
  }

  wrap.appendChild(listItem({
    leading: icon(isWifi ? `hud/wifi-${link.level}` : 'hud/ethernet', {
      size: 18,
      className: 'network-leading',
    }),
    title: isWifi ? (link.ssid || t('net.wifi')) : t('net.ethernet'),
    subtitle: subtitleBits.join(' · '),
    trailing,
  }));

  const actions = document.createElement('div');
  actions.className = 'ui-row network-actions';
  if (isWifi) {
    actions.appendChild(button({
      label: t('net.disconnect'),
      variant: 'quiet',
      size: 'sm',
      icon: 'ui/close',
      onClick: () => void disconnect(),
    }));
    const profile = (s.saved || []).find((p) => p.ssid === link.ssid);
    if (profile) {
      actions.appendChild(button({
        label: t('common.forget'),
        variant: 'danger',
        size: 'sm',
        icon: 'ui/trash',
        onClick: () => confirmForget(profile),
      }));
    }
  } else {
    actions.appendChild(badge(t('net.wired'), { tone: 'ok' }));
  }
  wrap.appendChild(actions);
  return wrap;
}

function networksSection(s) {
  const wrap = document.createElement('div');
  const networks = s.networks || [];
  const header = document.createElement('div');
  header.className = 'network-section-head';
  header.appendChild(sectionTitle(t('net.available')));
  if (networks.length) header.appendChild(badge(String(networks.length)));
  wrap.appendChild(header);

  if (!networks.length) {
    const empty = document.createElement('div');
    empty.className = 'network-idle';
    empty.textContent = t('net.noneFound');
    wrap.appendChild(empty);
    return wrap;
  }

  const listEl = document.createElement('div');
  listEl.className = 'ui-list';
  for (const net of networks) {
    const bits = [net.security || t('net.open'), net.band].filter(Boolean);
    if (net.active) bits.unshift(t('net.connected'));
    else if (net.known) bits.unshift(t('net.known'));
    if (net.hidden || !net.ssid) bits.unshift(t('net.hidden'));

    listEl.appendChild(listItem({
      leading: icon(`hud/wifi-${net.level}`, { size: 17, className: 'network-leading' }),
      title: net.ssid || t('net.hiddenNetwork'),
      subtitle: bits.join(' · '),
      trailing: strengthTrailing(net),
      onClick: () => onNetworkClick(net),
    }));
  }
  wrap.appendChild(listEl);
  return wrap;
}

function savedSection(saved, s) {
  const wrap = document.createElement('div');
  wrap.appendChild(sectionTitle(t('net.saved')));
  const listEl = document.createElement('div');
  listEl.className = 'ui-list';
  for (const profile of saved) {
    const trailing = document.createElement('span');
    trailing.className = 'network-trailing';
    trailing.appendChild(iconButton({
      icon: 'ui/trash',
      label: t('net.forgetName', { name: profile.ssid }),
      variant: 'quiet',
      size: 'sm',
      onClick: () => confirmForget(profile),
    }));
    listEl.appendChild(listItem({
      leading: icon('ui/lock', { size: 15, className: 'network-leading' }),
      title: profile.ssid || profile.id,
      subtitle: profile.autoconnect ? t('net.autoconnect') : t('net.manual'),
      trailing,
      onClick: () => {
        const net = (s.networks || []).find((n) => n.ssid === profile.ssid)
          || { ssid: profile.ssid, interface: profile.interface, secured: true, known: true };
        void connect(net, null);
      },
    }));
  }
  wrap.appendChild(listEl);
  return wrap;
}

/** Wired interfaces + any kernel link NetworkManager does not list. */
function linksSection(s) {
  const wrap = document.createElement('div');
  wrap.appendChild(sectionTitle(t('net.wired')));
  const listEl = document.createElement('div');
  listEl.className = 'ui-list';

  const seen = new Set();
  for (const wired of s.wired || []) {
    seen.add(wired.interface);
    const bits = [
      wired.state,
      wired.ip4,
      wired.speed_mbps ? `${wired.speed_mbps} Mb/s` : null,
      wired.managed === false ? t('net.systemManaged') : null,
    ].filter(Boolean);
    listEl.appendChild(listItem({
      leading: icon('hud/ethernet', { size: 17, className: 'network-leading' }),
      title: wired.interface,
      subtitle: bits.join(' · '),
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
    listEl.appendChild(listItem({
      leading: icon('hud/ethernet', { size: 17, className: 'network-leading' }),
      title: link.interface,
      subtitle: bits.join(' · '),
    }));
  }
  wrap.appendChild(listEl);
  return wrap;
}
