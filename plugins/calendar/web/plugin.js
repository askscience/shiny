/**
 * calendar.js — the Calendar plugin's window (month grid + day detail).
 *
 * A month grid with a hairline toolbar (prev/next/today/new event), a 7-column
 * day grid, and a detail strip for the selected day. Events live in the
 * plugin-owned `calendar_events` table; the window lists/creates/updates/
 * deletes them via /api/calendar/events.
 *
 * AI wiring: `calendar_*` tool outcomes arrive via `agent:actions` — the
 * window reloads its month and selects the date the AI touched.
 */

import { button, emptyState, field, input, modal, textarea, toast, toggleRow } from '/ui/index.js';
import { setIcon } from '/ui/index.js';
import { apiFetch } from '/js/api.js';

export const CALENDAR_PLUGIN = 'calendar';

const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June',
  'July', 'August', 'September', 'October', 'November', 'December'];

let tileEl = null;
let barEl = null;
let monthLabelEl = null;
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

function monthLabel() {
  return `${MONTHS[viewMonth]} ${viewYear}`;
}

function eventsOn(date) {
  return events
    .filter((e) => e.date === date)
    .sort((a, b) => (a.start_time || '').localeCompare(b.start_time || '')
      || (a.title || '').localeCompare(b.title || ''));
}

function fmtTime(ev) {
  if (ev.all_day) return 'All day';
  if (ev.start_time && ev.end_time) return `${ev.start_time}–${ev.end_time}`;
  if (ev.start_time) return ev.start_time;
  return '—';
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
}

function renderWeekdays() {
  weekdaysEl.textContent = '';
  for (const wd of WEEKDAYS) {
    const el = document.createElement('div');
    el.className = 'calendar-weekday';
    el.textContent = wd;
    weekdaysEl.appendChild(el);
  }
}

function renderGrid() {
  gridEl.textContent = '';
  const first = new Date(viewYear, viewMonth, 1);
  const startOffset = first.getDay();
  const daysInMonth = new Date(viewYear, viewMonth + 1, 0).getDate();
  const todayIso = isoDate(new Date());

  const weeks = Math.ceil((startOffset + daysInMonth) / 7);
  for (let w = 0; w < weeks; w++) {
    for (let d = 0; d < 7; d++) {
      const dayNum = w * 7 + d - startOffset + 1;
      const cell = document.createElement('button');
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

      const chips = eventsOn(date).slice(0, 3);
      if (chips.length) {
        const wrap = document.createElement('span');
        wrap.className = 'calendar-day-chips';
        for (const ev of chips) {
          const chip = document.createElement('span');
          chip.className = 'calendar-chip';
          chip.textContent = ev.title;
          wrap.appendChild(chip);
        }
        cell.appendChild(wrap);
      }

      if (date === todayIso) cell.classList.add('is-today');
      if (date === selectedDate) cell.classList.add('is-selected');
      cell.addEventListener('click', () => selectDate(date));
      gridEl.appendChild(cell);
    }
  }
}

function renderDetail() {
  if (!detailEl || !detailHeadEl) return;
  const dayEvents = eventsOn(selectedDate);
  const d = parseIso(selectedDate);
  const label = `${MONTHS[d.month]} ${d.day}, ${d.year}`;
  detailHeadEl.textContent = label;

  detailEl.textContent = '';
  if (!dayEvents.length) {
    detailEl.appendChild(emptyState({ title: 'No events', body: 'Nothing scheduled. Add an event or ask the AI to schedule one.' }));
    return;
  }
  for (const ev of dayEvents) {
    const row = document.createElement('div');
    row.className = 'calendar-detail-item';

    const time = document.createElement('span');
    time.className = 'calendar-detail-time';
    time.textContent = fmtTime(ev);
    const main = document.createElement('div');
    main.className = 'calendar-detail-main';
    const title = document.createElement('div');
    title.className = 'calendar-detail-title';
    title.textContent = ev.title;
    main.appendChild(title);
    const meta = [];
    if (ev.location) meta.push(ev.location);
    if (ev.description) meta.push(ev.description);
    if (meta.length) {
      const sub = document.createElement('div');
      sub.className = 'calendar-detail-sub';
      sub.textContent = meta.join(' · ');
      main.appendChild(sub);
    }

    const actions = document.createElement('div');
    actions.className = 'calendar-detail-actions';
    const editBtn = button({ label: 'Edit', variant: 'ghost', size: 'sm', onClick: () => openEdit(ev) });
    editBtn.title = 'Edit';
    const delBtn = button({ icon: 'ui/trash', variant: 'ghost', size: 'sm', onClick: () => removeEvent(ev) });
    delBtn.classList.add('ui-btn--icon', 'calendar-tool--danger');
    delBtn.title = 'Delete';
    actions.append(editBtn, delBtn);

    row.append(time, main, actions);
    detailEl.appendChild(row);
  }
}

function selectDate(date) {
  selectedDate = date;
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
    toast(e.message || 'Could not load events', { type: 'error' });
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
  titleInput = input({ placeholder: 'e.g. Team standup', maxlength: 200 });
  dateInput = input({ type: 'date' });
  startInput = input({ type: 'time' });
  endInput = input({ type: 'time' });
  allDayRow = toggleRow({ label: 'All day' });
  locInput = input({ placeholder: 'Location (optional)', maxlength: 500 });
  descInput = textarea({ placeholder: 'Notes (optional)', rows: 3 });

  const actions = document.createElement('div');
  actions.className = 'ui-row';
  actions.style.justifyContent = 'flex-end';
  actions.style.gap = '8px';
  const cancelBtn = button({ label: 'Cancel', variant: 'ghost', size: 'sm' });
  const saveBtn = button({ label: 'Save event', variant: 'primary', size: 'sm' });
  actions.append(cancelBtn, saveBtn);

  eventModal = modal({
    title: 'Event',
    wide: true,
    body: [
      field({ label: 'Title', control: titleInput }),
      field({ label: 'Date', control: dateInput }),
      field({ label: 'Start time', control: startInput }),
      field({ label: 'End time', control: endInput }),
      allDayRow,
      field({ label: 'Location', control: locInput }),
      field({ label: 'Notes', control: descInput }),
      actions,
    ],
  });

  cancelBtn.addEventListener('click', () => eventModal.close());
  saveBtn.addEventListener('click', () => void saveFromModal());
}

function openNew() {
  ensureModal();
  editingId = null;
  eventModal.card.querySelector('h2').textContent = 'New event';
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
  eventModal.card.querySelector('h2').textContent = 'Edit event';
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
  if (!payload.title) { toast('Title is required', { type: 'error' }); return; }
  if (!payload.date) { toast('Date is required', { type: 'error' }); return; }

  try {
    if (editingId) await apiUpdate(editingId, payload);
    else await apiCreate(payload);
    eventModal.close();
    const p = parseIso(payload.date);
    viewYear = p.year;
    viewMonth = p.month;
    selectedDate = payload.date;
    await loadEvents();
    toast(editingId ? 'Event updated' : 'Event scheduled', { type: 'info' });
    editingId = null;
  } catch (e) {
    toast(e.message || 'Could not save event', { type: 'error' });
  }
}

async function removeEvent(ev) {
  if (!window.confirm(`Delete "${ev.title}"?`)) return;
  try {
    await apiDelete(ev.event_id);
    await loadEvents();
    toast('Event deleted', { type: 'info' });
  } catch (e) {
    toast(e.message || 'Could not delete event', { type: 'error' });
  }
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

/** Create the Calendar tile element (the plugin's window container). */
export function mountCalendarTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile calendar-tile';
  tileEl.dataset.plugin = CALENDAR_PLUGIN;

  /* Top bar */
  barEl = document.createElement('div');
  barEl.className = 'calendar-bar';

  const prevBtn = button({ icon: 'ui/chevron-left', variant: 'ghost', size: 'sm', onClick: () => changeMonth(-1) });
  prevBtn.classList.add('ui-btn--icon', 'calendar-nav-btn');
  prevBtn.title = 'Previous month';

  monthLabelEl = document.createElement('div');
  monthLabelEl.className = 'calendar-month-label';

  const nextBtn = button({ icon: 'ui/chevron-right', variant: 'ghost', size: 'sm', onClick: () => changeMonth(1) });
  nextBtn.classList.add('ui-btn--icon', 'calendar-nav-btn');
  nextBtn.title = 'Next month';

  const spacer = document.createElement('div');
  spacer.className = 'calendar-bar-spacer';

  const todayBtn = button({ label: 'Today', variant: 'ghost', size: 'sm', onClick: gotoToday });
  const newBtn = button({ icon: 'ui/plus', label: 'New event', variant: 'primary', size: 'sm', onClick: openNew });

  barEl.append(prevBtn, monthLabelEl, nextBtn, spacer, todayBtn, newBtn);
  tileEl.appendChild(barEl);

  /* Weekday header */
  weekdaysEl = document.createElement('div');
  weekdaysEl.className = 'calendar-weekdays';
  tileEl.appendChild(weekdaysEl);
  renderWeekdays();

  /* Day grid */
  gridEl = document.createElement('div');
  gridEl.className = 'calendar-grid';
  tileEl.appendChild(gridEl);

  /* Day detail */
  const detail = document.createElement('div');
  detail.className = 'calendar-detail';
  detailHeadEl = document.createElement('div');
  detailHeadEl.className = 'calendar-detail-head';
  detail.appendChild(detailHeadEl);
  detailEl = document.createElement('div');
  detailEl.className = 'calendar-detail-list';
  detail.appendChild(detailEl);
  tileEl.appendChild(detail);

  renderBar();
  renderGrid();
  renderDetail();
  void loadEvents();
  return tileEl;
}

/** Deactivated: drop the window. */
export function unmountCalendarTile() {
  eventModal?.close();
  tileEl?.remove();
  tileEl = null;
  barEl = null;
  monthLabelEl = null;
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

export default {
  name: 'calendar',
  icon: 'ui/calendar',
  mount: mountCalendarTile,
  unmount: unmountCalendarTile,
  getElement: getCalendarTileElement,
  wireEvents: wireCalendarEvents,
};
