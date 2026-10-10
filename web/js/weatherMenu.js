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
import { notifyMenuChange } from './menuState.js';
import {
  forecastDayLabel, tempText, weatherIconStem, weatherLabel,
} from './weatherShared.js';
import { getWeatherError, getWeatherState, refreshWeatherNow } from './hudLeft.js';

let popup = null;
let trigger = null;
let open = false;

let headIconEl = null;
let tempEl = null;
let labelEl = null;
let metaEl = null;
let daysEl = null;
let emptyEl = null;

export function isWeatherMenuOpen() {
  return open;
}

export function openWeatherMenu(chip) {
  trigger = chip || trigger;
  ensurePopup();
  open = true;
  popup.classList.remove('hidden');
  // Restart the entry animation: the element is reused between opens.
  popup.style.animation = 'none';
  void popup.offsetHeight;
  popup.style.animation = '';
  render();
  if (!getWeatherState()) refreshWeatherNow();
  document.addEventListener('pointerdown', onOutside, true);
  document.addEventListener('keydown', onKey, true);
  window.addEventListener('resize', reposition);
  trigger?.setAttribute('aria-expanded', 'true');
  trigger?.classList.add('is-open');
  reposition();
  notifyMenuChange();
}

export function closeWeatherMenu() {
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

export function toggleWeatherMenu(chip) {
  if (open) closeWeatherMenu();
  else openWeatherMenu(chip);
}

function onOutside(event) {
  if (event.target.closest?.('.ui-modal')) return;
  if (popup?.contains(event.target)) return;
  if (trigger?.contains(event.target)) return;
  closeWeatherMenu();
}

function onKey(event) {
  if (event.key === 'Escape' && !document.querySelector('.ui-modal:not(.hidden)')) {
    closeWeatherMenu();
  }
}

/** The weather chip sits left of the bar, so the popup hangs from its left. */
function reposition() {
  if (!popup || !trigger) return;
  const rect = trigger.getBoundingClientRect();
  const width = popup.offsetWidth;
  const left = Math.max(12, Math.min(rect.left, window.innerWidth - width - 12));
  const top = rect.bottom + 8;
  popup.style.left = `${left}px`;
  popup.style.top = `${top}px`;
  popup.style.maxHeight = `${Math.max(220, window.innerHeight - top - 16)}px`;
}

/* ── Skeleton ──────────────────────────────────────────────── */

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function ensurePopup() {
  if (popup) return;
  popup = document.createElement('div');
  popup.className = 'ui-hud-menu-popup weather-menu hidden';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', 'Weather forecast');

  const head = el('div', 'weather-menu-head');
  headIconEl = el('span', 'weather-menu-head-icon');
  void setIcon(headIconEl, 'insights/weather-cloud', { size: 30 });
  const main = el('div', 'weather-menu-head-main');
  tempEl = el('div', 'weather-menu-temp', '—°');
  labelEl = el('div', 'weather-menu-label', 'Locating…');
  main.append(tempEl, labelEl);
  head.append(headIconEl, main);

  metaEl = el('div', 'weather-menu-meta');

  emptyEl = emptyState({
    icon: 'insights/weather-sun',
    title: 'Waiting for a location fix',
    body: 'The forecast appears once the shell knows where you are.',
  });

  daysEl = el('div', 'weather-menu-days');

  popup.append(head, metaEl, emptyEl, daysEl);
  document.body.appendChild(popup);
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
    if (state) setEmpty('Forecast unavailable', 'No daily forecast came back for this position.');
    else if (error === 'Location unavailable') setEmpty('Location unavailable', 'The shell could not get a position fix for this device.');
    else if (error) setEmpty('Weather unavailable', 'Open-Meteo could not be reached from here.');
    else setEmpty('Waiting for a location fix', 'The forecast appears once the shell knows where you are.');
    return;
  }

  metaEl.textContent = [state.timezone, 'Open-Meteo'].filter(Boolean).join(' · ');

  // Current conditions head the list, drawn from the same response.
  void setIcon(headIconEl, `insights/${weatherIconStem(state.current.code)}`, { size: 30 });
  tempEl.textContent = tempText(state.current.temp);
  labelEl.textContent = weatherLabel(state.current.code);

  const locale = navigator.language || 'en-US';
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

  requestAnimationFrame(() => reposition());
}

export function initWeatherMenu() {
  const chip = document.getElementById('hud-meteo');
  if (!chip) return;
  chip.setAttribute('aria-haspopup', 'dialog');
  chip.setAttribute('aria-expanded', 'false');
  chip.addEventListener('click', () => toggleWeatherMenu(chip));
  window.addEventListener('weather:update', () => {
    if (open) render();
  });
}
