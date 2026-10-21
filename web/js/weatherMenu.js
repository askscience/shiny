/**
 * weatherMenu.js — the weather chip's popover: current conditions and the
 * next five days at the located place.
 *
 * The chip (`hudLeft.js`) owns the Open-Meteo fetch, its 10-minute TTL and the
 * `weather:update` event; this only renders that state and asks for a fresh
 * fix on its first open, in case a boot-time denial left the chip empty. The
 * popup follows the shared HUD-menu pattern (body-appended, outside pointerdown
 * and Escape close it, resize repositions it).
 */

import { emptyState, setIcon } from '../ui/index.js';
import { createMenu } from './menuFactory.js';
import { getLocale, t } from './i18n.js';
import {
  forecastDayLabel, tempText, weatherIconStem, weatherLabel,
} from './weatherShared.js';
import { getWeatherError, getWeatherState, refreshWeatherNow } from './hudLeft.js';

let headIconEl = null;
let tempEl = null;
let labelEl = null;
let metaEl = null;
let daysEl = null;
let emptyEl = null;

const menu = createMenu({
  build: buildPopup,
  onOpen: () => {
    render();
    if (!getWeatherState()) refreshWeatherNow();
  },
});

export function openWeatherMenu(chip) {
  menu.show(chip);
}

export function closeWeatherMenu() {
  menu.close();
}

export function toggleWeatherMenu(chip) {
  menu.toggle(chip);
}

/* ── Skeleton ──────────────────────────────────────────────── */

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function buildPopup() {
  const popup = document.createElement('div');
  popup.className = 'ui-hud-menu-popup weather-menu hidden';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', t('weather.forecastTitle'));

  const head = el('div', 'weather-menu-head');
  headIconEl = el('span', 'weather-menu-head-icon');
  void setIcon(headIconEl, 'insights/weather-cloud', { size: 30 });
  const main = el('div', 'weather-menu-head-main');
  tempEl = el('div', 'weather-menu-temp', '—°');
  labelEl = el('div', 'weather-menu-label', t('weather.locating'));
  main.append(tempEl, labelEl);
  head.append(headIconEl, main);

  metaEl = el('div', 'weather-menu-meta');

  emptyEl = emptyState({
    icon: 'insights/weather-sun',
    title: t('weather.waitingFix'),
    body: t('weather.waitingFixBody'),
  });

  daysEl = el('div', 'weather-menu-days');

  popup.append(head, metaEl, emptyEl, daysEl);
  return popup;
}

/** Swap the empty state's copy (it is built once, shown for two reasons). */
function setEmpty(title, body) {
  const titleEl = emptyEl.querySelector('.ui-empty-title');
  if (titleEl) titleEl.textContent = title;
  const bodyEl = emptyEl.querySelector('.ui-subtitle');
  if (bodyEl) bodyEl.textContent = body;
}

/* ── Rendering ─────────────────────────────────────────────── */

function render() {
  if (!daysEl) return;
  const state = getWeatherState();
  const days = state?.daily || [];

  emptyEl.classList.toggle('hidden', days.length > 0);
  daysEl.textContent = '';

  if (!days.length) {
    const error = getWeatherError();
    if (state) setEmpty(t('weather.unavailable'), t('weather.unavailableBody'));
    else if (error === 'location') setEmpty(t('weather.locationUnavailable'), t('weather.locationUnavailableBody'));
    else if (error) setEmpty(t('weather.offline'), t('weather.offlineBody'));
    else setEmpty(t('weather.waitingFix'), t('weather.waitingFixBody'));
    return;
  }

  metaEl.textContent = [state.timezone, 'Open-Meteo'].filter(Boolean).join(' · ');

  // Current conditions head the list, drawn from the same response.
  void setIcon(headIconEl, `insights/${weatherIconStem(state.current.code)}`, { size: 30 });
  tempEl.textContent = tempText(state.current.temp);
  labelEl.textContent = weatherLabel(state.current.code);

  const locale = getLocale();
  state.daily.forEach((day, index) => {
    const row = el('div', 'weather-day');
    const iconEl = el('span', 'weather-day-icon');
    void setIcon(iconEl, `insights/${weatherIconStem(day.code)}`, { size: 18 });
    const temps = el('span', 'weather-day-temps');
    temps.append(
      el('span', 'weather-day-max', tempText(day.max)),
      el('span', 'weather-day-min', tempText(day.min)),
    );
    row.append(
      el('span', 'weather-day-name', forecastDayLabel(day.date, index, locale)),
      iconEl,
      temps,
    );
    row.title = weatherLabel(day.code);
    daysEl.appendChild(row);
  });

  requestAnimationFrame(() => menu.reposition());
}

export function initWeatherMenu() {
  const chip = document.getElementById('hud-meteo');
  if (!chip) return;
  chip.setAttribute('aria-haspopup', 'dialog');
  chip.setAttribute('aria-expanded', 'false');
  chip.addEventListener('click', () => toggleWeatherMenu(chip));
  window.addEventListener('weather:update', () => {
    if (menu.isOpen()) render();
  });
}
