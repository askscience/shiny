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
import { notifyMenuChange } from './menuState.js';
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

const chipEl = document.getElementById('hud-battery');

let popup = null;
let body = null;
let headIcon = null;
let headTitle = null;
let headHint = null;
let status = null;
let open = false;
let trigger = null;
let refreshing = false;

/* ── Open / close ──────────────────────────────────────────── */

export function isBatteryMenuOpen() {
  return open;
}

export function openBatteryMenu(chip) {
  trigger = chip || trigger || chipEl;
  ensurePopup();
  open = true;
  popup.classList.remove('hidden');
  void refresh();
  document.addEventListener('pointerdown', onOutside, true);
  document.addEventListener('keydown', onKey, true);
  window.addEventListener('resize', reposition);
  trigger?.setAttribute('aria-expanded', 'true');
  trigger?.classList.add('is-open');
  render();
  reposition();
  notifyMenuChange();
}

export function closeBatteryMenu() {
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

export function toggleBatteryMenu(chip) {
  if (open) closeBatteryMenu();
  else openBatteryMenu(chip);
}

function onOutside(event) {
  if (event.target.closest?.('.ui-modal')) return;
  if (popup?.contains(event.target)) return;
  if (trigger?.contains(event.target)) return;
  closeBatteryMenu();
}

function onKey(event) {
  if (event.key === 'Escape' && !document.querySelector('.ui-modal:not(.hidden)')) {
    closeBatteryMenu();
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
  popup.className = 'ui-hud-menu-popup battery-menu hidden';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', 'Battery and power');

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
  if (!open || refreshing) return;
  refreshing = true;
  try {
    const res = await apiFetch('/api/battery/status', { authRedirect: false });
    if (res?.data) status = res.data;
  } catch (_) {
    // Server restarting or offline: keep the last snapshot on screen.
  } finally {
    refreshing = false;
    if (open) render();
  }
}

/* ── Rendering ─────────────────────────────────────────────── */

function render() {
  if (!body) return;
  renderHead();
  body.textContent = '';

  if (!status || !status.available || !status.present) {
    body.appendChild(emptyState({
      title: 'No battery on this machine',
      body: 'Power management is only available on a laptop.',
    }));
    return;
  }

  body.appendChild(powerModeSection());
  body.appendChild(switchesSection());
  body.appendChild(summarySection());
  body.appendChild(settingsAction());
  requestAnimationFrame(() => reposition());
}

function renderHead() {
  if (!headTitle || !headIcon) return;
  const available = status && status.available && status.present;
  headTitle.textContent = available
    ? `Battery ${chipPercent(status) || ''}`.trim()
    : 'Battery';
  void setIcon(headIcon, available ? batteryIconName(status) : 'hud/battery-4', { size: 18 });
  const state = available ? chipStateClass(status) : '';
  headIcon.classList.toggle('is-charging', state === 'is-charging');
  headIcon.classList.toggle('is-low', state === 'is-low');

  const bits = available
    ? [chipLabel(status), timeLabel(status), status.ac_online === true ? 'plugged in' : null]
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
  row.setAttribute('aria-label', 'Power mode');
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
  return section('Power mode', [row]);
}

function switchesSection() {
  const auto = toggleRow({
    label: 'Automatic Power Saver',
    hint: 'Low power on battery or below 20%.',
    checked: getAutoPowerSaver(),
    onChange: (on) => { setAutoPowerSaver(on); render(); },
  });

  const lowAi = toggleRow({
    label: 'Low-power AI',
    hint: 'Vosk + Supertonic instead of Whisper / Qwen.',
    checked: getLowPowerAi(),
    onChange: (on) => { setLowPowerAi(on); render(); },
  });

  return section('AI & battery', [auto, lowAi]);
}

function summarySection() {
  const effective = getEffectivePowerMode();
  const saver = isLowPowerAiActive();
  const line = el('div', 'battery-menu-summary');
  line.textContent = effective === 'saver'
    ? (saver
      ? `Power Saver active — ${describeEngines(getEffectiveSttEngine(), getEffectiveTtsEngine())}`
      : 'Power Saver active — AI engines unchanged')
    : `${powerModeLabel(effective)} — full-quality AI`;
  return section(null, [line]);
}

function settingsAction() {
  const actions = el('div', 'battery-menu-actions');
  actions.appendChild(button({
    label: 'Power settings',
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
  if (open) render();
});

window.addEventListener('power:settings', () => {
  if (open) render();
});
