import {
  confirmDialog, controlButton, button, emptyState, field, input, modal, notify, textarea, toast,
  toggleRow, setTileGlow, glowGradient, largeTitle, metaLine, hairlineList, hairlineRow,
  listReveal,
} from '../../ui/index.js';
/**
 * calendar.js — the Calendar plugin's window (month grid + day detail).
 *
 * Editorial month view: a large month title with a mono "YEAR · N EVENTS"
 * line that hands over to the window bar on scroll (largeTitle), grey-pill
 * month arrows + Today and an ink "New event" pill, a hairline 7-column day
 * grid (mono numerals, today as an ink pill, selection as a grey fill, events
 * as small monochrome dot lines), and the selected day's events as hairline
 * rows (mono time column, title, mono location). Events live in the
 * plugin-owned `calendar_events` table; the window lists/creates/updates/
 * deletes them via /api/calendar/events.
 *
 * AI wiring: `calendar_*` tool outcomes arrive via `agent:actions` — the
 * window reloads its month and selects the date the AI touched.
 */

import { apiFetch } from '../../js/api.js';
import { t, tp } from '../../js/i18n.js';

export const CALENDAR_PLUGIN = 'calendar';

const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const WEEKDAY_KEYS = ['plugin.calendar.grid.sun', 'plugin.calendar.grid.mon', 'plugin.calendar.grid.tue',
  'plugin.calendar.grid.wed', 'plugin.calendar.grid.thu', 'plugin.calendar.grid.fri', 'plugin.calendar.grid.sat'];
const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June',
  'July', 'August', 'September', 'October', 'November', 'December'];
const MONTH_KEYS = ['plugin.calendar.month.january', 'plugin.calendar.month.february', 'plugin.calendar.month.march',
  'plugin.calendar.month.april', 'plugin.calendar.month.may', 'plugin.calendar.month.june',
  'plugin.calendar.month.july', 'plugin.calendar.month.august', 'plugin.calendar.month.september',
  'plugin.calendar.month.october', 'plugin.calendar.month.november', 'plugin.calendar.month.december'];

let tileEl = null;
let barEl = null;
let monthLabelEl = null;   // the bar's title (fades in as the large title scrolls away)
let scrollEl = null;
let hero = null;           // largeTitle() handle
let detailMetaEl = null;
let lastRevealKey = '';    // listReveal the day's rows once per (date, events) load
let shownMonthKey = '';
let weekdaysEl = null;
let gridEl = null;
let detailEl = null;
let detailHeadEl = null;

let viewYear = new Date().getFullYear();
let viewMonth = new Date().getMonth(); // 0-based
let selectedDate = isoDate(new Date());
let events = [];
let busy = false;

/* Event modal state */
let eventModal = null;
let editingId = null;
let titleInput = null;
let dateInput = null;
let startInput = null;
let endInput = null;
let allDayRow = null;
let descInput = null;
let locInput = null;

/* ── Dates ──────────────────────────────────────────────────── */

function pad(n) { return String(n).padStart(2, '0'); }

function isoDate(d) {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function parseIso(iso) {
  const [y, m, d] = String(iso).split('-').map(Number);
  return { year: y, month: m - 1, day: d };
}

function monthName(month = viewMonth) {
  return t(MONTH_KEYS[month], null, MONTHS[month]);
}

function monthLabel() {
  return `${monthName()} ${viewYear}`;
}

function eventsOn(date) {
  return events
    .filter((e) => e.date === date)
    .sort((a, b) => (a.start_time || '').localeCompare(b.start_time || '')
      || (a.title || '').localeCompare(b.title || ''));
}

function fmtTime(ev) {
  if (ev.all_day) return t('plugin.calendar.event.allDay', null, 'All day');
  if (ev.start_time && ev.end_time) return `${ev.start_time}–${ev.end_time}`;
  if (ev.start_time) return ev.start_time;
  return '—';
}

/** Ambient glow from theme tokens: the accent pair, placed by the selected
 *  date and strengthened by that day's load. The calendar always has a
 *  subject, so this never clears the glow. */
function updateCalendarGlow() {
  const d = parseIso(selectedDate);
  const load = Math.min(eventsOn(selectedDate).length / 4, 1);
  const a = `color-mix(in srgb, var(--accent) ${Math.round(70 + load * 30)}%, transparent)`;
  const b = `color-mix(in srgb, var(--accent-2) ${Math.round(60 + load * 40)}%, transparent)`;
  const x = Math.round(((d.day - 1) / 30) * 80 + 10);
  setTileGlow(tileEl, glowGradient(a, b, { x, y: 0, x2: 100 - x, y2: 100 }));
}

/* ── API ────────────────────────────────────────────────────── */

async function apiList(month) {
  const res = await apiFetch(`/api/calendar/events?month=${encodeURIComponent(month)}&limit=1000`);
  return res?.data?.events || [];
}

async function apiCreate(payload) {
  const res = await apiFetch('/api/calendar/events', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
  return res?.data;
}

async function apiUpdate(id, payload) {
  const res = await apiFetch(`/api/calendar/events/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify(payload),
  });
  return res?.data;
}

async function apiDelete(id) {
  await apiFetch(`/api/calendar/events/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/* ── Rendering ──────────────────────────────────────────────── */

function renderBar() {
  if (monthLabelEl) monthLabelEl.textContent = monthLabel();
  if (hero) {
    // decodeText only when the month actually changes, not on every reload.
    const key = `${viewYear}-${viewMonth}`;
    hero.setTitle(monthName(), { decode: !!shownMonthKey && shownMonthKey !== key });
    shownMonthKey = key;
    hero.setMeta([String(viewYear), tp('plugin.calendar.count.events', events.length)]);
  }
}

function renderWeekdays() {
  weekdaysEl.textContent = '';
  WEEKDAYS.forEach((wd, i) => {
    const el = document.createElement('div');
    el.className = 'calendar-weekday';
    el.textContent = t(WEEKDAY_KEYS[i], null, wd);
    weekdaysEl.appendChild(el);
  });
}

function renderGrid() {
  // Calendar data loads can resolve after the tile was unmounted (gridEl
  // torn down) — guard so a late render doesn't throw on a null node.
  if (!gridEl) return;
  gridEl.textContent = '';
  const first = new Date(viewYear, viewMonth, 1);
  const startOffset = first.getDay();
  const daysInMonth = new Date(viewYear, viewMonth + 1, 0).getDate();
  const todayIso = isoDate(new Date());

  const weeks = Math.ceil((startOffset + daysInMonth) / 7);
  for (let w = 0; w < weeks; w++) {
    for (let d = 0; d < 7; d++) {
      const dayNum = w * 7 + d - startOffset + 1;
      const cell = controlButton();
      cell.type = 'button';
      cell.className = 'calendar-day';

      if (dayNum < 1 || dayNum > daysInMonth) {
        cell.classList.add('is-other');
        cell.disabled = true;
        gridEl.appendChild(cell);
        continue;
      }

      const date = `${viewYear}-${pad(viewMonth + 1)}-${pad(dayNum)}`;
      cell.dataset.date = date;

      const num = document.createElement('span');
      num.className = 'calendar-day-num';
      num.textContent = String(dayNum);
      cell.appendChild(num);

      // Events read as small monochrome dot lines (title only where the
      // cell is wide enough; the dots alone on narrow windows).
      const dayEvents = eventsOn(date);
      const chips = dayEvents.slice(0, 3);
      if (chips.length) {
        const wrap = document.createElement('span');
        wrap.className = 'calendar-day-chips';
        for (const ev of chips) {
          const chip = document.createElement('span');
          chip.className = 'calendar-chip';
          chip.title = ev.title;
          const dot = document.createElement('span');
          dot.className = 'calendar-chip-dot';
          dot.setAttribute('aria-hidden', 'true');
          const label = document.createElement('span');
          label.className = 'calendar-chip-label';
          label.textContent = ev.title;
          chip.append(dot, label);
          wrap.appendChild(chip);
        }
        if (dayEvents.length > chips.length) {
          const more = document.createElement('span');
          more.className = 'calendar-chip-more';
          more.textContent = `+${dayEvents.length - chips.length}`;
          wrap.appendChild(more);
        }
        cell.appendChild(wrap);
        cell.setAttribute('aria-label', `${dayNum} · ${tp('plugin.calendar.count.events', dayEvents.length)}`);
      }

      if (date === todayIso) cell.classList.add('is-today');
      if (date === selectedDate) cell.classList.add('is-selected');
      cell.addEventListener('click', () => selectDate(date));
      gridEl.appendChild(cell);
    }
  }
  updateCalendarGlow();
}

function renderDetail() {
  if (!detailEl || !detailHeadEl) return;
  const dayEvents = eventsOn(selectedDate);
  const d = parseIso(selectedDate);
  const label = `${t(MONTH_KEYS[d.month], null, MONTHS[d.month])} ${d.day}, ${d.year}`;
  detailHeadEl.textContent = label;
  detailMetaEl?.set([tp('plugin.calendar.count.events', dayEvents.length)]);

  detailEl.textContent = '';
  if (!dayEvents.length) {
    detailEl.appendChild(emptyState({
      title: t('plugin.calendar.empty.noEvents', null, 'No events'),
      body: t('plugin.calendar.empty.noEventsBody', null, 'Nothing scheduled. Add an event or ask the AI to schedule one.'),
    }));
    return;
  }
  const list = hairlineList('calendar-detail-rows');
  for (const ev of dayEvents) {
    const time = document.createElement('span');
    time.className = 'calendar-detail-time';
    time.textContent = fmtTime(ev);

    const actions = document.createElement('div');
    actions.className = 'calendar-detail-actions';
    const editLabel = t('plugin.calendar.detail.edit', null, 'Edit');
    const editBtn = button({ label: editLabel, variant: 'ghost', size: 'sm', onClick: () => openEdit(ev) });
    editBtn.title = editLabel;
    const delBtn = button({ icon: 'ui/trash', variant: 'ghost', size: 'sm', onClick: () => removeEvent(ev) });
    delBtn.classList.add('ui-btn--icon', 'calendar-tool--danger');
    delBtn.title = t('plugin.calendar.detail.delete', null, 'Delete');
    actions.append(editBtn, delBtn);

    // Hairline row (the Mail pattern): mono time column, title + notes,
    // mono location, then the row's actions.
    const row = hairlineRow({
      lead: time,
      title: ev.title,
      sub: ev.description || '',
      meta: ev.location || '',
      trail: actions,
    });
    row.classList.add('calendar-detail-item');
    list.appendChild(row);
  }
  detailEl.appendChild(list);
  // Reveal once per loaded day, not on every repaint of the same list.
  const revealKey = `${selectedDate}|${dayEvents.map((ev) => ev.event_id).join(',')}`;
  if (revealKey !== lastRevealKey) {
    lastRevealKey = revealKey;
    void listReveal(list.children);
  }
}

function selectDate(date) {
  selectedDate = date;
  updateCalendarGlow();
  renderGrid();
  renderDetail();
}

/* ── Load / navigation ──────────────────────────────────────── */

async function loadEvents() {
  if (busy) return;
  busy = true;
  try {
    events = await apiList(`${viewYear}-${pad(viewMonth + 1)}`);
  } catch (e) {
    toast(e.message || t('plugin.calendar.toast.loadFailed', null, 'Could not load events'), { type: 'error' });
  } finally {
    busy = false;
  }
  renderBar();
  renderGrid();
  renderDetail();
}

function changeMonth(delta) {
  viewMonth += delta;
  if (viewMonth < 0) { viewMonth = 11; viewYear -= 1; }
  else if (viewMonth > 11) { viewMonth = 0; viewYear += 1; }
  const d = parseIso(selectedDate);
  selectedDate = `${viewYear}-${pad(viewMonth + 1)}-${pad(Math.min(d.day, new Date(viewYear, viewMonth + 1, 0).getDate()))}`;
  void loadEvents();
}

function gotoToday() {
  const now = new Date();
  viewYear = now.getFullYear();
  viewMonth = now.getMonth();
  selectedDate = isoDate(now);
  void loadEvents();
}

/* ── Event modal ────────────────────────────────────────────── */

function ensureModal() {
  if (eventModal) return;
  titleInput = input({ placeholder: t('plugin.calendar.dialog.titlePlaceholder', null, 'e.g. Team standup'), maxlength: 200 });
  dateInput = input({ type: 'date' });
  startInput = input({ type: 'time' });
  endInput = input({ type: 'time' });
  allDayRow = toggleRow({ label: t('plugin.calendar.event.allDay', null, 'All day') });
  locInput = input({ placeholder: t('plugin.calendar.dialog.locationPlaceholder', null, 'Location (optional)'), maxlength: 500 });
  descInput = textarea({ placeholder: t('plugin.calendar.dialog.notesPlaceholder', null, 'Notes (optional)'), rows: 3 });

  const actions = document.createElement('div');
  actions.className = 'ui-row calendar-dialog-actions';
  const cancelBtn = button({ label: t('plugin.calendar.dialog.cancel', null, 'Cancel'), variant: 'ghost', size: 'sm' });
  const saveBtn = button({ label: t('plugin.calendar.dialog.save', null, 'Save event'), variant: 'primary', size: 'sm', arrow: true });
  actions.append(cancelBtn, saveBtn);

  eventModal = modal({
    title: t('plugin.calendar.dialog.title', null, 'Event'),
    wide: true,
    body: [
      field({ label: t('plugin.calendar.dialog.titleField', null, 'Title'), control: titleInput }),
      field({ label: t('plugin.calendar.dialog.dateField', null, 'Date'), control: dateInput }),
      field({ label: t('plugin.calendar.dialog.startTimeField', null, 'Start time'), control: startInput }),
      field({ label: t('plugin.calendar.dialog.endTimeField', null, 'End time'), control: endInput }),
      allDayRow,
      field({ label: t('plugin.calendar.dialog.locationField', null, 'Location'), control: locInput }),
      field({ label: t('plugin.calendar.dialog.notesField', null, 'Notes'), control: descInput }),
      actions,
    ],
  });

  cancelBtn.addEventListener('click', () => eventModal.close());
  saveBtn.addEventListener('click', () => void saveFromModal());
}

function openNew() {
  ensureModal();
  editingId = null;
  eventModal.card.querySelector('h2').textContent = t('plugin.calendar.toolbar.newEvent', null, 'New event');
  titleInput.value = '';
  dateInput.value = selectedDate;
  startInput.value = '';
  endInput.value = '';
  allDayRow.toggle.setChecked(false, { silent: true });
  locInput.value = '';
  descInput.value = '';
  eventModal.open();
}

function openEdit(ev) {
  ensureModal();
  editingId = ev.event_id;
  eventModal.card.querySelector('h2').textContent = t('plugin.calendar.dialog.editTitle', null, 'Edit event');
  titleInput.value = ev.title || '';
  dateInput.value = ev.date || '';
  startInput.value = ev.start_time || '';
  endInput.value = ev.end_time || '';
  allDayRow.toggle.setChecked(!!ev.all_day, { silent: true });
  locInput.value = ev.location || '';
  descInput.value = ev.description || '';
  eventModal.open();
}

async function saveFromModal() {
  const payload = {
    title: titleInput.value.trim(),
    date: dateInput.value,
    start_time: startInput.value,
    end_time: endInput.value,
    all_day: allDayRow.toggle.isChecked(),
    location: locInput.value.trim(),
    description: descInput.value.trim(),
  };
  if (!payload.title) { toast(t('plugin.calendar.toast.titleRequired', null, 'Title is required'), { type: 'error' }); return; }
  if (!payload.date) { toast(t('plugin.calendar.toast.dateRequired', null, 'Date is required'), { type: 'error' }); return; }

  try {
    if (editingId) await apiUpdate(editingId, payload);
    else await apiCreate(payload);
    eventModal.close();
    const p = parseIso(payload.date);
    viewYear = p.year;
    viewMonth = p.month;
    selectedDate = payload.date;
    await loadEvents();
    toast(editingId ? t('plugin.calendar.toast.updated', null, 'Event updated') : t('plugin.calendar.toast.scheduled', null, 'Event scheduled'), { type: 'info' });
    editingId = null;
  } catch (e) {
    toast(e.message || t('plugin.calendar.toast.saveFailed', null, 'Could not save event'), { type: 'error' });
  }
}

async function removeEvent(ev) {
  if (!await confirmDialog(t('plugin.calendar.confirm.deleteEvent', { title: ev.title }, 'Delete "{title}"?'))) return;
  try {
    await apiDelete(ev.event_id);
    await loadEvents();
    toast(t('plugin.calendar.toast.deleted', null, 'Event deleted'), { type: 'info' });
  } catch (e) {
    toast(e.message || t('plugin.calendar.toast.deleteFailed', null, 'Could not delete event'), { type: 'error' });
  }
}

/* ── Upcoming-event reminders ─────────────────────────────────
   While the calendar window is open, poll today's events and raise a
   notification a few minutes before a timed event starts. */

const REMINDER_POLL_MS = 60 * 1000;
const REMINDER_WINDOW_MS = 10 * 60 * 1000;

let reminderTimer = null;
let notifiedKeys = new Set();

function eventStartMs(ev) {
  if (!ev?.date || ev.all_day || !ev.start_time) return null;
  const t = new Date(`${ev.date}T${ev.start_time}`).getTime();
  return Number.isNaN(t) ? null : t;
}

async function checkUpcomingEvents() {
  if (document.hidden) return;
  try {
    const now = new Date();
    const month = `${now.getFullYear()}-${pad(now.getMonth() + 1)}`;
    const list = await apiList(month);
    for (const ev of list) {
      const start = eventStartMs(ev);
      if (start == null) continue;
      const delta = start - Date.now();
      if (delta <= 0 || delta > REMINDER_WINDOW_MS) continue;
      const key = String(ev.event_id || ev.id);
      if (notifiedKeys.has(key)) continue;
      notifiedKeys.add(key);
      notify({
        app: 'Calendar',
        title: t('plugin.calendar.notify.eventSoon', null, 'Event starting soon'),
        body: `${ev.title} · ${ev.start_time}${ev.location ? ` · ${ev.location}` : ''}`,
        icon: 'ui/calendar',
        plugin: CALENDAR_PLUGIN,
        urgency: 'normal',
      });
    }
  } catch (_) { /* transient — next poll retries */ }
}

function startReminders() {
  stopReminders();
  void checkUpcomingEvents();
  reminderTimer = window.setInterval(() => void checkUpcomingEvents(), REMINDER_POLL_MS);
}

function stopReminders() {
  if (reminderTimer) window.clearInterval(reminderTimer);
  reminderTimer = null;
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

/** Create the Calendar tile element (the plugin's window container). */
export function mountCalendarTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile calendar-tile';
  tileEl.dataset.plugin = CALENDAR_PLUGIN;

  /* Top bar (merged with the window controls): grey-pill month arrows, the
     month title that fades in as the large title scrolls away, Today, and
     the ink "New event" pill. */
  barEl = document.createElement('div');
  barEl.className = 'calendar-bar';
  barEl.dataset.windowBar = '';

  const prevLabel = t('plugin.calendar.toolbar.previousMonth', null, 'Previous month');
  const prevBtn = button({ icon: 'ui/chevron-left', variant: 'ghost', size: 'sm', onClick: () => changeMonth(-1) });
  prevBtn.classList.add('calendar-nav-btn');
  prevBtn.title = prevLabel;
  prevBtn.setAttribute('aria-label', prevLabel);

  const nextLabel = t('plugin.calendar.toolbar.nextMonth', null, 'Next month');
  const nextBtn = button({ icon: 'ui/chevron-right', variant: 'ghost', size: 'sm', onClick: () => changeMonth(1) });
  nextBtn.classList.add('calendar-nav-btn');
  nextBtn.title = nextLabel;
  nextBtn.setAttribute('aria-label', nextLabel);

  monthLabelEl = document.createElement('div');
  monthLabelEl.className = 'calendar-month-label';

  const todayBtn = button({ label: t('plugin.calendar.toolbar.today', null, 'Today'), variant: 'ghost', size: 'sm', onClick: gotoToday });
  const newBtn = button({ label: t('plugin.calendar.toolbar.newEvent', null, 'New event'), variant: 'primary', size: 'sm', arrow: true, onClick: openNew });

  barEl.append(prevBtn, nextBtn, monthLabelEl, todayBtn, newBtn);
  tileEl.appendChild(barEl);

  /* Scrolling body: large month title, weekday header, day grid, day detail. */
  scrollEl = document.createElement('div');
  scrollEl.className = 'calendar-scroll';
  tileEl.appendChild(scrollEl);

  hero = largeTitle({ title: monthName(), meta: [String(viewYear)], scroller: scrollEl });
  hero.el.classList.add('calendar-hero');
  scrollEl.appendChild(hero.el);

  /* Weekday header */
  weekdaysEl = document.createElement('div');
  weekdaysEl.className = 'calendar-weekdays';
  scrollEl.appendChild(weekdaysEl);
  renderWeekdays();

  /* Day grid */
  gridEl = document.createElement('div');
  gridEl.className = 'calendar-grid';
  scrollEl.appendChild(gridEl);

  /* Day detail */
  const detail = document.createElement('div');
  detail.className = 'calendar-detail';
  const detailHead = document.createElement('div');
  detailHead.className = 'calendar-detail-head';
  detailHeadEl = document.createElement('span');
  detailHeadEl.className = 'calendar-detail-date';
  detailMetaEl = metaLine([]);
  detailHead.append(detailHeadEl, detailMetaEl);
  detail.appendChild(detailHead);
  detailEl = document.createElement('div');
  detailEl.className = 'calendar-detail-list';
  detail.appendChild(detailEl);
  scrollEl.appendChild(detail);

  renderBar();
  renderGrid();
  renderDetail();
  void loadEvents();
  startReminders();
  return tileEl;
}

/** Deactivated: drop the window. */
export function unmountCalendarTile() {
  stopReminders();
  eventModal?.close();
  hero?.dispose();
  hero = null;
  tileEl?.remove();
  tileEl = null;
  barEl = null;
  monthLabelEl = null;
  scrollEl = null;
  detailMetaEl = null;
  lastRevealKey = '';
  shownMonthKey = '';
  weekdaysEl = null;
  gridEl = null;
  detailEl = null;
  detailHeadEl = null;
}

/** The tile element (or null when the Calendar window is not mounted). */
export function getCalendarTileElement() {
  return tileEl;
}

/* ── AI wiring ──────────────────────────────────────────────── */

function onAgentActions(e) {
  const actions = e.detail || [];
  const calActions = actions.filter((a) => /^calendar_/.test(a?.action || ''));
  if (!calActions.length) return;

  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: CALENDAR_PLUGIN } }));

  const touched = calActions
    .map((a) => a.data?.date)
    .find((d) => !!d);
  if (touched) {
    const p = parseIso(touched);
    viewYear = p.year;
    viewMonth = p.month;
    selectedDate = touched;
  }
  void loadEvents();
}

let wired = false;
export function wireCalendarEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators + window management. */
export function calendarContextMenu() {
  return [
    { type: 'item', label: t('plugin.calendar.toolbar.newEvent', null, 'New event'), icon: 'ui/plus', onClick: () => openNew() },
    { type: 'item', label: t('plugin.calendar.toolbar.today', null, 'Today'), icon: 'ui/calendar', onClick: () => gotoToday() },
    { type: 'item', label: t('plugin.calendar.ctx.refresh', null, 'Refresh'), icon: 'ui/refresh', onClick: () => void loadEvents() },
    { type: 'separator' },
    { type: 'item', label: t('plugin.calendar.toolbar.previousMonth', null, 'Previous month'), icon: 'ui/chevron-left', onClick: () => changeMonth(-1) },
    { type: 'item', label: t('plugin.calendar.toolbar.nextMonth', null, 'Next month'), icon: 'ui/chevron-right', onClick: () => changeMonth(1) },
  ];
}

export default {
  name: 'calendar',
  icon: 'ui/calendar',
  mount: mountCalendarTile,
  unmount: unmountCalendarTile,
  getElement: getCalendarTileElement,
  wireEvents: wireCalendarEvents,
  contextMenu: calendarContextMenu,
};
