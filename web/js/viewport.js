/**
 * viewport — the one place that answers "is this a phone-shaped screen?".
 *
 * A narrow portrait or touch landscape screen uses a single column with the
 * workspace system standing down: the switcher, its shortcuts, the desktop
 * menus and the AI's workspace tools all assume there is room to spread
 * windows sideways, and there isn't.
 *
 * The stored arrangement is never rewritten — the mobile view is a view over
 * it — so a desktop gets its workspaces back exactly as they were.
 */

/** The narrow breakpoint the tile manager has always called "phone". */
export const PHONE_QUERY = window.matchMedia('(max-width: 640px), (pointer: coarse) and (max-width: 960px) and (max-height: 600px)');

/** Vertical and narrow: a column of windows, no workspaces. */
export const MOBILE_PORTRAIT_QUERY = window.matchMedia('(orientation: portrait) and (max-width: 900px)');

/** True on a vertical, phone-like screen. */
export function isMobilePortrait() {
  return MOBILE_PORTRAIT_QUERY.matches;
}

/** Phone interaction mode survives a rotation; desktop workspace state is retained. */
export const MOBILE_QUERY = window.matchMedia('(orientation: portrait) and (max-width: 900px), (pointer: coarse) and (max-width: 960px) and (max-height: 600px)');
export function isMobileScreen() { return MOBILE_QUERY.matches; }
