/**
 * clipboard.js — the central clipboard service.
 *
 * One place records everything the user copies, so that (a) the top-bar
 * Clipboard menu can list what was copied and (b) any surface in the app can
 * copy or paste without touching the platform APIs itself.
 *
 * How the pieces fit:
 *
 *   • The OS clipboard is the source of truth. Every copy writes to it (async
 *     Clipboard API, with an execCommand fallback for kiosk webviews that do
 *     not grant the permission) and then records the text here.
 *   • App windows keep their native Ctrl/Cmd+C/X/V: the service listens for
 *     the DOM copy/cut events in the capture phase and records what actually
 *     leaves, so no key handling has to be hijacked. Pasting refreshes the
 *     local mirror only — pastes are not history entries.
 *   • `mirror` holds the last known clipboard text. It is the fallback for
 *     `readClipboardText()` when the async read is refused (the terminal's
 *     Ctrl+V path).
 *   • Browser-plugin pages render in a native child view, so their Ctrl/Cmd+C
 *     never reaches this DOM. The Qt shell (crates/peakd) reports those copies
 *     through `window.__shinyClipboardCapture`, defined here.
 *
 * Window events:
 *   clipboard:changed  — the history changed (the menu re-renders)
 *   clipboard:paste    — an entry was picked in the menu: { text, focused }
 *                        (dispatched by clipboardMenu.js, consumed by surfaces
 *                        that can paste directly: terminal, browser page)
 */

import { toast } from '../ui/index.js';
import {
  isStorable, normalizeHistory, normalizeText, pushEntry, removeEntry,
} from './clipboardShared.js';
import { getClipboardHistory, setClipboardHistory } from './preferences.js';

/* ── State ────────────────────────────────────────────────────── */

/** Newest-first history entries: { text, source, at }. */
let entries = [];

/** Last clipboard text seen, whatever the direction. Read fallback. */
let mirror = '';

let installed = false;

/* ── Recording ────────────────────────────────────────────────── */

/** Publish the history to memory, storage and any open menu. */
function setHistory(next) {
  entries = next;
  setClipboardHistory(entries);
  window.dispatchEvent(new CustomEvent('clipboard:changed', {
    detail: { count: entries.length },
  }));
}

/**
 * Record a copy that already happened (a DOM copy event, the Qt shell's
 * browser report, or `copyText` itself). Oversized texts update the mirror
 * but are not stored.
 */
function recordCopy(text, source = 'app') {
  const clean = normalizeText(text);
  if (!clean) return;
  mirror = clean;
  if (!isStorable(clean)) return;
  setHistory(pushEntry(entries, { text: clean, source, at: Date.now() }));
}

/**
 * Put `text` on the OS clipboard and into the history. Every programmatic
 * copy in the app goes through here (terminal selection, menu picks, plugin
 * "Copy …" actions), so nothing can reach the clipboard without being
 * recorded.
 */
export async function copyText(text, { source = 'app' } = {}) {
  const clean = normalizeText(text);
  if (!clean) return false;
  const ok = await writeClipboard(clean);
  recordCopy(clean, source);
  if (!ok) toast('Clipboard unavailable', { type: 'error' });
  return ok;
}

/**
 * Read the current clipboard text, or null when nothing is available.
 *
 * `navigator.clipboard.readText()` needs a permission the kiosk shell grants
 * to the app origin (crates/peakd); where it is unavailable the last text
 * seen by this service is used instead, so paste still works for anything the
 * user copied inside the app.
 */
export async function readClipboardText() {
  try {
    if (navigator.clipboard?.readText) {
      const text = await navigator.clipboard.readText();
      if (typeof text === 'string') {
        // The read succeeded — an empty clipboard is "nothing", not a reason
        // to fall back to the mirror.
        mirror = text;
        return text || null;
      }
    }
  } catch (_) {
    /* Permission refused — fall through to the mirror. */
  }
  return mirror || null;
}

/** Snapshot of the recorded entries, newest first. */
export function history() {
  return entries.slice();
}

/** Drop one entry (menu's per-row remove). */
export function removeHistoryEntry(index) {
  setHistory(removeEntry(entries, index));
}

/** Forget every entry. */
export function clearHistory() {
  setHistory([]);
}

/* ── OS clipboard access ──────────────────────────────────────── */

async function writeClipboard(text) {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch (_) {
    /* Kiosk webview without the permission — try the legacy path. */
  }
  return legacyCopy(text);
}

/**
 * The pre-API copy path: a hidden textarea plus `execCommand('copy')`. It is
 * synchronous, needs no permission and is what keeps copying alive in a
 * webview that refuses the async Clipboard API.
 */
function legacyCopy(text) {
  try {
    const area = document.createElement('textarea');
    area.value = text;
    area.setAttribute('readonly', '');
    area.style.position = 'fixed';
    area.style.opacity = '0';
    document.body.appendChild(area);
    area.select();
    const ok = document.execCommand('copy');
    area.remove();
    return ok;
  } catch (_) {
    return false;
  }
}

/* ── Capture of the native copy/paste paths ───────────────────── */

/** Which surface a DOM event came from: its plugin window, or "app". */
function sourceOf(target) {
  const tile = target?.closest?.('[data-plugin]');
  return tile?.dataset?.plugin || 'app';
}

/** copy / cut (capture phase): let the browser do its thing, then record it. */
function onCopy(event) {
  const fromEvent = event.clipboardData?.getData('text/plain');
  const text = (typeof fromEvent === 'string' && fromEvent)
    ? fromEvent
    : window.getSelection?.()?.toString?.() || '';
  recordCopy(text, sourceOf(event.target));
}

/** paste (capture phase): keep the mirror fresh; pastes are not entries. */
function onPaste(event) {
  const text = event.clipboardData?.getData('text/plain');
  if (typeof text === 'string' && text) mirror = text;
}

/**
 * Install the service: load the stored history, capture the DOM clipboard
 * events, and expose the hook the Qt shell uses for Browser-page copies.
 * Called once from app.js after the user's preferences are loaded.
 */
export function installClipboard() {
  if (installed) return;
  installed = true;

  entries = normalizeHistory(getClipboardHistory());

  document.addEventListener('copy', onCopy, true);
  document.addEventListener('cut', onCopy, true);
  document.addEventListener('paste', onPaste, true);

  // The shell runs JS on the app view when Ctrl/Cmd+C copied a selection
  // inside a Browser-plugin page (crates/peakd, `on_clipboard`).
  window.__shinyClipboardCapture = (text) => recordCopy(text, 'browser');
}
