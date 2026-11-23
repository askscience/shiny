/**
 * The Radio window and the shell's Content-Security-Policy.
 *
 * `plugins/radio/web/plugin.js` reaches the station directory (Radio Browser)
 * and the now-playing artwork (MusicBrainz, Cover Art Archive) only through
 * its plugin's routes — the plugin sends the User-Agent those services ask
 * for — while its `<audio>` element opens each station's real stream URL,
 * which `media-src` in `CONTENT_SECURITY_POLICY` (`src/api/mod.rs`) must
 * admit. `connect-src` still lists the external hosts for Radio windows from
 * before 0.1.3 (plugins update apart from the shell); they can go once those
 * are gone.
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

// The window itself calls no external API: only its plugin's routes.
for (const host of ['radio-browser.info', 'musicbrainz.org', 'coverartarchive.org']) {
  check(`the window does not fetch ${host} itself`, !pluginJs.includes(host));
}
for (const route of ['/api/radio/search', '/api/radio/artwork', '/api/radio/nowplaying']) {
  check(`the window uses ${route}`, pluginJs.includes(route));
}
check('the plugin knows the Radio Browser hosts', /https:\/\/[a-z0-9.]*api\.radio-browser\.info/.test(pluginRs));

// Older Radio windows (before 0.1.3) fetch these directly: keep them admitted.
for (const origin of [
  'https://all.api.radio-browser.info',
  'https://musicbrainz.org',
  'https://coverartarchive.org',
  'https://archive.org',
]) {
  check(`connect-src still allows ${origin} for older windows`, sourceAllows(connectSrc, origin), connectSrc.join(' '));
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

console.log('\nthe Radio window goes through its plugin, and its streams are allowed by the CSP');
