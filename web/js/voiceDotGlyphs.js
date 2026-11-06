/**
 * voiceDotGlyphs.js — the tool marks the voice bar's glyph effect draws.
 *
 * Each mark is a 5-row dot matrix (`#` lit, `.` dark); voiceDots.js centres it
 * in the bar's grid and assembles it dot by dot while the tool runs. Tools are
 * matched by prefix, like toolIcons.js, so a plugin's whole family shares a
 * mark; an unknown tool still gets one (the spark).
 */

/* ── Tool marks for the glyph effect: 5-row dot matrices ─────── */

export const GLYPHS = {
  search: ['.###..', '#...#.', '#...#.', '.###..', '....##'],
  file: ['###.', '#.##', '#..#', '#..#', '####'],
  term: ['#....', '.#...', '..#..', '.#...', '#.###'],
  cloud: ['..##...', '.#..##.', '#.....#', '#.....#', '.#####.'],
  note: ['..##.', '..#.#', '..#..', '###..', '###..'],
  image: ['#####', '#...#', '#..##', '#.###', '#####'],
  mail: ['#####', '##.##', '#.#.#', '#...#', '#####'],
  calendar: ['.#.#.', '#####', '#...#', '#.#.#', '#####'],
  pin: ['.###.', '##.##', '.###.', '..#..', '..#..'],
  gear: ['.#.#.', '#####', '##.##', '#####', '.#.#.'],
  screen: ['#####', '#...#', '#####', '..#..', '.###.'],
  play: ['#...', '##..', '###.', '##..', '#...'],
  spark: ['..#..', '.###.', '#####', '.###.', '..#..'],
};

/** Longest-prefix-first, like toolIcons.js: a plugin's family shares a mark. */
const GLYPH_BY_PREFIX = [
  ['web_search', 'search'], ['map_search', 'search'], ['search', 'search'],
  ['generate_image', 'image'], ['image_', 'image'],
  ['generate_song', 'note'], ['music_', 'note'], ['radio_', 'note'], ['studio_', 'note'],
  ['youtube_', 'play'], ['play_', 'play'],
  ['mail_', 'mail'], ['calendar_', 'calendar'],
  ['weather', 'cloud'], ['get_weather', 'cloud'],
  ['navigate', 'pin'], ['route_', 'pin'], ['map_', 'pin'], ['trip_', 'pin'], ['plan_trip', 'pin'],
  ['create_trip', 'pin'], ['list_trips', 'pin'], ['get_trip', 'pin'], ['get_active_trip', 'pin'],
  ['list_locations', 'pin'], ['list_tracks', 'note'],
  ['run_', 'term'], ['shell', 'term'], ['terminal', 'term'], ['exec', 'term'], ['command', 'term'],
  ['file_', 'file'], ['read_file', 'file'], ['write_file', 'file'], ['doc_', 'file'], ['pdf_', 'file'],
  ['slide_', 'file'], ['calc_', 'file'], ['diary_', 'file'], ['generate_diary', 'file'],
  ['get_diary', 'file'], ['list_diary', 'file'], ['read_', 'file'], ['write_', 'file'],
  ['settings', 'gear'], ['theme_', 'gear'], ['icon_', 'gear'],
  ['desktop_', 'screen'], ['window_', 'screen'], ['workspace_', 'screen'], ['show_plugin', 'screen'],
];

/** The dot-matrix mark for a tool action (an unknown tool still gets one). */
export function dotGlyph(action) {
  const key = String(action || '').trim().toLowerCase();
  if (!key) return null;
  for (const [prefix, glyph] of GLYPH_BY_PREFIX) {
    if (key.startsWith(prefix)) return glyph;
  }
  return 'spark';
}
