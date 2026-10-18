/**
 * Voice-bar tool status: the tool → icon map, the glyph contract (one span, the
 * swap glitch) and the wire that feeds it — the running tool travels with the
 * step so the bar can wear its mark. Everything icon-shaped must come from the
 * icon catalogue (web/ui/icons/INDEX.md): a name that is not in it does not
 * exist.
 */
import './helpers/i18n.mjs';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { toolIcon } from '../toolIcons.js';

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

/* ── 2. The glyph: one span, text or icon, swapped with a fade ── */

const sphere = read('../../css/sphere.css');
assert.ok(/html\[data-orb-status='icons'\] \.voice-step/.test(sphere), 'the icon mode sizes the glyph');
assert.ok(/html\[data-orb-status='text'\] \.voice-step/.test(sphere) === false
  && /html\[data-orb-status='text'\] \{ --voice-slot/.test(sphere), 'a step line gets the wide slot');
assert.ok(/html\[data-orb-status='off'\] \.voice-step \{ display: none; \}/.test(sphere));
assert.ok(/@keyframes voiceGlyphFade/.test(sphere) && /\.voice-step\.is-fading/.test(sphere),
  'the swap fades');
assert.ok(!/Glitch/i.test(sphere), 'the glitch is gone');

/* The shadow is the audio effect alone: every offset, blur and spread rides the
   level, so a resting bar casts nothing and only the voice makes it glow. */
assert.ok(/--voice-glow:/.test(sphere) && /var\(--voice-glow\)/.test(sphere));
const glow = /--voice-glow:([\s\S]*?);/.exec(sphere)[1];
assert.ok(!/\+ \d/.test(glow), 'no additive constants — a resting bar casts nothing');
assert.ok((glow.match(/var\(--voice-level\)/g) || []).length >= 6,
  'the level scales the whole shadow');

/* The top-bar look: another plate, the audio glow kept on top of it. */
assert.ok(/data-orb-style='bar'\] #sphere-container \{[\s\S]{0,400}?background: var\(--surface\)/.test(sphere));
assert.ok(!/data-orb-style='bar'\] #sphere-container \{[\s\S]{0,400}?box-shadow:/.test(sphere),
  'the look never replaces the audio glow (a `none` glass shadow would drop it)');

const dock = read('../dockStep.js');
assert.ok(/paintBarGlyph[\s\S]{0,600}?paintGlyph\(/.test(dock) && /action = ''/.test(dock),
  'the bar repaints per step, with the tool');
assert.ok(/mode === 'off' \|\| \(mode === 'text' && !lastText\)/.test(dock),
  'the icon mode always carries a mark — the app\'s own at rest');
assert.ok(/desktop:immersive[\s\S]{0,80}?paintBarGlyph/.test(dock),
  'a mode switch repaints the glyph in place');

const appearance = read('../settings/appearance.js');
assert.ok(/orb_status/.test(appearance) && /orb-preview-glyph/.test(appearance),
  'the panel offers the mode and previews it');
assert.ok(/orb_style/.test(appearance) && /orbStyle/.test(appearance),
  'and the bar\'s own look');
assert.ok(/PREVIEW_TOOLS/.test(appearance) && /setInterval/.test(appearance),
  'the preview cycles tools');

/* ── 3. The wire: the running tool travels with the step ─────── */

assert.ok(/action: event\.action/.test(read('../agent.js')),
  'the SSE step action reaches the client');
assert.ok(/orb_status/.test(read('../preferences.js')) && /orbStatus/.test(read('../preferences.js')),
  'the setting is stored and published');
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
