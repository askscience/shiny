/**
 * vendorLoader — single-flight loaders for the vendor bundles that used to sit
 * in index.html. Vosk (5.7 MB), Leaflet and marked are only needed when their
 * feature is actually used, so the boot path no longer parses them.
 *
 * A classic <script> is used instead of import() because all three libraries
 * publish globals (`Vosk`, `L`, `marked`) and are not ES modules.
 */
const loads = new Map();

function remember(key, promise) {
  loads.set(key, promise);
  promise.catch(() => loads.delete(key));
  return promise;
}

function loadScript(src) {
  if (loads.has(src)) return loads.get(src);
  return remember(src, new Promise((resolve, reject) => {
    const script = document.createElement('script');
    script.src = src;
    script.async = true;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error(`Failed to load ${src}`));
    document.head.appendChild(script);
  }));
}

function loadStyle(href) {
  if (loads.has(href)) return loads.get(href);
  return remember(href, new Promise((resolve, reject) => {
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = href;
    link.onload = () => resolve();
    link.onerror = () => reject(new Error(`Failed to load ${href}`));
    document.head.appendChild(link);
  }));
}

export async function ensureVosk() {
  if (globalThis.Vosk) return;
  await loadScript('/vendor/vosk-browser/vosk.js');
  if (!globalThis.Vosk) throw new Error('Vosk failed to initialize');
}

export async function ensureLeaflet() {
  if (globalThis.L) return;
  await loadStyle('/vendor/leaflet/leaflet.css');
  await loadScript('/vendor/leaflet/leaflet.js');
  if (!globalThis.L) throw new Error('Leaflet failed to initialize');
}

export async function ensureMarked() {
  if (globalThis.marked) return;
  await loadScript('/vendor/marked/marked.min.js');
  if (!globalThis.marked) throw new Error('marked failed to initialize');
}
