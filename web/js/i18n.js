/**
 * i18n.js — the UI translation runtime (no build step, no dependencies).
 *
 * Catalogs are flat JSON objects in `web/i18n/<code>.json`:
 *
 *   { "login.signIn": "Sign In",
 *     "settings.remote.connections.one": "{count} connection",
 *     "settings.remote.connections.other": "{count} connections" }
 *
 * Keys are dotted, grouped by surface. Placeholders are `{name}` and are
 * filled from the params object; `tp()` picks the plural form with
 * `Intl.PluralRules` (keys `….one`, `….other`, `….few`, …).
 *
 * English (`en.json`) is the reference catalog AND the fallback: a key missing
 * from the active language renders its English string; a key missing
 * everywhere renders the caller's fallback (plugin strings pass their English
 * inline). Static markup in index.html keeps its English text as the last
 * resort and only carries the `data-i18n*` keys, so a catalog that fails to
 * load leaves the UI in English rather than blank.
 *
 * Plugin windows ship `i18n/<code>.json` next to their `plugin.js` and call
 * `loadPluginCatalog(import.meta.url)`; their keys are fully qualified
 * (`plugin.<name>.…`), so one lookup table serves the whole app.
 *
 * Locale resolution: the stored `ui.locale` preference (per traveller, with an
 * unscoped mirror for the pre-paint script), else the browser language, else
 * English. A change dispatches `locale:change` — the same CustomEvent idiom as
 * `appearance:change` / `theme:change`.
 */

const DEFAULT_LOCALE = 'en';
const STORAGE_KEY = 'ui.locale';
const RTL_LANGUAGES = new Set(['ar', 'he', 'fa', 'ur', 'yi', 'dv', 'ps', 'sd', 'ug']);

let locale = DEFAULT_LOCALE;   // the code actually in use
let preference = '';           // explicit choice; '' = follow the browser
let index = null;              // { locales: [{ code, name, dir? }] }
let plural = null;             // Intl.PluralRules for `locale`
const tables = new Map();      // code -> Map<key, message>
const warned = new Set();      // keys already reported as missing

/* ── storage ────────────────────────────────────────────────────
 * i18n.js imports nothing on purpose: `app.js` awaits initI18n() before
 * anything else can render text, and early-i18n.js reads the same key before
 * first paint, so this module must not drag the shell graph in. The scoped-key
 * rule therefore mirrors preferences.js's scopedKey().
 * ─────────────────────────────────────────────────────────────── */

function scopedKey() {
  let id = null;
  try {
    id = JSON.parse(localStorage.getItem('traveler') || 'null')?.id || null;
  } catch (_) {
    /* signed out / corrupt value */
  }
  return id ? `${STORAGE_KEY}.${id}` : STORAGE_KEY;
}

/** The explicit interface language, or '' when following the browser. */
export function getLocalePreference() {
  try {
    return localStorage.getItem(scopedKey()) || '';
  } catch (_) {
    return '';
  }
}

/** The language in use right now (resolved catalog code). */
export function getLocale() {
  return locale;
}

/** The locales with a shipped catalog, for the Settings picker. */
export function getAvailableLocales() {
  return index?.locales?.filter((l) => l && typeof l.code === 'string') || [];
}

function isRtl(code) {
  return RTL_LANGUAGES.has(String(code || '').toLowerCase().split('-')[0]);
}

/* ── catalog loading ────────────────────────────────────────────
 * A missing catalog file is NOT a 404: ServeDir's fallback serves index.html
 * with HTTP 200 (src/api/mod.rs). Parsing — not the status — is the check.
 * ─────────────────────────────────────────────────────────────── */

async function fetchJson(url) {
  try {
    const res = await fetch(url);
    if (!res.ok) return null;
    const data = JSON.parse(await res.text());
    if (!data || typeof data !== 'object' || Array.isArray(data)) return null;
    return data;
  } catch (_) {
    return null;
  }
}

function mergeInto(code, data) {
  let table = tables.get(code);
  if (!table) {
    table = new Map();
    tables.set(code, table);
  }
  for (const [key, value] of Object.entries(data)) {
    if (typeof value === 'string' && key && key[0] !== '_') table.set(key, value);
  }
}

async function loadLocale(code) {
  if (!code) return;
  if (!tables.has(DEFAULT_LOCALE)) {
    const en = await fetchJson(`/i18n/${DEFAULT_LOCALE}.json`);
    if (en) mergeInto(DEFAULT_LOCALE, en);
  }
  if (code !== DEFAULT_LOCALE && !tables.has(code)) {
    const data = await fetchJson(`/i18n/${code}.json`);
    if (data) mergeInto(code, data);
  }
}

/** Best catalog match for a requested language code ('' when none matches). */
function pickLocale(requested) {
  const raw = String(requested || '').trim().toLowerCase();
  if (!raw) return '';
  const codes = getAvailableLocales().map((l) => l.code.toLowerCase());
  if (!codes.length) return raw.split('-')[0];
  if (codes.includes(raw)) return codes[codes.indexOf(raw)];
  const base = raw.split('-')[0];
  if (codes.includes(base)) return base;
  return codes.find((c) => c.split('-')[0] === base) || '';
}

/** The code a request resolves to, falling back to English. */
function targetLocale(requested) {
  return pickLocale(requested) || (pickLocale(navigator.language || '') || DEFAULT_LOCALE);
}

/* ── applying ───────────────────────────────────────────────── */

function applyDocumentLocale() {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  root.lang = locale;
  root.setAttribute('data-locale', locale);
  root.dir = isRtl(locale) ? 'rtl' : 'ltr';
}

/**
 * Apply the stored preference (re-reading it first) and re-render. Called at
 * boot and after `loadUserPreferences()` copies the server values in.
 */
export async function refreshLocaleFromStorage() {
  const stored = getLocalePreference();
  if (stored === preference && tables.has(locale)) return locale;
  preference = stored;
  return applyLocale(targetLocale(preference || navigator.language || ''), false);
}

async function applyLocale(code, announce) {
  await loadLocale(code);
  const changed = code !== locale;
  locale = code;
  plural = null;
  applyDocumentLocale();
  localize(document);
  if (changed || announce) {
    window.dispatchEvent(new CustomEvent('locale:change', { detail: { locale } }));
  }
  return locale;
}

/**
 * Persist an interface language. `code` empty/null goes back to "automatic"
 * (the browser language). The choice is mirrored to the unscoped key because
 * early-i18n.js has to tag the document before any module runs.
 */
export async function setLocalePreference(code) {
  const value = String(code || '').trim().toLowerCase();
  preference = value;
  try {
    if (value) localStorage.setItem(scopedKey(), value);
    else localStorage.removeItem(scopedKey());
    if (value) localStorage.setItem(STORAGE_KEY, value);
    else localStorage.removeItem(STORAGE_KEY);
  } catch (_) {
    /* private mode / storage disabled */
  }
  return applyLocale(value ? targetLocale(value) : targetLocale(navigator.language || ''), true);
}

/** Load the index + catalogs and localize the static markup. */
export async function initI18n() {
  index = await fetchJson('/i18n/index.json');
  if (!Array.isArray(index?.locales)) index = { locales: [{ code: DEFAULT_LOCALE, name: 'English' }] };
  preference = getLocalePreference();
  await applyLocale(targetLocale(preference || navigator.language || ''), false);
  return locale;
}

/** Fetch a plugin's `i18n/<code>.json` relative to its module URL. */
export async function loadPluginCatalog(moduleUrl) {
  return loadPluginCatalogFromUrl(new URL(`./i18n/${locale}.json`, moduleUrl).href);
}

/** Same, for a plugin whose module URL is not in hand (e.g. the Plugins list). */
export async function loadPluginCatalogFromUrl(url) {
  const data = await fetchJson(url);
  if (data) mergeInto(locale, data);
  return Boolean(data);
}

/* ── lookup ─────────────────────────────────────────────────── */

function lookup(key) {
  const value = tables.get(locale)?.get(key);
  if (value !== undefined) return value;
  const fallback = tables.get(DEFAULT_LOCALE)?.get(key);
  return fallback !== undefined ? fallback : null;
}

function interpolate(message, params) {
  if (!params) return message;
  return message.replace(/\{(\w+)\}/g, (match, name) => (
    params[name] === undefined || params[name] === null ? match : String(params[name])
  ));
}

function warnMissing(key) {
  if (warned.has(key)) return;
  warned.add(key);
  console.warn(`i18n: no message for "${key}" (${locale})`);
}

/**
 * Translate `key`. Falls back to the English catalog, then to `fallback`, then
 * to the key itself.
 */
export function t(key, params = null, fallback = null) {
  const message = lookup(key);
  if (message === null) {
    if (fallback === null) warnMissing(key);
    return fallback === null ? key : fallback;
  }
  return interpolate(message, params);
}

/**
 * Plural form of `key` for `count`: looks up `<key>.<category>` (CLDR
 * categories via Intl.PluralRules), then `<key>.other`, then `fallback`.
 * `count` is added to the params and formatted for the active language.
 */
export function tp(key, count, params = null, fallback = null) {
  const n = Number(count) || 0;
  plural = plural || new Intl.PluralRules(locale);
  const message = lookup(`${key}.${plural.select(n)}`) ?? lookup(`${key}.other`);
  if (message === null) {
    if (fallback === null) warnMissing(key);
    return fallback === null ? key : fallback;
  }
  return interpolate(message, { count: formatNumber(n), ...(params || {}) });
}

/** Format a number for the active language. */
export function formatNumber(value, options = undefined) {
  return new Intl.NumberFormat(locale, options).format(value);
}

/* ── static markup ──────────────────────────────────────────────
 * index.html carries English text plus a key; localize() replaces it wherever
 * a message exists. Unknown keys leave the English text (and attributes)
 * untouched — the safe direction during and after a partial translation.
 * ─────────────────────────────────────────────────────────────── */

export const LOCALIZE_ATTRIBUTES = [
  ['data-i18n', null],                    // textContent
  ['data-i18n-title', 'title'],
  ['data-i18n-placeholder', 'placeholder'],
  ['data-i18n-aria-label', 'aria-label'],
];

export function localize(root = document) {
  if (!root || typeof root.querySelectorAll !== 'function') return;
  for (const [attribute, target] of LOCALIZE_ATTRIBUTES) {
    for (const el of root.querySelectorAll(`[${attribute}]`)) {
      const message = lookup(el.getAttribute(attribute));
      if (message === null) continue;
      if (target) el.setAttribute(target, message);
      else el.textContent = message;
    }
  }
}
