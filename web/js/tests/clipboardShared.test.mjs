/**
 * Pure-unit checks for clipboardShared.js. Run:
 *   node web/js/tests/clipboardShared.test.mjs
 */

import {
  CLIPBOARD_LIMIT, CLIPBOARD_PREVIEW_CHARS,
  isStorable, normalizeEntry, normalizeHistory, normalizeText,
  previewText, pushEntry, relativeTime, removeEntry,
} from '../clipboardShared.js';

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}`);
  }
}

/* ── normalizeText ─────────────────────────────────────────── */

check('text is trimmed', normalizeText('  hi  ') === 'hi');
check('empty text is null', normalizeText('') === null);
check('whitespace-only text is null', normalizeText('  \n ') === null);
check('non-strings are null', normalizeText(42) === null && normalizeText(null) === null);

/* ── storable ──────────────────────────────────────────────── */

check('short text is storable', isStorable('hello') === true);
check('oversized text is not storable', isStorable('x'.repeat(40000)) === false);

/* ── normalizeEntry / normalizeHistory ─────────────────────── */

check('entry keeps text/source/at', (() => {
  const e = normalizeEntry({ text: ' hi ', source: 'terminal', at: 1700000000000 });
  return e.text === 'hi' && e.source === 'terminal' && e.at === 1700000000000;
})());
check('entry defaults its source to app', normalizeEntry({ text: 'x' }).source === 'app');
check('entry without text is dropped', normalizeEntry({ at: 1 }) === null);
check('oversized entry is dropped', normalizeEntry({ text: 'x'.repeat(40000) }) === null);
check('non-object history is empty', normalizeHistory('nope').length === 0);
check('junk rows are dropped, order kept', (() => {
  const rows = normalizeHistory([
    { text: 'a', source: 'app', at: 2 },
    { nope: true },
    { text: 'b', source: 'browser', at: 1 },
  ]);
  return rows.length === 2 && rows[0].text === 'a' && rows[1].text === 'b';
})());
check('history is capped', normalizeHistory(
  Array.from({ length: CLIPBOARD_LIMIT + 10 }, (_, i) => ({ text: `t${i}`, at: i })),
).length === CLIPBOARD_LIMIT);

/* ── pushEntry ─────────────────────────────────────────────── */

check('push puts the entry on top', (() => {
  const list = pushEntry([{ text: 'old', at: 1 }], { text: 'new', at: 2 });
  return list.length === 2 && list[0].text === 'new' && list[1].text === 'old';
})());
check('push collapses a duplicate to the top', (() => {
  const list = pushEntry(
    [{ text: 'a', at: 1 }, { text: 'b', at: 2 }, { text: 'c', at: 3 }],
    { text: 'b', at: 4 },
  );
  return list.length === 3 && list.map((e) => e.text).join(',') === 'b,a,c';
})());
check('push trims to the limit', (() => {
  const full = Array.from({ length: CLIPBOARD_LIMIT }, (_, i) => ({ text: `t${i}`, at: i }));
  const list = pushEntry(full, { text: 'newest', at: 99 });
  return list.length === CLIPBOARD_LIMIT && list[0].text === 'newest'
    && list.at(-1).text === `t${CLIPBOARD_LIMIT - 2}`;
})());
check('push ignores an unusable entry', (() => {
  const list = pushEntry([{ text: 'a' }], { text: '   ' });
  return list.length === 1 && list[0].text === 'a';
})());

/* ── removeEntry ───────────────────────────────────────────── */

check('remove drops the indexed row', (() => {
  const list = removeEntry([{ text: 'a' }, { text: 'b' }], 0);
  return list.length === 1 && list[0].text === 'b';
})());
check('remove ignores out-of-range indices', removeEntry([{ text: 'a' }], 5).length === 1);

/* ── previewText ───────────────────────────────────────────── */

check('preview collapses newlines and tabs', previewText('a\n\tb  c') === 'a b c');
check('preview ellipsizes long text', (() => {
  const out = previewText('x'.repeat(500));
  return out.length === CLIPBOARD_PREVIEW_CHARS && out.endsWith('…');
})());

/* ── relativeTime ──────────────────────────────────────────── */

const NOW = 1700000000000;
check('fresh copy reads "just now"', relativeTime(NOW - 5000, NOW) === 'just now');
check('minutes are rounded', relativeTime(NOW - 4 * 60 * 1000, NOW) === '4 min ago');
check('hours are rounded', relativeTime(NOW - 3 * 3600 * 1000, NOW) === '3 h ago');
check('days are rounded', relativeTime(NOW - 2 * 86400 * 1000, NOW) === '2 d ago');
check('a missing timestamp reads empty', relativeTime(0, NOW) === '');
check('a week or more becomes a date',
  relativeTime(new Date(2020, 0, 2).getTime(), new Date(2020, 0, 10).getTime()) === '2/1/2020');

process.exit(failures ? 1 : 0);
