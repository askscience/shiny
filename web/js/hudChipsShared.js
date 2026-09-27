/**
 * hudChipsShared.js — pure helpers for the top-bar host status chips (sound,
 * network, Bluetooth). Each chip is a button made of an icon, a device label
 * and a numeric percentage; this module decides which of the two text parts
 * the user has chosen to show. Hiding both leaves an icon-only button.
 *
 * Import-free on purpose: `web/js/tests/hudChipsShared.test.mjs` loads it in
 * Node with no DOM.
 */

/** The two optional parts of a chip. */
export const HUD_CHIP_PARTS = ['name', 'percent'];

/** Percentages visible, device names hidden — the compact default: icon + %. */
export const DEFAULT_HUD_CHIPS = { name: false, percent: true };

/**
 * Normalize a stored value into `{ name, percent }`. Device names are hidden
 * unless explicitly turned on; percentages are shown unless explicitly turned
 * off. A missing or malformed value therefore falls back to the default
 * (icon + %), never to an unexpected icon-only chip.
 */
export function normalizeHudChips(value) {
  const src = value && typeof value === 'object' && !Array.isArray(value) ? value : {};
  return {
    name: src.name === true,
    percent: src.percent !== false,
  };
}

/** True when the chip shows nothing but its icon. */
export function isIconOnly(chips) {
  const c = normalizeHudChips(chips);
  return !c.name && !c.percent;
}

/**
 * The data attributes written onto <html>; the stylesheet hides the matching
 * label / percentage spans. Keys are dataset-style (`hudName` -> data-hud-name).
 */
export function hudChipsDataset(chips) {
  const c = normalizeHudChips(chips);
  return {
    hudName: c.name ? '1' : '0',
    hudPercent: c.percent ? '1' : '0',
  };
}
