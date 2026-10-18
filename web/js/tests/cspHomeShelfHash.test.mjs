/**
 * The Browser start page's inline script vs. the app's CSP hash.
 *
 * The Browser plugin's start page is a sandboxed `srcdoc` iframe built by
 * `homeDocument()` in `plugins/browser/web/plugin.js`. The app's global CSP has
 * no `'unsafe-inline'`, so that inline script runs only because its exact
 * SHA-256 is allowlisted in `CONTENT_SECURITY_POLICY` (`src/api/mod.rs`).
 * Editing the script without updating the hash makes Chromium refuse the frame,
 * and the kiosk then paints "127.0.0.1 refused to connect" over the Browser
 * window's page area (this shipped once — see commit 08dfc23).
 *
 * Run: node web/js/tests/cspHomeShelfHash.test.mjs
 */
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
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

const pluginJs = readFileSync(join(root, 'plugins/browser/web/plugin.js'), 'utf8');
const apiMod = readFileSync(join(root, 'src/api/mod.rs'), 'utf8');

// homeDocument() writes its script tags as `<scr${'ipt'}>` so the plugin file
// itself has no closing tag. The click/refresh handler is the only block that
// starts on its own line (the JSON data block is inline), and Chromium hashes
// the element's text — the newlines around the body included.
const blocks = [...pluginJs.matchAll(/<scr\$\{'ipt'\}>\n([\s\S]*?)\n<\/scr\$\{'ipt'\}>/g)];
check(
  'homeDocument emits exactly one executable inline script',
  blocks.length === 1,
  `${blocks.length} multi-line <script> block(s)`,
);

const handler = blocks[0]?.[1] ?? '';
check(
  'the hashed block is the shelf click/refresh handler',
  handler.includes('browser:open-card') && handler.includes('parent.postMessage'),
);

// The invariant that broke the start page once: a `${…}` in the script makes
// its bytes depend on the resolved value (locale, data), so the one allowlisted
// hash can never match what Chromium sees. Translated text belongs in the JSON
// data block next to the script, which is not hashed.
check(
  'the hashed script carries no interpolation',
  !handler.includes('${'),
  'move the interpolated value into the JSON data block instead',
);

const homeAt = pluginJs.indexOf('function homeDocument(');
check(
  'the handler is the one inside homeDocument',
  homeAt !== -1 && !!blocks[0] && blocks[0].index > homeAt,
);

// Chromium hashes the script element's text, so the leading and trailing
// newlines are part of the digest — this exact rule reproduces the hash that
// shipped with the shelf before the i18n pass changed the script.
const digest = `sha256-${createHash('sha256').update(`\n${handler}\n`, 'utf8').digest('base64')}`;

check(
  'the CSP allowlists the current home-shelf script hash',
  apiMod.includes(`'${digest}'`),
  `expected '${digest}' in src/api/mod.rs — update CONTENT_SECURITY_POLICY`,
);

// The voice engine's own CSP requirement. vosk-browser boots its worker from a
// blob whose emscripten glue evaluates a string, and a blob worker inherits the
// document's policy; 'wasm-unsafe-eval' permits WebAssembly compilation only.
// Dropping plain 'unsafe-eval' makes every Vosk model load fail with "Speech
// model failed to load" — speech input *and* the wake word go with it.
const scriptSrc = /script-src([^;]*);/.exec(apiMod)?.[1] ?? '';
check(
  "script-src allows 'wasm-unsafe-eval' (Vosk's WebAssembly core)",
  scriptSrc.includes("'wasm-unsafe-eval'"),
);
check(
  "script-src allows 'unsafe-eval' (Vosk's blob worker boots through eval)",
  scriptSrc.includes("'unsafe-eval'"),
);

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}

console.log('\nthe CSP hash matches the Browser start-page script, and the Vosk worker allowance is intact');
