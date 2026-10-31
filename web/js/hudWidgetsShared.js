/**
 * hudWidgetsShared.js — pure helpers for the clock and weather chips in the
 * top bar. Each chip carries one optional text part beside its core: the
 * clock's date and the weather's description. This module decides which ones
 * the user has chosen to show.
 *
 * Import-free on purpose: `web/js/tests/hudWidgetsShared.test.mjs` loads it in
 * Node with no DOM.
 */

/** The two optional text parts of the core top-bar chips. */
export const HUD_WIDGET_PARTS = ['clock_date', 'weather_text'];

/** Both text parts shown — the stock top bar. */
export const DEFAULT_HUD_WIDGETS = { clock_date: true, weather_text: true };

/**
 * Normalize a stored value into `{ clock_date, weather_text }`. Both are shown
 * unless explicitly turned off, so a missing or malformed value keeps the
 * stock top bar instead of hiding text nobody asked to hide.
 */
export function normalizeHudWidgets(value) {
  const src = value && typeof value === 'object' && !Array.isArray(value) ? value : {};
  return {
    clock_date: src.clock_date !== false,
    weather_text: src.weather_text !== false,
  };
}

/**
 * The data attributes written onto <html>; the stylesheet hides the matching
 * date / description spans. Keys are dataset-style (`hudClockDate` ->
 * data-hud-clock-date).
 */
export function hudWidgetsDataset(widgets) {
  const w = normalizeHudWidgets(widgets);
  return {
    hudClockDate: w.clock_date ? '1' : '0',
    hudWeatherText: w.weather_text ? '1' : '0',
  };
}
