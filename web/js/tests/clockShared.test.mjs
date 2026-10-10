/**
 * Pure-unit checks for clockShared.js. Run:
 *   node web/js/tests/clockShared.test.mjs
 */

import {
  buildMonthCells, formatEventTime, formatLocationDate, formatLocationTime,
  groupEventsByDate, isoDate, isoDayLabel, monthKey, monthLabel, pad2, shiftMonth,
  weekdayShortNames,
} from '../clockShared.js';

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}`);
  }
}

check('pad2', pad2(3) === '03' && pad2(12) === '12');
check('iso date', isoDate(new Date(2026, 9, 10)) === '2026-10-10');
check('month key', monthKey(2026, 9) === '2026-10');

check('shift month forward', JSON.stringify(shiftMonth(2026, 11, 1)) === JSON.stringify({ year: 2027, month: 0 }));
check('shift month back', JSON.stringify(shiftMonth(2026, 0, -1)) === JSON.stringify({ year: 2025, month: 11 }));
check('shift month none', JSON.stringify(shiftMonth(2026, 5, 0)) === JSON.stringify({ year: 2026, month: 5 }));

// 2026-10-01 is a Thursday, so Sunday-first October 2026 opens with 4 blanks.
const october = buildMonthCells(2026, 9);
check('october pads the week start', october.slice(0, 4).every((c) => c === null));
check('october first day', october[4]?.day === 1 && october[4]?.iso === '2026-10-01');
check('october fills whole weeks', october.length % 7 === 0);
check('october day count', october.filter(Boolean).length === 31);
check('october last day', october.filter(Boolean).at(-1)?.iso === '2026-10-31');
check('leap february', buildMonthCells(2024, 1).filter(Boolean).length === 29);
check('non-leap february', buildMonthCells(2025, 1).filter(Boolean).length === 28);

check('month label', monthLabel(2026, 9, 'en-US') === 'October 2026');
check('weekday names', weekdayShortNames('en-US').join(',') === 'Sun,Mon,Tue,Wed,Thu,Fri,Sat');
check('day label', isoDayLabel('2026-10-10', 'en-US') === 'Saturday, Oct 10');
check('day label bad input', isoDayLabel('', 'en-US') === '');

const events = [
  { title: 'Zeta', date: '2026-10-10', start_time: '09:30', end_time: '10:00' },
  { title: 'Alpha', date: '2026-10-10', start_time: '09:30' },
  { title: 'Early', date: '2026-10-10', start_time: '08:00' },
  { title: 'Holiday', date: '2026-10-11', all_day: true },
  { title: 'No date' },
];
const grouped = groupEventsByDate(events);
check('groups by day', grouped.size === 2 && grouped.get('2026-10-10').length === 3);
check('sorts by time then title', grouped.get('2026-10-10').map((e) => e.title).join(',') === 'Early,Alpha,Zeta');
check('groups nothing', groupEventsByDate(null).size === 0);

check('event range', formatEventTime({ start_time: '09:30', end_time: '10:00' }) === '09:30–10:00');
check('event start only', formatEventTime({ start_time: '09:30' }) === '09:30');
check('event all day wins', formatEventTime({ all_day: true, start_time: '09:30' }) === 'All day');
check('event unknown', formatEventTime({}) === '—');

// 2026-10-10T12:00Z: Rome is still on CEST (UTC+2), New York on EDT (UTC-4).
const noonUtc = new Date(Date.UTC(2026, 9, 10, 12, 0, 0));
const hhmm = { hour: '2-digit', minute: '2-digit', hour12: false };
check('time in Rome', formatLocationTime(noonUtc, 'Europe/Rome', 'en-US', hhmm) === '14:00');
check('time in New York', formatLocationTime(noonUtc, 'America/New_York', 'en-US', hhmm) === '08:00');
check('time without a zone', formatLocationTime(noonUtc, null, 'en-US', hhmm) === noonUtc.toLocaleTimeString('en-US', hhmm));
check('time with a bad zone falls back', formatLocationTime(noonUtc, 'Not/AZone', 'en-US', hhmm) === noonUtc.toLocaleTimeString('en-US', hhmm));
check('date with a bad zone falls back', formatLocationDate(noonUtc, 'Not/AZone', 'en-US') === noonUtc.toLocaleDateString('en-US', { weekday: 'short', day: 'numeric', month: 'short' }));

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall clockShared checks passed');
