/**
 * Dock ownership test.
 *
 * A window's dock must show that window's own cards and nothing else. This
 * exists because it did not: `getDockSummaries()` returned every plugin's
 * artifacts, so a `browser_page` card — whose title *is* its URL — was drawn
 * inside the traveler window, and traveler cards could reach any other window's
 * dock. The rule now lives in `artifactBelongsTo`, and the cases below are the
 * real mix of artifact types and owners from the live database.
 *
 * The predicate is extracted from `web/js/artifactStore.js` and evaluated
 * here, because the module's other imports (`api.js` → browser globals) cannot
 * load in Node. The extraction asserts it found the function, so a rename
 * fails loudly rather than silently testing nothing.
 *
 * Run:  node web/js/tests/artifactOwnership.test.mjs
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const storePath = join(here, '..', 'artifactStore.js');
const source = readFileSync(storePath, 'utf8');

const start = source.indexOf('const TRAVELER_TYPES');
const end = source.indexOf('export function getSavedDestinations');
if (start < 0 || end < 0 || end <= start) {
  console.error('could not locate the ownership rule in artifactStore.js — did it move?');
  process.exit(1);
}
// Only the pure predicates; their helpers are taken along by the same slice.
const body = source
  .slice(start, end)
  .replace(/^export /gm, '')
  .replace(/^import .*$/gm, '');
// `summaries` and `activeDestinationKey` are module state, so they are
// re-exposed as setters rather than reached for from out here.
const scopeHelpers = `
  let summaries = [];
  let activeDestinationKey = '';
  function dedupeSummaries(list) { return list; }
  function sortDockSummaries(list) { return list; }
  function destinationKeyForSummary() { return ''; }
  function topicSlotForSummary() { return 'other'; }
  const DOCK_TOPIC_SLOTS = [];
  function __setSummaries(list) { summaries = list; }
  function __setActiveDestination(key) { activeDestinationKey = key; }
`;
const { artifactBelongsTo, getDockSummaries, __setSummaries, __setActiveDestination } = new Function(
  `${scopeHelpers}${body}; return { artifactBelongsTo, getDockSummaries, __setSummaries, __setActiveDestination };`,
)();

/**
 * `[artifact_type, plugin, count]` exactly as stored today.
 *
 * The untagged rows matter most: they were saved before plugins stamped their
 * owner, and they are traveler guides. Treating "no owner" as "no window" would
 * have silently emptied the traveler dock of 103 real cards.
 */
const REAL_ARTIFACTS = [
  ['youtube_video', 'youtube', 33],
  ['travel_plan', null, 28],
  ['site_info', null, 27],
  ['monument_info', null, 24],
  ['poi_list', null, 24],
  ['radio_station', 'radio', 20],
  ['travel_plan', 'traveler', 2],
  ['browser_page', 'browser', 1],
  ['monument_info', 'traveler', 1],
  ['poi_list', 'traveler', 1],
  ['site_info', 'core', 1],
  ['site_info', 'traveler', 1],
];

let failures = 0;
const check = (name, condition, detail = '') => {
  console.log(`  ${condition ? 'ok  ' : 'FAIL'} ${name}${detail ? ` — ${detail}` : ''}`);
  if (!condition) failures += 1;
};

console.log('dock ownership');

for (const [type, plugin, count] of REAL_ARTIFACTS) {
  const summary = { type, plugin };
  const label = `${type}/${plugin || '(untagged)'} x${count}`;
  const inTraveler = artifactBelongsTo(summary, 'traveler');
  const inBrowser = artifactBelongsTo(summary, 'browser');

  if (plugin === 'browser') {
    check(`${label}: not in the traveler dock`, !inTraveler);
    check(`${label}: shown in the browser dock`, inBrowser);
  } else if (plugin === 'traveler') {
    check(`${label}: shown in the traveler dock`, inTraveler);
    check(`${label}: not in the browser dock`, !inBrowser);
  } else if (plugin === null) {
    check(`${label}: legacy card still reaches the traveler dock`, inTraveler);
    check(`${label}: not in the browser dock`, !inBrowser);
  } else {
    check(`${label}: not in the traveler dock`, !inTraveler);
    check(`${label}: not in the browser dock`, !inBrowser);
  }
}

// The exact regression: a browser card must never be judged the traveler's.
check(
  'the reported bug cannot recur',
  artifactBelongsTo({ type: 'browser_page', plugin: 'browser' }, 'traveler') === false,
);
check(
  'an untagged browser page cannot pose as a traveler card',
  artifactBelongsTo({ type: 'browser_page' }, 'traveler') === false,
);
check('a null summary matches nothing', artifactBelongsTo(null, 'traveler') === false);
check(
  'an unknown scope matches nothing',
  artifactBelongsTo({ type: 'browser_page', plugin: 'browser' }, 'nosuch') === false,
);

// A browser dock must not be emptied by the traveler's current city, and the
// traveler's topic slots must not decide what the browser shows.
const browserSummaries = [
  { id: 'b1', type: 'browser_page', plugin: 'browser', title: 'https://example.com' },
  { id: 't1', type: 'travel_plan', plugin: 'traveler', title: 'Trip to Rome', theme: 'overview' },
];
__setSummaries(browserSummaries);
__setActiveDestination('rome');
check(
  'the browser dock shows its own card even when the traveler has an active city',
  getDockSummaries('browser').length === 1 && getDockSummaries('browser')[0].id === 'b1',
  JSON.stringify(getDockSummaries('browser')),
);
check(
  'the browser dock never draws a traveler card',
  getDockSummaries('browser').every((s) => s.plugin === 'browser'),
);
check(
  'the unscoped chrome dock still groups by destination',
  getDockSummaries().every((s) => s.id !== 'b1'),
);

if (failures) {
  console.error(`\n${failures} failure(s)`);
  process.exit(1);
}
console.log('\nall dock ownership checks passed');
