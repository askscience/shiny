/**
 * files.js — desktop-style saving into the user's Shiny home folders.
 *
 * Exporting used to trigger a browser download. On this desktop, artifacts
 * land in the user's home folders (Documents / Pictures / Music / …) through
 * the Files plugin, so apps behave like a real desktop. If the Files plugin
 * isn't installed the upload route is absent and the helper falls back to an
 * ordinary browser download, so nothing breaks.
 */
import { apiFetch } from './api.js';
import { toast } from '../ui/index.js';

/** Sensible default folder per content kind. */
export const FILES_FOLDER = {
  document: 'Documents',
  spreadsheet: 'Documents',
  presentation: 'Documents',
  pdf: 'Documents',
  image: 'Pictures',
  audio: 'Music',
  video: 'Videos',
  archive: 'Downloads',
  download: 'Downloads',
};

/** Keep a name usable as a single path component. */
export function safeFileName(name, fallback = 'file') {
  const base = String(name ?? '')
    .replace(/[\\/]+/g, '-')
    .replace(/[\u0000-\u001f]/g, ' ')
    .trim();
  return base || fallback;
}

/**
 * Write a Blob into the user's home via the Files plugin.
 * Resolves `{ ok: true, path, name, dir }` or `{ ok: false, error }`.
 */
export async function saveToFiles(blob, { name, dir = FILES_FOLDER.document } = {}) {
  const filename = safeFileName(name);
  const qs = new URLSearchParams({ path: dir, name: filename });
  try {
    const res = await apiFetch(`/api/files/upload?${qs.toString()}`, { method: 'POST', body: blob });
    const path = res?.data?.path || (dir ? `${dir}/${filename}` : filename);
    window.dispatchEvent(new CustomEvent('files:changed', { detail: { dir, path } }));
    return { ok: true, path, name: filename, dir };
  } catch (error) {
    return { ok: false, error };
  }
}

/** Classic browser download (the fallback when Files is unavailable). */
export function downloadBlob(blob, name) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = safeFileName(name);
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 5000);
  return url;
}

/**
 * Desktop-style save: write into the home folder, notify, and reveal in the
 * Files window. Falls back to a browser download when the Files plugin is not
 * installed. Returns the saved relative path, or null when it downloaded.
 *
 *   await saveOrDownload(blob, { name: 'notes.odt', dir: 'Documents', app: 'Word' });
 */
export async function saveOrDownload(blob, { name, dir = FILES_FOLDER.document, app = null } = {}) {
  const filename = safeFileName(name);
  const res = await saveToFiles(blob, { name: filename, dir });
  if (res.ok) {
    toast(app ? `${app} saved to ${res.path}` : `Saved to ${res.path}`, { type: 'ok' });
    return res.path;
  }
  downloadBlob(blob, filename);
  return null;
}

/**
 * Open the OS file picker and resolve with the chosen files (empty on cancel).
 *
 * Why this exists: WebKit — Safari and the native WKWebView shell — only opens
 * the panel for a file input that is **connected and rendered**. A detached
 * input, or one hidden with `display:none`, is silently ignored when
 * `.click()` is called; Chrome tolerates both. That is why the same
 * `input.click()` "works in other browsers" but does nothing in the native
 * browser, and it is not a macOS permission (the shell is not sandboxed and
 * needs no usage string for the open panel to appear).
 *
 * The input is therefore appended just off-screen (still laid out, not
 * `display:none`) and removed once the dialog closes.
 */
export function pickFiles({ accept = '', multiple = false } = {}) {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    if (accept) input.accept = accept;
    input.multiple = multiple;
    input.setAttribute('aria-hidden', 'true');
    input.tabIndex = -1;
    Object.assign(input.style, {
      position: 'fixed',
      left: '0',
      top: '0',
      width: '1px',
      height: '1px',
      opacity: '0',
      pointerEvents: 'none',
      zIndex: '-1',
    });

    let settled = false;
    const settle = () => {
      if (settled) return;
      settled = true;
      const files = [...(input.files || [])];
      input.remove();
      resolve(files);
    };

    input.addEventListener('change', settle);
    // Safari/WebKit ≥ 16.4 cancels the picker without a `change`; fall back to
    // the window regaining focus when `cancel` is not supported.
    if ('oncancel' in input) {
      input.addEventListener('cancel', settle);
    } else {
      window.addEventListener('focus', () => setTimeout(settle, 300), { once: true });
    }

    document.body.appendChild(input);
    input.click();
  });
}

/** Focus the Files window, optionally at a folder. */
export function revealInFiles(dir = '') {
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: 'files', path: dir } }));
}

/* ── Open a file with the right app (desktop "double-click") ─────────────── */

/** Extension → plugin that opens it. (Audio/WAV is intentionally absent.) */
export const FILE_OPENERS = {
  odt: 'word',
  ods: 'calc',
  odp: 'impress',
  pdf: 'pdf',
  png: 'image',
  jpg: 'image',
  jpeg: 'image',
  gif: 'image',
  webp: 'image',
  bmp: 'image',
};

/** The plugin name that can open `name`, or null when none does. */
export function pluginForFile(name) {
  const ext = String(name || '').toLowerCase().split('.').pop();
  return FILE_OPENERS[ext] || null;
}

/**
 * Ask the app that owns this file type to open it. Activates the plugin (its
 * routes exist as soon as it's installed), wakes the desktop so its window
 * mounts, then hands it the file. If the app is already mounted it reacts to
 * `files:open`; if it mounts a moment later it drains `window.__pendingFileOpen`.
 */
export async function openWithPlugin({ plugin, path, name }) {
  if (!plugin || !path) return false;
  try {
    await apiFetch('/api/plugins/activate', { method: 'POST', body: JSON.stringify({ name: plugin }) });
  } catch (_) {
    // Not installed (or activation failed) — the caller can fall back to preview.
    return false;
  }
  const detail = { plugin, path, name };
  window.__pendingFileOpen = { ...detail, ts: Date.now() };
  window.dispatchEvent(new CustomEvent('plugins:changed', { detail: { opened: plugin } }));
  window.dispatchEvent(new CustomEvent('files:open', { detail }));
  return true;
}

/**
 * Register an app as a file opener. Call once from the surface's `wireEvents`.
 * Drains a queued open (when the window wasn't mounted yet) and handles later
 * ones.
 */
export function onOpenFromFiles(plugin, handler) {
  const handle = (detail) => {
    if (!detail || detail.plugin !== plugin) return;
    const pending = window.__pendingFileOpen;
    if (pending && pending.plugin === plugin) window.__pendingFileOpen = null;
    try {
      Promise.resolve(handler(detail)).catch(() => {});
    } catch (_) { /* an opener must never break the caller */ }
  };
  window.addEventListener('files:open', (e) => handle(e.detail));
  const pending = window.__pendingFileOpen;
  if (pending && pending.plugin === plugin) handle(pending);
}

/** Fetch a home file's bytes and wrap them as a `File` for an import route. */
export async function fileFromHome(path, name, type = 'application/octet-stream') {
  const blob = await apiFetch(`/api/files/raw?path=${encodeURIComponent(path)}`, { responseType: 'blob' });
  return new File([blob], safeFileName(name, 'file'), { type });
}
