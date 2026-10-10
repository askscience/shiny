/**
 * clockShared.js — pure time and mini-calendar helpers for the top-bar clock
 * and its popover (`hudLeft.js`, `clockMenu.js`).
 *
 * Dates the calendar plugin stores ("YYYY-MM-DD" in the server's local zone)
 * are handled as plain strings; only display formatting goes through Intl.
 * Kept import-free on purpose: `web/js/tests/clockShared.test.mjs` loads it in
 * Node with no DOM.
 */

export function pad2(n) {
  return String(n).padStart(2, '0');
}

/** Local date -> "YYYY-MM-DD" (the calendar plugin's storage format). */
export function isoDate(date) {
  return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())}`;
}

/** "YYYY-MM" for the calendar API's `month` parameter (month is 0-based). */
export function monthKey(year, month) {
  return `${year}-${pad2(month + 1)}`;
}

/** Step a {year, month} pair by whole months (month is 0-based). */
export function shiftMonth(year, month, delta) {
  const d = new Date(year, month + delta, 1);
  return { year: d.getFullYear(), month: d.getMonth() };
}

/**
 * The month as display cells, Sunday-first like the Calendar plugin: `null`
 * for each padding slot, else `{ iso, day }`.
 */
export function buildMonthCells(year, month) {
  const firstOffset = new Date(year, month, 1).getDay();
  const daysInMonth = new Date(year, month + 1, 0).getDate();
  const cells = [];
  for (let i = 0; i < firstOffset; i++) cells.push(null);
  for (let day = 1; day <= daysInMonth; day++) {
    cells.push({ iso: `${year}-${pad2(month + 1)}-${pad2(day)}`, day });
  }
  while (cells.length % 7 !== 0) cells.push(null);
  return cells;
}

/** "October 2026". */
export function monthLabel(year, month, locale = 'en-US') {
  return new Date(year, month, 1).toLocaleDateString(locale, { month: 'long', year: 'numeric' });
}

/** The seven short weekday names, Sunday-first, in the user's locale. */
export function weekdayShortNames(locale = 'en-US') {
  const fmt = new Intl.DateTimeFormat(locale, { weekday: 'short' });
  // 2024-01-07 was a Sunday; midday keeps a DST switch from moving the day.
  const sunday = Date.UTC(2024, 0, 7, 12);
  return Array.from({ length: 7 }, (_, i) => fmt.format(new Date(sunday + i * 86400000)));
}

/** A day header ("Friday, 10 Oct") from an ISO date. */
export function isoDayLabel(iso, locale = 'en-US') {
  const [y, m, d] = String(iso).split('-').map(Number);
  if (!y || !m || !d) return '';
  return new Date(y, m - 1, d).toLocaleDateString(locale, {
    weekday: 'long',
    day: 'numeric',
    month: 'short',
  });
}

/** Events indexed by day, each list in the plugin's order: time, then title. */
export function groupEventsByDate(events) {
  const map = new Map();
  for (const ev of events || []) {
    if (!ev?.date) continue;
    const list = map.get(ev.date) || [];
    list.push(ev);
    map.set(ev.date, list);
  }
  for (const list of map.values()) {
    list.sort((a, b) => (a.start_time || '').localeCompare(b.start_time || '')
      || (a.title || '').localeCompare(b.title || ''));
  }
  return map;
}

/** The plugin's time column: "All day", "09:30–10:00", "09:30" or "—". */
export function formatEventTime(ev) {
  if (ev?.all_day) return 'All day';
  if (ev?.start_time && ev?.end_time) return `${ev.start_time}–${ev.end_time}`;
  if (ev?.start_time) return ev.start_time;
  return '—';
}

/**
 * Wall-clock time at the located place. Falls back to the OS timezone when the
 * place's IANA zone is unknown (or Intl rejects it).
 */
export function formatLocationTime(date, timeZone, locale = 'en-US', options = {}) {
  const opts = { hour: '2-digit', minute: '2-digit', ...options };
  try {
    return date.toLocaleTimeString(locale, timeZone ? { ...opts, timeZone } : opts);
  } catch {
    return date.toLocaleTimeString(locale, opts);
  }
}

/** The date line next to the clock, in the same zone as the time. */
export function formatLocationDate(date, timeZone, locale = 'en-US', options = {}) {
  const opts = { weekday: 'short', day: 'numeric', month: 'short', ...options };
  try {
    return date.toLocaleDateString(locale, timeZone ? { ...opts, timeZone } : opts);
  } catch {
    return date.toLocaleDateString(locale, opts);
  }
}
