/**
 * Voice-bar status: the tool → icon map (tool cards), the bar's dot grid
 * (voiceDots.js: its tool marks, its settings, its signal) and the wire that
 * feeds it — the running tool travels with the step so the bar can draw its
 * mark. Everything icon-shaped must come from the icon catalogue
 * (web/ui/icons/INDEX.md): a name that is not in it does not exist.
 */
import './helpers/i18n.mjs';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { toolIcon } from '../toolIcons.js';
import { GLYPHS, dotGlyph } from '../voiceDotGlyphs.js';

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8');

/* ── 1. Tool → icon ─────────────────────────────────────────── */

assert.equal(toolIcon('web_search'), 'ui/search');
assert.equal(toolIcon('generate_image'), 'ui/image');
assert.equal(toolIcon('generate_song'), 'ui/music');
assert.equal(toolIcon('mail_send'), 'ui/mail');
assert.equal(toolIcon('calendar_create'), 'ui/calendar');
assert.equal(toolIcon('doc_write'), 'apps/word');
assert.equal(toolIcon('studio_play'), 'apps/studio');
assert.equal(toolIcon('navigate_to'), 'ui/navigator');
assert.equal(toolIcon('image_edit'), 'ui/magic-wand', 'the specific image tools win over the family');
assert.equal(toolIcon('some_unknown_tool'), 'ui/puzzle', 'an unknown tool is still a tool');
assert.equal(toolIcon(''), null, 'no tool at all → the app mark');
assert.equal(toolIcon(null), null);

const source = read('../toolIcons.js');
const names = [...source.matchAll(/'([a-z]+\/[a-z0-9-]+)'/g)].map((match) => match[1]);
assert.ok(names.length >= 20, 'the map is populated');
const catalogue = read('../../ui/icons/INDEX.md');
for (const name of names) {
  assert.ok(catalogue.includes(name), `${name} is not in the icon catalogue`);
}

/* ── 2. The dot grid: the bar's one effect ──────────────────── */

for (const [name, rows] of Object.entries(GLYPHS)) {
  assert.equal(rows.length, 5, `${name} is a 5-row mark`);
  assert.ok(rows.every((row) => row.length === rows[0].length && /^[#.]+$/.test(row)), `${name} is a clean matrix`);
}
assert.equal(dotGlyph('web_search'), 'search');
assert.equal(dotGlyph('get_weather'), 'cloud');
assert.equal(dotGlyph('run_command'), 'term');
assert.equal(dotGlyph('file_read'), 'file');
assert.equal(dotGlyph('generate_song'), 'note');
assert.equal(dotGlyph('some_unknown_tool'), 'spark', 'an unknown tool is still a tool');
assert.equal(dotGlyph(''), null, 'no tool at all → thinking');

const sphere = read('../../css/sphere.css');
assert.ok(/#sphere-container > \.voice-dots \{[\s\S]{0,200}?inset: 0/.test(sphere), 'the dots fill the bar');
assert.ok(!/--voice-glow|box-shadow|\.voice-step/.test(sphere), 'no audio shadow, no status glyph');
assert.ok(/data-orb-style='bar'\] #sphere-container \{[\s\S]{0,400}?--voice-ink: var\(--text\)/.test(sphere),
  'the theme look draws the dots in the theme\'s text');

const dots = read('../voiceDots.js');
for (const effect of ['glyph', 'eq', 'ripple', 'firefly']) {
  assert.ok(new RegExp(`\\n  ${effect}: \\{`).test(dots), `the ${effect} effect exists`);
}
assert.ok(/'voice:state'/.test(dots) && /'voice:level'/.test(dots) && /'agent:step'/.test(dots),
  'the grid follows the voice state, the audio level and the steps');
assert.ok(/prefs\.react === 'voice'/.test(dots) && /prefs\.react === 'tools'/.test(dots),
  'what the dots react to is a setting');
assert.ok(/document\.hidden/.test(dots) && /prefers-reduced-motion/.test(dots),
  'a hidden page draws nothing, reduced motion rests');

const dock = read('../dockStep.js');
assert.ok(/detail: \{ message: text, action: String\(action \|\| ''\) \}/.test(dock),
  'every step carries its tool to the bar');

const appearance = read('../settings/appearance.js');
assert.ok(/orb_effect/.test(appearance) && /orb_react/.test(appearance) && /mountVoiceDots\(dotsPreview/.test(appearance),
  'the panel offers the effect and what it follows, with a live preview');
assert.ok(/theme: 'bar', dark: 'dark', light: 'light'/.test(appearance) && /orb_style: 'accent', orb_color: value/.test(appearance),
  'one colour control: theme, dark, light, accent or every accent');
assert.ok(/data-orb-style='dark'\] \{[\s\S]{0,200}?--voice-plate-ink: #f2f2f0/.test(sphere),
  'the dark plate draws light dots, whatever the theme');

/* ── 3. The wire: the running tool travels with the step ─────── */

assert.ok(/action: event\.action/.test(read('../agent.js')),
  'the SSE step action reaches the client');
assert.ok(/orb_effect: 'glyph'/.test(read('../preferences.js')) && /orb_react: 'both'/.test(read('../preferences.js'))
  && /orbEffect/.test(read('../preferences.js')),
  'the settings are stored (glyph, both by default) and published');
assert.ok(/action: Option<String>/.test(read('../../../src/api/agent.rs')),
  'the stream event carries the tool');

/* ── 4. The bottom edge behaves like the top one ─────────────── */

const fullscreen = read('../fullscreen.js');
assert.ok(/function bottomVoiceBarGap/.test(fullscreen)
  && (fullscreen.match(/REVEAL_ZONE \+ bottomVoiceBarGap\(cfg\)|EDGE_ZONE \+ bottomVoiceBarGap\(cfg\)/g) || []).length === 2,
  'the bottom strip covers the voice bar that sits on it');
const chrome = read('../../index.html');
assert.ok(/<header id="hud-header"[^>]*>[\s\S]{0,500}?id="fullscreen-bubble"/.test(chrome),
  'the fullscreen bubble is a child of the row, so it follows the bar to any edge');
assert.ok(/on_step\(&ui_step\(state, &action\), Some\(action\.as_str\(\)\)\)/.test(
  read('../../../src/services/agent_runner.rs')),
  'the runner passes the tool with the step');
console.log('voice-bar tool-status checks passed');
