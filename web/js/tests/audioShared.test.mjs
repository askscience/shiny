/**
 * Sound panel helpers test.
 *
 * The chip and the Sound menu must agree on a node's icon and label, so both
 * read them from `audioShared.js` — this pins that mapping: mute state wins,
 * then form factor, then the level bucket. A muted mic can never draw a live
 * mic glyph, and headphones never draw speaker waves.
 *
 * `audioShared.js` is deliberately import-free, so Node can load it directly.
 *
 * Run:  node web/js/tests/audioShared.test.mjs
 */
import { currentNode, isHeadphones, nodeIcon, nodeLabel, nodeSubtitle } from '../audioShared.js';

let failures = 0;
const check = (name, condition, detail = '') => {
  console.log(`  ${condition ? 'ok  ' : 'FAIL'} ${name}${detail ? ` — ${detail}` : ''}`);
  if (!condition) failures += 1;
};

const sink = (over = {}) => ({
  id: 63,
  name: 'alsa_output.pci-0000_04_00.3.HiFi__Speaker__sink',
  description: 'Apple Audio Device Internal Speakers',
  nick: 'Speaker',
  card: 'Apple Audio Device',
  is_default: true,
  volume_percent: 42,
  muted: false,
  level: 2,
  state: 'RUNNING',
  channels: 6,
  form_factor: null,
  icon: 'audio-speakers',
  active_port: '[Out] Speaker',
  ...over,
});

console.log('sound panel helpers');

check(
  'the default node wins',
  currentNode([sink({ is_default: false }), sink({ id: 9, is_default: true, nick: 'Headphones' })], null)?.nick === 'Headphones',
);
check('with no default the first node is used', currentNode([sink({ is_default: false })], null)?.id === 63);
check('a named default is honoured', currentNode([sink({ is_default: false })], sink().name)?.id === 63);
check('no nodes → null', currentNode([], null) === null);
check('missing nodes → null', currentNode(undefined, 'x') === null);

check('an unmuted speaker uses the level bucket', nodeIcon(sink(), 'sink') === 'hud/volume-2');
check('a muted sink shows the muted glyph', nodeIcon(sink({ muted: true }), 'sink') === 'hud/volume-muted');
check('a muted mic shows mic-off', nodeIcon(sink({ muted: true }), 'source') === 'ui/mic-off');
check('a live mic shows the mic glyph', nodeIcon(sink({ muted: false }), 'source') === 'ui/mic');
check('headphones (form factor) win over waves', nodeIcon(sink({ form_factor: 'headphones' }), 'sink') === 'hud/headphones');
check('headphones (driver icon) win over waves', nodeIcon(sink({ icon: 'audio-headphones' }), 'sink') === 'hud/headphones');
check('mute wins over headphones', nodeIcon(sink({ muted: true, form_factor: 'headphones' }), 'sink') === 'hud/volume-muted');
check('a missing node is silent', nodeIcon(null, 'sink') === 'hud/volume-0');
check('a missing input still reads as a mic', nodeIcon(null, 'source') === 'ui/mic');
check('isHeadphones reads the form factor', isHeadphones(sink({ form_factor: 'headset' })) === true);

check('the nick is the label', nodeLabel(sink()) === 'Speaker');
check('the description is the label fallback', nodeLabel(sink({ nick: null })) === 'Apple Audio Device Internal Speakers');
check('the subtitle joins card, port and use', nodeSubtitle(sink()) === 'Apple Audio Device · [Out] Speaker · in use');
check('idle nodes drop the in-use bit', nodeSubtitle(sink({ state: 'SUSPENDED' })) === 'Apple Audio Device · [Out] Speaker');

if (failures) {
  console.error(`\n${failures} failure(s)`);
  process.exit(1);
}
console.log('\nall sound panel checks passed');
