/**
 * Link router test.
 *
 * Which link clicks the kiosk routes to the browser plugin, and which plugin
 * they go to (the user's choice while it is installed, the `browser` default
 * otherwise). The classifier decides whether a click is hijacked at all, so a
 * wrong answer here is a dead link or a hijacked in-app navigation.
 *
 * The pure functions are extracted from `web/js/linkRouter.js` and evaluated
 * here, because the module's other imports (`api.js`, `preferences.js`) cannot
 * load in Node. The extraction asserts it found the functions, so a rename
 * fails loudly rather than silently testing nothing.
 *
 * Run:  node web/js/tests/linkRouter.test.mjs
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, '..', 'linkRouter.js'), 'utf8');

const start = source.indexOf('export function isKiosk');
const end = source.indexOf('/** The shared plugin catalog');
if (start < 0 || end < 0 || end <= start) {
  console.error('could not locate the router predicates in linkRouter.js — did they move?');
  process.exit(1);
}
const body = source.slice(start, end).replace(/^export /gm, '');
const scopeHelpers = `
  const location = { origin: 'https://app.local' };
  const LINK_CAPABILITY = 'open-urls';
  const DEFAULT_LINK_PLUGIN = 'browser';
`;
const build = new Function(
  'window',
  `${scopeHelpers}${body}; return { isKiosk, externalLinkUrl, linkPlugins, resolveLinkPlugin };`,
);

/** The kiosk case: both shells expose the bridge. */
const router = build({ ipc: { postMessage: () => {} } });
/** A desktop browser: no bridge, so the router must stay inert. */
const plain = build({});

let failures = 0;
const check = (name, condition, detail = '') => {
  console.log(`  ${condition ? 'ok  ' : 'FAIL'} ${name}${detail ? ` — ${detail}` : ''}`);
  if (!condition) failures += 1;
};

console.log('link router');

/* ── What counts as a routable link ───────────────────────────── */

check('the kiosk is detected from the shell bridge', router.isKiosk() === true);
check('a desktop browser is not a kiosk', plain.isKiosk() === false);

const APP = 'https://app.local';
check(
  'an https link on another origin is routed',
  router.externalLinkUrl('https://example.com/x', APP) === 'https://example.com/x',
);
check(
  'an http link is routed',
  router.externalLinkUrl('http://example.com/x', APP) === 'http://example.com/x',
);
check(
  'a same-origin link keeps its default behaviour',
  router.externalLinkUrl('https://app.local/settings', APP) === null,
);
check('a relative path is not routed', router.externalLinkUrl('/settings', APP) === null);
check('a fragment is not routed', router.externalLinkUrl('#section', APP) === null);
check('mailto is not routed', router.externalLinkUrl('mailto:a@example.com', APP) === null);
check('blob is not routed', router.externalLinkUrl('blob:https://app.local/abc', APP) === null);
check('javascript is not routed', router.externalLinkUrl('javascript:alert(1)', APP) === null);
check('garbage is not a URL', router.externalLinkUrl('not a url', APP) === null);

/* ── Which plugin a link goes to ──────────────────────────────── */

const CATALOG = [
  { name: 'browser', surface: true, capabilities: ['open-urls'] },
  { name: 'studio', surface: true, capabilities: ['music'] },
  { name: 'files', surface: true }, // predates capabilities: no key at all
  { name: 'alt-browser', surface: true, capabilities: ['open-urls'] },
  { name: 'headless', surface: false, capabilities: ['open-urls'] }, // no window
];

check(
  'only capability plugins with a window are candidates',
  router.linkPlugins(CATALOG).map((p) => p.name).join(',') === 'browser,alt-browser',
  router.linkPlugins(CATALOG).map((p) => p.name).join(','),
);
check(
  'a plugin without capabilities is never chosen',
  router.resolveLinkPlugin([{ name: 'studio', surface: true, capabilities: ['music'] }], 'studio') === null,
);
check('no candidates → nothing to open with', router.resolveLinkPlugin([], 'browser') === null);
check('an empty catalog resolves to nothing', router.resolveLinkPlugin(null, 'browser') === null);

check(
  "the user's choice wins while it is installed",
  router.resolveLinkPlugin(CATALOG, 'alt-browser') === 'alt-browser',
);
check(
  'a choice that is no longer installed falls back to the browser',
  router.resolveLinkPlugin(CATALOG, 'uninstalled') === 'browser',
);
check(
  'without a browser plugin the first candidate is used',
  router.resolveLinkPlugin(CATALOG.filter((p) => p.name !== 'browser'), 'browser') === 'alt-browser',
);
check(
  'a browser plugin without the capability is not the default',
  router.resolveLinkPlugin([{ name: 'browser', surface: true }], 'browser') === null,
);

if (failures) {
  console.error(`\n${failures} failure(s)`);
  process.exit(1);
}
console.log('\nall link router checks passed');
