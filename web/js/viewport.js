/**
 * viewport — the one place that answers "is this a phone-shaped screen?".
 *
 * A vertical, narrow screen is laid out as a single column of windows with the
 * workspace system standing down: the switcher, its shortcuts, the desktop
 * menus and the AI's workspace tools all assume there is room to spread
 * windows sideways, and there isn't.
 *
 * The stored arrangement is never rewritten — the mobile view is a view over
 * it — so a desktop gets its workspaces back exactly as they were.
 */

/** The narrow breakpoint the tile manager has always called "phone". */
export const PHONE_QUERY = window.matchMedia('(max-width: 640px)');

/** Vertical and narrow: a column of windows, no workspaces. */
export const MOBILE_PORTRAIT_QUERY = window.matchMedia('(orientation: portrait) and (max-width: 900px)');

/** True on a vertical, phone-like screen. */
export function isMobilePortrait() {
  return MOBILE_PORTRAIT_QUERY.matches;
}
