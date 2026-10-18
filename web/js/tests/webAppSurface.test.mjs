/**
 * Web-app plugin window surface test.
 *
 * A plugin with `kind = "web"` in its `plugin.toml` ships no `web/plugin.js`:
 * its payload is a static site the core serves at `/plugins/<name>/`, mounted
 * in a tile as an iframe. This file pins the contract the tile manager
 * consumes (`mount` / `unmount` / `getElement` / `wireEvents`), the iframe's
 * URL and capabilities, and two deliberate choices:
 *
 *   • no `sandbox` attribute — sandboxing same-origin content is a no-op
 *     unless `allow-same-origin` is dropped, and without it the app loses the
 *     storage (IndexedDB/OPFS) that holds the user's files;
 *   • the same element comes back on every `mount()`, because detaching and
 *     re-adding a tile reloads any iframe inside it (tiles.js reconciles the
 *     tile grid in place for exactly that reason).
 *
 * The repo's JS tests have no DOM, so this file brings a stub of document —
 * enough for the surface builder, which touches nothing else.
 *
 * Run:  node web/js/tests/webAppSurface.test.mjs
 */

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

/* ── A document, just enough of one ──────────────────────────── */

const created = [];

globalThis.document = {
  createElement(tag) {
    const el = {
      tagName: String(tag).toUpperCase(),
      className: '',
      dataset: {},
      attributes: {},
      children: [],
      setAttribute(name, value) { this.attributes[name] = String(value); },
      getAttribute(name) { return Object.hasOwn(this.attributes, name) ? this.attributes[name] : null; },
      appendChild(child) { this.children.push(child); return child; },
    };
    created.push(el);
    return el;
  },
};

const { webAppSurface, forgetWebAppSurface } = await import('../webAppSurface.js');
const defaultExport = (await import('../webAppSurface.js')).default;

/* ── The surface contract ────────────────────────────────────── */

check('default export is the factory', defaultExport === webAppSurface);

const surface = webAppSurface('photocraft');
check('surface carries the plugin name', surface.name === 'photocraft');
check('surface exposes the window contract',
  typeof surface.mount === 'function'
  && typeof surface.unmount === 'function'
  && typeof surface.getElement === 'function'
  && typeof surface.wireEvents === 'function',
  JSON.stringify(Object.keys(surface).sort()));

const tile = surface.mount();
check('mount returns a section with the plugin identity',
  tile.tagName === 'SECTION' && tile.dataset.plugin === 'photocraft'
  && tile.className.split(' ').includes('tile'), JSON.stringify(tile.className));
check('the tile is marked as a web-app window', tile.className.includes('tile--webapp'));
check('getElement returns the mounted element', surface.getElement() === tile);
check('wireEvents is safe to call', surface.wireEvents() === undefined);
check('unmount keeps the element (the shell detaches it)', surface.unmount() === undefined
  && surface.getElement() === tile);

/* ── The iframe ──────────────────────────────────────────────── */

const frames = created.filter((el) => el.tagName === 'IFRAME');
check('exactly one iframe per surface', frames.length === 1, `got ${frames.length}`);
const frame = frames[0];

check('the iframe points at the plugin document root',
  /^file:\/\/.*\/plugins\/photocraft\/$/.test(frame.src), frame.src);
check('the iframe is styled and identified',
  frame.className === 'webapp-frame' && frame.dataset.plugin === 'photocraft');
check('the frame is allowed the capabilities the apps ask for',
  ['fullscreen', 'clipboard-read', 'clipboard-write', 'autoplay']
    .every((token) => frame.attributes.allow.includes(token)),
  frame.attributes.allow);
check('the frame may go fullscreen', frame.attributes.allowfullscreen === '');
check('no sandbox attribute: a same-origin app keeps its storage',
  frame.getAttribute('sandbox') === null);
check('the iframe lives inside the tile', tile.children.includes(frame));

/* ── Stability across re-renders ─────────────────────────────── */

check('the same surface object comes back', webAppSurface('photocraft') === surface);
check('re-mounting reuses the same element (no iframe reload)',
  webAppSurface('photocraft').mount() === tile);
check('another plugin gets its own surface',
  webAppSurface('filmcraft') !== surface
  && webAppSurface('filmcraft').mount() !== tile);

forgetWebAppSurface('photocraft');
const rebuilt = webAppSurface('photocraft');
check('forgetting a surface rebuilds it', rebuilt !== surface && rebuilt.mount() !== tile);

console.log(failures === 0
  ? '\nwebAppSurface: all checks passed'
  : `\nwebAppSurface: ${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
