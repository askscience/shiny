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

import { apiFetch } from './api.js';
import { pickPaths, filesFromEntries, readFileBytes } from './filePicker.js';
import { getLocale } from './i18n.js';
import { getActiveTheme, themeMode } from '../ui/theme-loader.js';

/** @type {Map<string, object>} */
const surfaces = new Map();

/**
 * The host context a web-app plugin may adapt to: the interface language and
 * the theme chosen inside PEAK'D — not the OS language or the browser's own
 * color scheme. A framed app cannot read either through `navigator.language`:
 * that follows the OS, and an iframe has no access to the shell's prefs.
 *
 * Delivered as a `shiny:context` message on every frame load and again when
 * the theme or locale changes. The same values are mirrored on the frame
 * element (`lang`, `data-shiny-*`, `color-scheme`), so an app that only reads
 * `prefers-color-scheme` still gets the shell's dark/light choice for free.
 */
function hostContext() {
  const locale = getLocale();
  return {
    type: 'shiny:context',
    version: 1,
    theme: getActiveTheme(),
    mode: themeMode(),
    locale,
    language: locale,
    dir: document.documentElement?.dir || 'ltr',
  };
}

function applyHostContext(frame, { post = false } = {}) {
  const ctx = hostContext();
  if (ctx.mode) frame.style.colorScheme = ctx.mode;
  frame.lang = ctx.locale;
  frame.dataset.shinyTheme = ctx.theme || '';
  frame.dataset.shinyMode = ctx.mode || '';
  frame.dataset.shinyLocale = ctx.locale;
  frame.dataset.shinyDir = ctx.dir;
  if (!post) return;
  try {
    frame.contentWindow?.postMessage?.(ctx, window.location.origin);
  } catch (_) {
    /* the app navigated away; its next load re-sends */
  }
}

function broadcastHostContext() {
  for (const surface of surfaces.values()) {
    if (surface.frame) applyHostContext(surface.frame, { post: true });
  }
}

for (const type of ['theme:change', 'locale:change']) {
  window.addEventListener(type, broadcastHostContext);
}

/**
 * Live AI control for a Craft app whose page exposes the vendor's documented
 * page API — `window.<app>.request(method, params)`, the same methods its
 * desktop control channel takes. The core runs a loopback proxy that speaks the
 * app CLI's control protocol (`src/plugins/craft_control.rs`); this is the page
 * half: every request the proxy forwards is executed inside the running app and
 * answered back, so the agent's tools act on the window the user sees.
 *
 * Feature-detected and app-agnostic: the global is the plugin's own name
 * (`vectorcraft` → `window.vectorcraft`), so any Craft app — present or
 * installed later — that ships the documented API gets live control with no
 * change here. A build without it keeps the headless behavior: nothing is
 * attached and the core's fallback servers stay in charge.
 */
function attachControl(name, frame, surface) {
  const api = frame.contentWindow?.[name];
  if (!api || typeof api.request !== 'function') return;
  if (surface.controlFor === frame.contentWindow) return; // already attached
  surface.control?.close();
  surface.controlFor = frame.contentWindow;

  const source = new EventSource(
    new URL(`../api/plugins/craft/control/events?app=${encodeURIComponent(name)}`, import.meta.url),
  );
  source.addEventListener('message', (event) => {
    let request;
    try {
      request = JSON.parse(event.data);
    } catch {
      return;
    }
    if (!request || typeof request.method !== 'string') return;
    const reply = { app: name, id: request.id };
    void (async () => {
      try {
        reply.result = await api.request(request.method, request.params ?? {});
        reply.ok = true;
      } catch (error) {
        reply.ok = false;
        reply.error = String(error?.message ?? error);
      }
      try {
        await apiFetch('/api/plugins/craft/control/reply', {
          method: 'POST',
          body: JSON.stringify(reply),
        });
      } catch (error) {
        console.warn(`craft control: could not answer ${name}`, error);
      }
    })();
  });
  // EventSource reconnects by itself; a dead stream is the server's business.
  surface.control = source;
}

/**
 * In-app Open… for web apps. The kiosk's web engine has no handler for a classic
 * `<input type=file>`, so clicking Open did nothing at all. For the lifetime of
 * the frame's document, its file inputs are pointed at the shell's picker
 * (`filePicker.js`): the user picks through the Files plugin, the bytes come
 * back from it, and the app receives real `File` objects on its own input with
 * `input`/`change` dispatched — exactly what its code listens for.
 *
 * App-agnostic by construction: any web app that uses a file input gets this
 * with no code of its own.
 */
function installFileInputShim(frame) {
  const realm = frame.contentWindow;
  if (!realm || realm.__shinyFileInput) return;
  const proto = realm.HTMLInputElement?.prototype;
  if (!proto) return;
  realm.__shinyFileInput = true;

  const nativeClick = proto.click;
  const nativeShow = proto.showPicker;

  const fill = async (input) => {
    const picked = await pickPaths({ accept: input.accept || '', multiple: !!input.multiple });
    if (!picked || !picked.length) return;
    const files = await filesFromEntries(realm, picked);
    const transfer = new realm.DataTransfer();
    for (const file of files) transfer.items.add(file);
    input.files = transfer.files;
    input.dispatchEvent(new realm.Event('input', { bubbles: true }));
    input.dispatchEvent(new realm.Event('change', { bubbles: true }));
  };

  const intercept = (input) => {
    void fill(input).catch((error) => {
      console.warn('web app file picker failed', error);
    });
  };

  proto.click = function (...args) {
    if (this instanceof realm.HTMLInputElement && this.type === 'file' && !this.disabled) {
      intercept(this);
      return undefined;
    }
    return nativeClick.apply(this, args);
  };
  if (typeof nativeShow === 'function') {
    proto.showPicker = function (...args) {
      if (this instanceof realm.HTMLInputElement && this.type === 'file' && !this.disabled) {
        intercept(this);
        return undefined;
      }
      return nativeShow.apply(this, args);
    };
  }
}

/**
 * Hand a file to the app when its page API can take bytes:
 * `window.<app>.openFile(name, bytes)`. This is what “Open with app” in Files
 * (and a pending request from before the window mounted) needs; a build without
 * the API simply cannot receive one yet — the same gate as live control.
 */
async function handFileToApp(name, frame, detail) {
  const api = frame.contentWindow?.[name];
  if (!detail?.path || typeof api?.openFile !== 'function') return false;
  try {
    const blob = await readFileBytes(detail.path);
    const bytes = new Uint8Array(await blob.arrayBuffer());
    const filename = detail.name || detail.path.split('/').pop() || 'file';
    return api.openFile(filename, bytes) !== false;
  } catch (error) {
    console.warn(`could not open ${detail.path} in ${name}`, error);
    return false;
  }
}

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
  // Theme/language the app should boot with, before its own first paint.
  applyHostContext(frame);
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
    frame,
    mount() { return tile; },
    // Detaching is the shell's business; the frame simply goes with the tile.
    unmount() {},
    getElement() { return tile; },
    wireEvents() {},
  };
  // Files → "Open with app" (and a request queued before this window mounted)
  // reaches the app through its page API when the build can take bytes.
  window.addEventListener('files:open', (event) => {
    if (event.detail?.plugin === name) void handFileToApp(name, frame, event.detail);
  });
  // The app's document (and its page API) is ready at `load`; a reload replaces
  // the window the bridge was talking to, so re-attach on every load.
  frame.addEventListener('load', () => {
    attachControl(name, frame, surface);
    installFileInputShim(frame);
    applyHostContext(frame, { post: true });
    const pending = window.__pendingFileOpen;
    if (pending?.plugin === name) {
      window.__pendingFileOpen = null;
      void handFileToApp(name, frame, pending);
    }
  });
  surfaces.set(name, surface);
  return surface;
}

/** The surface module for one web-app plugin (stable across calls). */
export function webAppSurface(name) {
  return ensureSurface(name);
}

/** Drop a cached surface (tests; the shell keeps surfaces for their window). */
export function forgetWebAppSurface(name) {
  surfaces.get(name)?.control?.close();
  surfaces.delete(name);
}

export default webAppSurface;
