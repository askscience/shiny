/**
 * The Radio window talks to external services from the shell document.
 *
 * `plugins/radio/web/plugin.js` fetches the public Radio Browser directory
 * (station grid), MusicBrainz and the Cover Art Archive (now-playing artwork)
 * directly from the browser, and its `<audio>` element opens each station's
 * real stream URL. All of that is governed by `CONTENT_SECURITY_POLICY` in
 * `src/api/mod.rs`: without the hosts in `connect-src` every fetch fails with
 * a CSP violation and the window paints "Radio Browser unreachable"; without
 * external `media-src` the stream itself is refused. This test keeps the hosts
 * used by the plugin and the policy in sync.
 *
 * Run: node web/js/tests/cspRadioHosts.test.mjs
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../../', import.meta.url));

let failures = 0;
function check(name, condition, detail) {
  if (condition) console.log(`  ok  ${name}`);
  else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

const apiMod = readFileSync(join(root, 'src/api/mod.rs'), 'utf8');
const pluginJs = readFileSync(join(root, 'plugins/radio/web/plugin.js'), 'utf8');
const pluginRs = readFileSync(join(root, 'plugins/radio/src/radio_browser.rs'), 'utf8');

// Strip the Rust line-continuation escapes (`\` + newline) before parsing.
const policy = (/const CONTENT_SECURITY_POLICY: &str = "([\s\S]*?)";/.exec(apiMod)?.[1] ?? '')
  .replace(/\\\n/g, ' ');
const directives = new Map(
  policy.split(';').map((part) => {
    const tokens = part.trim().split(/\s+/).filter(Boolean);
    return [tokens[0], tokens.slice(1)];
  }),
);

function hostOf(url) {
  try {
    const u = new URL(url);
    return `${u.protocol}//${u.host}`;
  } catch {
    return null;
  }
}

/** CSP host-source matching: exact origin or a `*.` subdomain wildcard. */
function sourceAllows(sources, origin) {
  const target = new URL(origin);
  return sources.some((source) => {
    if (source === "'self'") return false;
    const wildcard = /^(\w+):\/\/\*\.(.+)$/.exec(source);
    if (wildcard) {
      return target.protocol === `${wildcard[1]}:` && target.hostname.endsWith(`.${wildcard[2]}`);
    }
    if (/^\w+:\/\/[^/]+$/.test(source)) return source === origin;
    return false;
  });
}

const connectSrc = directives.get('connect-src') ?? [];
const mediaSrc = directives.get('media-src') ?? [];

// Every API host the window may try must be admitted by connect-src.
const jsHosts = [...pluginJs.matchAll(/'(https:\/\/[^']+)'/g)]
  .map((m) => hostOf(m[1]))
  .filter((host) => host && host.includes('api.radio-browser.info'));
const rustHosts = [...pluginRs.matchAll(/"(https:\/\/[^"]+)"/g)]
  .map((m) => hostOf(m[1]))
  .filter((host) => host && host.includes('api.radio-browser.info'));

check('the window has Radio Browser API hosts', jsHosts.length > 0);
check(
  'plugin.js and radio_browser.rs try the same Radio Browser hosts',
  JSON.stringify(jsHosts) === JSON.stringify(rustHosts),
  `js: ${jsHosts.join(', ')} · rust: ${rustHosts.join(', ')}`,
);
for (const host of new Set(jsHosts)) {
  check(`connect-src allows ${host}`, sourceAllows(connectSrc, host), connectSrc.join(' '));
}

// MusicBrainz looks up the release; Cover Art Archive answers with a 307 to
// archive.org (and then a per-server `*.archive.org` node), so the whole
// redirect chain must be admitted too.
for (const origin of [
  'https://musicbrainz.org',
  'https://coverartarchive.org',
  'https://archive.org',
  'https://dn710701.ca.archive.org',
]) {
  check(`connect-src allows ${origin}`, sourceAllows(connectSrc, origin), connectSrc.join(' '));
}

check(
  'media-src admits external https streams',
  mediaSrc.includes('https:'),
  mediaSrc.join(' '),
);
check(
  'media-src admits external http streams (many stations are plain HTTP)',
  mediaSrc.includes('http:'),
  mediaSrc.join(' '),
);

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}

console.log('\nthe Radio window\'s external hosts and stream media are allowed by the CSP');
