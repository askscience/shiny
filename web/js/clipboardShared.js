/**
 * clipboardShared.js — pure rules for the central clipboard history.
 *
 * The service (clipboard.js) keeps a small "recording ring" of copied texts:
 * newest first, identical texts collapse instead of piling up, and the list is
 * capped so a long session cannot grow it without bound. The rules live here
 * as plain data in, plain data out — no DOM, no imports — so the Node test
 * (web/js/tests/clipboardShared.test.mjs) can exercise them on their own, the
 * same split hudChipsShared.js uses for the top-bar chips.
 */

/** How many entries the history keeps. */
export const CLIPBOARD_LIMIT = 50;

/**
 * Longest text worth storing. A bigger copy still reaches the OS clipboard,
 * but is not recorded: a 1 MB paste has no business in a preference row.
 */
export const CLIPBOARD_MAX_CHARS = 32768;

/** Longest preview line the menu shows per entry. */
export const CLIPBOARD_PREVIEW_CHARS = 140;

/**
 * Normalize arbitrary text for the clipboard/history.
 *
 * Returns the trimmed text, or null for anything that is not a non-empty
 * string. Trimming only affects the stored entry: `copyText` puts the trimmed
 * text on the OS clipboard too, so a row can never trail invisible
 * whitespace.
 */
export function normalizeText(value) {
  if (typeof value !== 'string') return null;
  const text = value.trim();
  return text ? text : null;
}

/** True when a normalized text is small enough to be stored in the history. */
export function isStorable(text) {
  return typeof text === 'string' && text.length <= CLIPBOARD_MAX_CHARS;
}

/** One history entry from unknown input; null when unusable. */
export function normalizeEntry(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const text = normalizeText(value.text);
  if (!text || !isStorable(text)) return null;
  const at = Number(value.at);
  return {
    text,
    source: typeof value.source === 'string' && value.source ? value.source : 'app',
    at: Number.isFinite(at) && at > 0 ? at : 0,
  };
}

/**
 * Tolerant history reader: drops junk rows (hand-written preferences, rows
 * from an older shape), keeps the stored order and applies the cap. A missing
 * or malformed value therefore reads as "no history", never as a crash.
 */
export function normalizeHistory(value) {
  if (!Array.isArray(value)) return [];
  const out = [];
  for (const raw of value) {
    const entry = normalizeEntry(raw);
    if (entry) out.push(entry);
    if (out.length >= CLIPBOARD_LIMIT) break;
  }
  return out;
}

/**
 * New list with `entry` on top. An identical text already in the list is
 * removed first, so copying the same thing twice moves it to the top instead
 * of duplicating it. Returns the list unchanged when the entry is unusable.
 */
export function pushEntry(list, entry) {
  const rows = Array.isArray(list) ? list : [];
  const clean = normalizeEntry(entry);
  if (!clean) return rows.slice();
  return [clean, ...rows.filter((row) => row?.text !== clean.text)].slice(0, CLIPBOARD_LIMIT);
}

/** Drop the entry at `index` (out-of-range indices are a no-op). */
export function removeEntry(list, index) {
  if (!Array.isArray(list)) return [];
  if (!Number.isInteger(index) || index < 0 || index >= list.length) return list.slice();
  return list.filter((_, i) => i !== index);
}

/** Single-line preview for a menu row: whitespace collapsed, ellipsized. */
export function previewText(text, max = CLIPBOARD_PREVIEW_CHARS) {
  const flat = typeof text === 'string' ? text.replace(/\s+/g, ' ').trim() : '';
  if (flat.length <= max) return flat;
  return `${flat.slice(0, Math.max(1, max - 1)).trimEnd()}…`;
}

/** Human "how long ago" for the menu's meta line. Empty for a missing time. */
export function relativeTime(at, now = Date.now()) {
  const t = Number(at);
  if (!Number.isFinite(t) || t <= 0) return '';
  const seconds = Math.max(0, Math.round((Number(now) - t) / 1000));
  if (seconds < 45) return 'just now';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  if (days < 7) return `${days} d ago`;
  const date = new Date(t);
  return `${date.getDate()}/${date.getMonth() + 1}/${date.getFullYear()}`;
}
