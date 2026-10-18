/**
 * webAppSurface.js — window surface for **web-app plugins** (`kind = "web"`).
 *
 * A web-app plugin ships no `web/plugin.js`: its payload is a static site
 * (usually a wasm build of a desktop app) served at `/plugins/<name>/`. The
 * shell mounts it as an iframe filling the tile, and the core serves that
 * route with framing allowed (`X-Frame-Options: SAMEORIGIN`) and a policy that
 * admits the app's own inline boot script and wasm — see
 * `WEB_APP_CONTENT_SECURITY_POLICY` in `src/api/mod.rs`.
 *
 * Why a module at all: `tiles.js` drives every window through the surface
 * contract (`mount`/`unmount`/`getElement`/`wireEvents`). Wrapping the iframe
 * in that contract means the window chrome, focus, workspaces, the tray and
 * the launcher all treat a web app exactly like any other plugin window.
 *
 * The element is created once per plugin and reused: detaching and re-adding a
 * tile reloads any iframe inside it (the same reason `tiles.js` reconciles the
 * tile grid in place), and reloading an app would throw away everything the
 * user has open in it.
 */

/** @type {Map<string, object>} */
const surfaces = new Map();

function ensureSurface(name) {
  let surface = surfaces.get(name);
  if (surface) return surface;

  const tile = document.createElement('section');
  tile.className = 'tile tile--webapp';
  tile.dataset.plugin = name;

  const frame = document.createElement('iframe');
  frame.className = 'webapp-frame';
  frame.dataset.plugin = name;
  // Relative on purpose: a module specifier resolves against the *module's*
  // URL, and the app can be served through the filtering proxy (the kiosk),
  // where a root-absolute `/plugins/…` would leave the app's origin — the same
  // note `tiles.js` carries for plugin module imports.
  frame.src = new URL(`../plugins/${name}/`, import.meta.url).href;
  frame.title = name;
  // Capabilities the apps ask their frame for: their own fullscreen mode,
  // copy/paste through the clipboard, and media playback.
  frame.setAttribute('allow', 'fullscreen; clipboard-read; clipboard-write; autoplay');
  frame.setAttribute('allowfullscreen', '');
  // Deliberately no `sandbox` attribute: sandboxing same-origin content is a
  // no-op unless `allow-same-origin` is dropped, and without it the app loses
  // its storage (IndexedDB/OPFS hold the user's files). These apps are
  // installed from their vendor's release and SHA-256-verified, the same trust
  // class as a plugin cdylib.
  tile.appendChild(frame);

  surface = {
    name,
    mount() { return tile; },
    // Detaching is the shell's business; the frame simply goes with the tile.
    unmount() {},
    getElement() { return tile; },
    wireEvents() {},
  };
  surfaces.set(name, surface);
  return surface;
}

/** The surface module for one web-app plugin (stable across calls). */
export function webAppSurface(name) {
  return ensureSurface(name);
}

/** Drop a cached surface (tests; the shell keeps surfaces for their window). */
export function forgetWebAppSurface(name) {
  surfaces.delete(name);
}

export default webAppSurface;
