/**
 * clockMenu.js — the clock's popover: the time at the located place plus a
 * compact month calendar.
 *
 * Days with events are dotted and a day's list shows underneath, read from the
 * calendar plugin's REST API — so it answers "are there events saved?" even
 * with the Calendar window closed. A missing/unbuilt plugin is an explicit
 * empty state, not an error. The popup follows the shared HUD-menu pattern:
 * appended to <body> (the HUD pill clips overflow), outside pointerdown and
 * Escape close it, resize repositions it.
 */

import { apiFetch } from './api.js';
import { button, emptyState, iconButton } from '../ui/index.js';
import { createMenu } from './menuFactory.js';
import { getLocale, t, tp } from './i18n.js';
import {
  buildMonthCells, formatEventTime, formatLocationTime, groupEventsByDate,
  isoDate, isoDayLabel, monthKey, monthLabel, shiftMonth, weekdayShortNames,
} from './clockShared.js';
import { getWeatherState } from './hudLeft.js';
import { openPlugin } from './hudPlugins.js';

const CALENDAR_PLUGIN = 'calendar';

/* Viewed month (month is 0-based) and the selected day ("YYYY-MM-DD"). */
let viewYear = 0;
let viewMonth = 0;
let selectedDate = null;
/** True until a day is picked or a month browsed: the popover then re-anchors
 *  to today on every open, so a kiosk left running for weeks stays current. */
let followToday = true;

let eventsByDate = new Map();
/** 'loading' | 'ready' | 'missing' | 'error' — from the month fetch. */
let monthState = 'loading';
let fetchToken = 0;
let tickTimer = null;

let monthEl = null;
let timeEl = null;
let weekdaysEl = null;
let gridEl = null;
let dayHeadEl = null;
let eventsEl = null;

const menu = createMenu({
  build: buildPopup,
  onOpen: () => {
    const today = new Date();
    if (followToday || !selectedDate) {
      selectedDate = isoDate(today);
      viewYear = today.getFullYear();
      viewMonth = today.getMonth();
    }

    renderClock();
    renderCalendar();
    void loadMonth();

    if (!tickTimer) tickTimer = setInterval(renderClock, 1000);
  },
  onClose: () => {
    if (tickTimer) clearInterval(tickTimer);
    tickTimer = null;
  },
});

export function openClockMenu(chip) {
  menu.show(chip);
}

export function closeClockMenu() {
  menu.close();
}

export function toggleClockMenu(chip) {
  menu.toggle(chip);
}

/* ── Skeleton ──────────────────────────────────────────────── */

function buildPopup() {
  const popup = document.createElement('div');
  popup.className = 'ui-hud-menu-popup clock-menu hidden';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', t('hud.timeCalendar'));

  const head = document.createElement('div');
  head.className = 'clock-menu-head';
  monthEl = document.createElement('div');
  monthEl.className = 'clock-menu-month';
  head.append(
    iconButton({
      icon: 'ui/chevron-left',
      label: t('clock.prevMonth'),
      variant: 'quiet',
      size: 'sm',
      onClick: () => stepMonth(-1),
    }),
    monthEl,
    iconButton({
      icon: 'ui/chevron-right',
      label: t('clock.nextMonth'),
      variant: 'quiet',
      size: 'sm',
      onClick: () => stepMonth(1),
    }),
  );

  timeEl = document.createElement('div');
  timeEl.className = 'clock-menu-time';

  weekdaysEl = document.createElement('div');
  weekdaysEl.className = 'clock-menu-weekdays';
  for (const name of weekdayShortNames(getLocale())) {
    const cell = document.createElement('div');
    cell.className = 'clock-menu-weekday';
    cell.textContent = name;
    weekdaysEl.appendChild(cell);
  }

  gridEl = document.createElement('div');
  gridEl.className = 'clock-menu-grid';

  dayHeadEl = document.createElement('div');
  dayHeadEl.className = 'clock-menu-day-head';
  eventsEl = document.createElement('div');
  eventsEl.className = 'clock-menu-events';

  const detail = document.createElement('div');
  detail.className = 'clock-menu-detail';
  detail.append(dayHeadEl, eventsEl);

  const actions = document.createElement('div');
  actions.className = 'clock-menu-actions';
  actions.appendChild(button({
    label: t('clock.openCalendar'),
    icon: 'ui/calendar',
    variant: 'quiet',
    size: 'sm',
    onClick: () => {
      closeClockMenu();
      void openPlugin(CALENDAR_PLUGIN);
    },
  }));

  popup.append(head, timeEl, weekdaysEl, gridEl, detail, actions);
  return popup;
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/* ── Rendering ─────────────────────────────────────────────── */

/** The live line under the month: wall-clock time at the located place. */
function renderClock() {
  if (!timeEl) return;
  const locale = getLocale();
  const zone = getWeatherState()?.timezone || null;
  const now = formatLocationTime(new Date(), zone, locale, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
  timeEl.textContent = zone ? `${now} · ${zone}` : now;
  timeEl.title = zone ? t('clock.timeAtLocation', { zone }) : t('clock.timeOnMachine');
}

function renderCalendar() {
  if (!gridEl) return;
  const locale = getLocale();
  monthEl.textContent = monthLabel(viewYear, viewMonth, locale);

  const todayIso = isoDate(new Date());
  gridEl.textContent = '';
  for (const cell of buildMonthCells(viewYear, viewMonth)) {
    if (!cell) {
      gridEl.appendChild(el('span', 'clock-menu-pad'));
      continue;
    }
    const dayEvents = eventsByDate.get(cell.iso) || [];
    const day = el('button', 'clock-menu-day');
    day.type = 'button';
    day.textContent = String(cell.day);
    day.classList.toggle('is-today', cell.iso === todayIso);
    day.classList.toggle('is-selected', cell.iso === selectedDate);
    if (dayEvents.length) {
      const dot = el('span', 'clock-menu-dot');
      dot.setAttribute('aria-hidden', 'true');
      day.appendChild(dot);
    }
    const count = dayEvents.length ? tp('clock.eventCount', dayEvents.length) : '';
    day.setAttribute('aria-label', `${isoDayLabel(cell.iso, locale)}${count}`);
    day.addEventListener('click', () => selectDate(cell.iso));
    gridEl.appendChild(day);
  }

  renderDay();
}

function selectDate(iso) {
  selectedDate = iso;
  followToday = false;
  renderCalendar();
}

function renderDay() {
  if (!dayHeadEl || !eventsEl) return;
  dayHeadEl.textContent = selectedDate ? isoDayLabel(selectedDate, getLocale()) : '';
  eventsEl.textContent = '';

  if (monthState === 'loading') {
    eventsEl.appendChild(emptyState({ title: t('clock.loadingEvents') }));
    return;
  }
  if (monthState === 'missing') {
    eventsEl.appendChild(emptyState({
      icon: 'ui/puzzle',
      title: t('clock.pluginMissing'),
      body: t('clock.pluginMissingBody'),
    }));
    return;
  }
  if (monthState === 'error') {
    eventsEl.appendChild(emptyState({ icon: 'ui/warning', title: t('clock.loadEventsFailed') }));
    return;
  }

  const list = selectedDate ? (eventsByDate.get(selectedDate) || []) : [];
  if (!list.length) {
    eventsEl.appendChild(el('div', 'clock-menu-none', t('clock.noEvents')));
    return;
  }

  for (const ev of list) {
    const row = el('div', 'clock-menu-event');
    row.append(
      el('span', 'clock-menu-event-time', formatEventTime(ev)),
      el('span', 'clock-menu-event-title', ev.title || t('clock.untitled')),
    );
    if (ev.location) row.title = ev.location;
    eventsEl.appendChild(row);
  }
}

/* ── Data ──────────────────────────────────────────────────── */

function stepMonth(delta) {
  followToday = false;
  const next = shiftMonth(viewYear, viewMonth, delta);
  viewYear = next.year;
  viewMonth = next.month;
  eventsByDate = new Map();
  monthState = 'loading';
  renderCalendar();
  void loadMonth();
}

async function loadMonth() {
  if (!menu.isOpen()) return;
  const token = ++fetchToken;
  try {
    const res = await apiFetch(
      `/api/calendar/events?month=${encodeURIComponent(monthKey(viewYear, viewMonth))}&limit=1000`,
      { authRedirect: false },
    );
    if (token !== fetchToken || !menu.isOpen()) return;
    eventsByDate = groupEventsByDate(res?.data?.events || []);
    monthState = 'ready';
  } catch (err) {
    if (token !== fetchToken || !menu.isOpen()) return;
    eventsByDate = new Map();
    // No route at all means the plugin is not installed/loaded.
    monthState = err?.status === 404 ? 'missing' : 'error';
  }
  renderCalendar();
}

export function initClockMenu() {
  const chip = document.querySelector('.hud-clock');
  if (!chip) return;
  chip.setAttribute('aria-haspopup', 'dialog');
  chip.setAttribute('aria-expanded', 'false');
  chip.addEventListener('click', () => toggleClockMenu(chip));
  // The zone can arrive (or move) while the popover is open.
  window.addEventListener('weather:update', () => {
    if (menu.isOpen()) renderClock();
  });
}
