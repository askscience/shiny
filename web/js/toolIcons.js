/**
 * toolIcons.js — which catalogue icon stands for which tool.
 *
 * Settings → Appearance → "Tool status: icons" makes the voice bar wear the
 * running tool's icon instead of the status line (dockStep.js renders it,
 * sphere.css animates it). Names come from the icon catalogue
 * (web/ui/icons/INDEX.md) and are matched by prefix, so a plugin's whole family
 * shares its mark (`mail_send`, `mail_read` → ui/mail).
 *
 * `null` means "no tool": the bar then wears the app's own mark, which is what
 * thinking (and a turn before its first tool) looks like.
 */

/** Longest-prefix wins, so the specific names come before the general ones. */
const BY_PREFIX = [
  ['web_search', 'ui/search'],
  ['generate_image', 'ui/image'],
  ['image_edit', 'ui/magic-wand'],
  ['image_', 'ui/image'],
  ['generate_song', 'ui/music'],
  ['show_artifact', 'ui/layers'],
  ['update_artifact', 'ui/layers'],
  ['show_plugin', 'ui/puzzle'],
  ['plugin_', 'ui/puzzle'],
  ['list_plugins', 'ui/puzzle'],
  ['desktop_', 'ui/monitor'],
  ['window_', 'ui/monitor'],
  ['workspace_', 'ui/monitor'],
  ['focus_', 'ui/eye'],
  ['settings', 'ui/settings'],
  ['theme_', 'ui/palette'],
  ['icon_', 'ui/palette'],
  ['mail_', 'ui/mail'],
  ['calendar_', 'ui/calendar'],
  ['doc_', 'apps/word'],
  ['slide_', 'apps/impress'],
  ['calc_', 'apps/calc'],
  ['calculator_', 'ui/calculator'],
  ['file_', 'ui/folder'],
  ['pdf_', 'apps/pdf'],
  ['studio_', 'apps/studio'],
  ['radio_', 'apps/radio'],
  ['youtube_', 'apps/youtube'],
  ['music_', 'ui/music'],
  ['play_', 'ui/play'],
  ['navigate', 'ui/navigator'],
  ['route_', 'ui/navigator'],
  ['map_search', 'ui/search'],
  ['map_', 'ui/pin'],
  ['plan_trip', 'ui/pin'],
  ['create_trip', 'ui/pin'],
  ['list_trips', 'ui/pin'],
  ['trip_', 'ui/pin'],
  ['generate_diary', 'ui/doc'],
  ['diary_', 'ui/doc'],
  ['search', 'ui/search'],
  ['hello', 'ui/message-circle'],
];

/** A tool we have no mark for is still a tool: the plugins mark. */
const FALLBACK = 'ui/puzzle';

/** The catalogue icon for a tool action, or null for the app's own mark. */
export function toolIcon(action) {
  const key = String(action || '').trim().toLowerCase();
  if (!key) return null;
  for (const [prefix, icon] of BY_PREFIX) {
    if (key.startsWith(prefix)) return icon;
  }
  return FALLBACK;
}
