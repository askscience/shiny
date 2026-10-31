/**
 * batteryMenu.js — the top-bar battery quick menu: the full power-management
 * panel. It shows the live charge, the power mode and switches, and the
 * screen-brightness and keyboard-backlight sliders.
 *
 * The popover is appended to <body> (the HUD pill clips overflow), is built
 * from the shared status-menu anatomy (hudMenuShared.js: a big-percentage
 * hero, power-mode rows with a check, slider rows, switch rows) and is
 * positioned against the battery chip.
 * It re-renders from `battery:changed` (the chip's stream) and `power:settings`
 * (a value changed here or automatically from the battery).
 */

import { apiFetch } from './api.js';
import { icon, setIcon, slider, toggle } from '../ui/index.js';
import { el, menuLabel, menuNote, menuPopup, menuRow, menuSlider } from './hudMenuShared.js';
import { createMenu } from './menuFactory.js';
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

const BACKLIGHTS = {
  screen: { endpoint: '/api/screen/brightness' },
  keyboard: { endpoint: '/api/keyboard/backlight' },
};

const backlights = {
  screen: { loaded: false, available: false, writable: false, percent: null },
  keyboard: { loaded: false, available: false, writable: false, percent: null },
};
const backlightWrites = { screen: null, keyboard: null };
let adjustingBacklight = false;

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
    void refreshBacklights();
    render();
  },
});

export function openBatteryMenu(chip) {
  menu.show(chip || chipEl);
}

export function toggleBatteryMenu(chip) {
  menu.toggle(chip || chipEl);
}

/* ── Skeleton ──────────────────────────────────────────────── */

function buildPopup() {
  const { popup, body: bodyEl } = menuPopup('battery-menu', t('battery.title'));
  body = bodyEl;

  // The hero: the charge as a big number, the state on a line under it.
  const hero = el('div', 'hud-menu-hero');
  const top = el('div', 'hud-menu-hero-top');
  headTitle = el('div', 'hud-menu-hero-value');
  headIcon = el('span', 'hud-menu-hero-icon');
  top.append(headTitle, headIcon);
  headHint = el('div', 'hud-menu-hero-hint');
  hero.append(top, headHint);

  popup.append(hero, body);
  return popup;
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

/**
 * Read both host backlights. The sliders write straight to sysfs through the
 * server; a machine without the device reports `available:false`, and one
 * without write permission reports `writable:false` — the control stays
 * disabled either way instead of pretending to work.
 */
async function refreshBacklights() {
  await Promise.all(Object.entries(BACKLIGHTS).map(async ([id, spec]) => {
    const state = backlights[id];
    try {
      const res = await apiFetch(spec.endpoint, { authRedirect: false });
      const data = res?.data;
      state.loaded = true;
      state.available = data?.available === true && data?.percent != null;
      state.writable = data?.writable !== false;
      state.percent = state.available ? data.percent : null;
    } catch (_) {
      state.loaded = true;
      state.available = false;
      state.writable = false;
      state.percent = null;
    }
  }));
  if (menu.isOpen()) render();
}

/* ── Rendering ─────────────────────────────────────────────── */

function render() {
  if (!body) return;
  renderHead();
  body.textContent = '';

  if (!status || !status.available || !status.present) {
    body.appendChild(menuNote(`${t('battery.none')} · ${t('battery.noneBody')}`));
    return;
  }

  body.appendChild(powerModeSection());
  body.appendChild(displaySection());
  body.appendChild(switchesSection());
  body.appendChild(summarySection());
  requestAnimationFrame(() => menu.reposition());
}

function renderHead() {
  if (!headTitle || !headIcon) return;
  const available = status && status.available && status.present;
  headTitle.textContent = available ? chipPercent(status) : t('battery.label');
  headTitle.classList.toggle('is-text', !available);
  void setIcon(headIcon, available ? batteryIconName(status) : 'hud/battery-4', { size: 22 });
  const state = available ? chipStateClass(status) : '';
  headIcon.classList.toggle('is-charging', state === 'is-charging');
  headIcon.classList.toggle('is-low', state === 'is-low');
  headTitle.classList.toggle('is-low', state === 'is-low');

  const bits = available
    ? [chipLabel(status), timeLabel(status), status.ac_online === true ? t('battery.pluggedIn') : null]
    : [];
  // `chipLabel` already says "Plugged in" when idle on AC: name it once.
  headHint.textContent = [...new Set(bits.filter(Boolean))].join(' · ');
}

function section(title, children) {
  const wrap = el('div', 'hud-menu-section');
  if (title) wrap.appendChild(menuLabel(title));
  for (const child of children) if (child) wrap.appendChild(child);
  return wrap;
}

const MODE_ICONS = { performance: 'ui/arrow-up', balanced: 'ui/contrast', saver: 'hud/battery-1' };

function powerModeSection() {
  const group = el('div', 'hud-menu-list');
  group.setAttribute('role', 'radiogroup');
  group.setAttribute('aria-label', t('power.mode'));
  for (const mode of POWER_MODES) {
    const active = getPowerMode() === mode;
    const row = menuRow({
      iconName: MODE_ICONS[mode],
      title: powerModeLabel(mode),
      tooltip: powerModeHint(mode),
      selected: active,
      role: 'radio',
      onClick: () => {
        setPowerMode(mode);
        render();
      },
    });
    row.setAttribute('aria-checked', String(active));
    group.appendChild(row);
  }
  return section(t('power.mode'), [group]);
}

function displaySection() {
  return section(t('common.display', null, 'Display'), [
    ...backlightField('screen'),
    ...backlightField('keyboard'),
  ]);
}

/** A slider row for one backlight, plus a note when it cannot be driven. */
function backlightField(id) {
  const spec = BACKLIGHTS[id];
  const state = backlights[id];

  const title = id === 'screen'
    ? t('settings.power.screenBrightness', null, 'Screen brightness')
    : t('settings.power.keyboardBacklight', null, 'Keyboard backlight');
  const unavailable = id === 'screen'
    ? t('settings.power.screenBacklightUnavailable', null, 'This machine does not expose a writable panel backlight to the app.')
    : t('settings.power.keyboardBacklightUnavailable', null, 'This machine does not expose a writable keyboard backlight to the app.');

  const control = slider({ min: 0, max: 100, value: state.percent ?? 50 });
  control.setAttribute('aria-label', title);
  const lead = icon(id === 'screen' ? 'ui/brightness' : 'ui/keyboard', { size: 17, className: 'hud-menu-icon' });
  lead.title = title;
  const { row, value } = menuSlider({ leading: lead, range: control });
  row.title = title;

  if (!state.loaded) {
    value.textContent = '—';
    control.disabled = true;
    return [row];
  }
  if (!state.available) {
    value.textContent = '—';
    control.disabled = true;
    return [row, menuNote(`${title}: ${t('settings.power.unavailable', null, 'Unavailable')}. ${unavailable}`)];
  }

  value.textContent = `${state.percent}`;
  control.disabled = !state.writable;
  if (!state.writable) {
    return [row, menuNote(t('settings.power.backlightReadOnly', null, 'The app cannot write this backlight on this machine.'))];
  }

  control.addEventListener('pointerdown', () => { adjustingBacklight = true; });
  control.addEventListener('input', () => {
    const percent = Number(control.value);
    state.percent = percent;
    value.textContent = `${percent}`;
    clearTimeout(backlightWrites[id]);
    backlightWrites[id] = setTimeout(() => {
      apiFetch(spec.endpoint, { method: 'POST', body: JSON.stringify({ percent }) }).catch(() => {});
    }, 150);
  });
  return [row];
}

function switchRow({ label, hint, checked, onChange }) {
  const sw = toggle({ checked, onChange });
  sw.setAttribute('aria-label', label);
  return menuRow({ title: label, sub: hint, trailing: [sw] });
}

function switchesSection() {
  const auto = switchRow({
    label: t('power.autoSaver'),
    hint: t('power.autoSaverHint'),
    checked: getAutoPowerSaver(),
    onChange: (on) => { setAutoPowerSaver(on); render(); },
  });

  const lowAi = switchRow({
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
  const line = menuNote(effective === 'saver'
    ? (saver
      ? t('power.saverWithEngines', { engines: describeEngines(getEffectiveSttEngine(), getEffectiveTtsEngine()) })
      : t('power.saverUnchanged'))
    : t('power.modeFullAi', { mode: powerModeLabel(effective) }));
  return section(null, [line]);
}

/* ── Live updates ──────────────────────────────────────────── */

/** A battery push must not rebuild the tree out from under a dragged slider. */
function isAdjustingBacklight() {
  const active = document.activeElement;
  return adjustingBacklight || (!!active && active.type === 'range' && !!body?.contains(active));
}

window.addEventListener('pointerup', () => { adjustingBacklight = false; });
window.addEventListener('pointercancel', () => { adjustingBacklight = false; });

window.addEventListener('battery:changed', (event) => {
  status = event.detail || null;
  if (menu.isOpen() && !isAdjustingBacklight()) render();
});

window.addEventListener('power:settings', () => {
  if (menu.isOpen() && !isAdjustingBacklight()) render();
});
