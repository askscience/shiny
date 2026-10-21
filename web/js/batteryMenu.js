/**
 * batteryMenu.js — the top-bar battery quick menu: a reduced power-management
 * panel. It shows the live charge and lets you pick a power mode and the two
 * power switches without opening Settings. The full controls live in
 * Settings → Power (`settingsWindow.js` `buildPower`).
 *
 * The popover is appended to <body> (the HUD pill clips overflow), reuses the
 * shared `.ui-hud-menu-popup` glass, and is positioned under the battery chip.
 * It re-renders from `battery:changed` (the chip's stream) and `power:settings`
 * (a value changed here, in Settings, or automatically from the battery).
 */

import { apiFetch } from './api.js';
import { button, emptyState, setIcon, toggleRow } from '../ui/index.js';
import { createMenu } from './menuFactory.js';
import { openCoreWindowSection } from './tiles.js';
import {
  getAutoPowerSaver, getEffectivePowerMode, getEffectiveSttEngine,
  getEffectiveTtsEngine, getLowPowerAi, getPowerMode, isLowPowerAiActive,
  setAutoPowerSaver, setLowPowerAi, setPowerMode,
} from './preferences.js';
import { describeEngines, powerModeHint, powerModeLabel, POWER_MODES } from './powerShared.js';
import {
  batteryIconName, chipLabel, chipPercent, chipStateClass, timeLabel,
} from './batteryShared.js';
import { t } from './i18n.js';

const chipEl = document.getElementById('hud-battery');

let body = null;
let headIcon = null;
let headTitle = null;
let headHint = null;
let status = null;
let refreshing = false;

/* ── Open / close ──────────────────────────────────────────── */

const menu = createMenu({
  build: buildPopup,
  align: 'right',
  restartAnimation: false,
  onOpen: () => {
    void refresh();
    render();
  },
});

export function openBatteryMenu(chip) {
  menu.show(chip || chipEl);
}

export function closeBatteryMenu() {
  menu.close();
}

export function toggleBatteryMenu(chip) {
  menu.toggle(chip || chipEl);
}

/* ── Skeleton ──────────────────────────────────────────────── */

function buildPopup() {
  const popup = document.createElement('div');
  popup.className = 'ui-hud-menu-popup battery-menu hidden';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', t('battery.title'));

  const head = document.createElement('div');
  head.className = 'battery-menu-head';
  headIcon = document.createElement('span');
  headIcon.className = 'battery-menu-head-icon';
  const meta = document.createElement('div');
  meta.className = 'battery-menu-head-main';
  headTitle = document.createElement('div');
  headTitle.className = 'battery-menu-title';
  headHint = document.createElement('div');
  headHint.className = 'battery-menu-hint';
  meta.append(headTitle, headHint);
  head.append(headIcon, meta);

  body = document.createElement('div');
  body.className = 'battery-menu-body';

  popup.append(head, body);
  return popup;
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/* ── Data ──────────────────────────────────────────────────── */

async function refresh() {
  if (!menu.isOpen() || refreshing) return;
  refreshing = true;
  try {
    const res = await apiFetch('/api/battery/status', { authRedirect: false });
    if (res?.data) status = res.data;
  } catch (_) {
    // Server restarting or offline: keep the last snapshot on screen.
  } finally {
    refreshing = false;
    if (menu.isOpen()) render();
  }
}

/* ── Rendering ─────────────────────────────────────────────── */

function render() {
  if (!body) return;
  renderHead();
  body.textContent = '';

  if (!status || !status.available || !status.present) {
    body.appendChild(emptyState({
      title: t('battery.none'),
      body: t('battery.noneBody'),
    }));
    return;
  }

  body.appendChild(powerModeSection());
  body.appendChild(switchesSection());
  body.appendChild(summarySection());
  body.appendChild(settingsAction());
  requestAnimationFrame(() => menu.reposition());
}

function renderHead() {
  if (!headTitle || !headIcon) return;
  const available = status && status.available && status.present;
  headTitle.textContent = available
    ? t('battery.headWithPercent', { percent: chipPercent(status) }).trim()
    : t('battery.label');
  void setIcon(headIcon, available ? batteryIconName(status) : 'hud/battery-4', { size: 18 });
  const state = available ? chipStateClass(status) : '';
  headIcon.classList.toggle('is-charging', state === 'is-charging');
  headIcon.classList.toggle('is-low', state === 'is-low');

  const bits = available
    ? [chipLabel(status), timeLabel(status), status.ac_online === true ? t('battery.pluggedIn') : null]
    : [];
  headHint.textContent = bits.filter(Boolean).join(' · ');
}

function section(title, children) {
  const wrap = el('div', 'battery-menu-section');
  if (title) wrap.appendChild(el('div', 'battery-menu-section-title', title));
  for (const child of children) if (child) wrap.appendChild(child);
  return wrap;
}

function powerModeSection() {
  const row = el('div', 'power-mode-row');
  row.setAttribute('role', 'radiogroup');
  row.setAttribute('aria-label', t('power.mode'));
  for (const mode of POWER_MODES) {
    const active = getPowerMode() === mode;
    const btn = el('button', `power-mode-btn${active ? ' is-active' : ''}`, powerModeLabel(mode));
    btn.type = 'button';
    btn.title = powerModeHint(mode);
    btn.setAttribute('role', 'radio');
    btn.setAttribute('aria-checked', String(active));
    btn.addEventListener('click', () => {
      setPowerMode(mode);
      render();
    });
    row.appendChild(btn);
  }
  return section(t('power.mode'), [row]);
}

function switchesSection() {
  const auto = toggleRow({
    label: t('power.autoSaver'),
    hint: t('power.autoSaverHint'),
    checked: getAutoPowerSaver(),
    onChange: (on) => { setAutoPowerSaver(on); render(); },
  });

  const lowAi = toggleRow({
    label: t('power.lowAi'),
    hint: t('power.lowAiHint'),
    checked: getLowPowerAi(),
    onChange: (on) => { setLowPowerAi(on); render(); },
  });

  return section(t('power.aiSection'), [auto, lowAi]);
}

function summarySection() {
  const effective = getEffectivePowerMode();
  const saver = isLowPowerAiActive();
  const line = el('div', 'battery-menu-summary');
  line.textContent = effective === 'saver'
    ? (saver
      ? t('power.saverWithEngines', { engines: describeEngines(getEffectiveSttEngine(), getEffectiveTtsEngine()) })
      : t('power.saverUnchanged'))
    : t('power.modeFullAi', { mode: powerModeLabel(effective) });
  return section(null, [line]);
}

function settingsAction() {
  const actions = el('div', 'battery-menu-actions');
  actions.appendChild(button({
    label: t('power.settings'),
    icon: 'ui/settings',
    variant: 'quiet',
    size: 'sm',
    onClick: () => {
      closeBatteryMenu();
      void openCoreWindowSection('settings', 'power');
    },
  }));
  return actions;
}

/* ── Live updates ──────────────────────────────────────────── */

window.addEventListener('battery:changed', (event) => {
  status = event.detail || null;
  if (menu.isOpen()) render();
});

window.addEventListener('power:settings', () => {
  if (menu.isOpen()) render();
});
