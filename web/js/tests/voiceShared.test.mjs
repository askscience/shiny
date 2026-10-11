/**
 * Pure-unit checks for voiceShared.js. Run:
 *   node web/js/tests/voiceShared.test.mjs
 */

import {
  SUPERTONIC_VOICES, QWEN_SPEAKERS, ENGINE_DEFAULT_VOICE, voicesForEngine, voiceForEngine,
} from '../voiceShared.js';

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}`);
  }
}

check('supertonic set is M1…F5', SUPERTONIC_VOICES.length === 10
  && SUPERTONIC_VOICES[0] === 'M1' && SUPERTONIC_VOICES[9] === 'F5');
check('qwen set is the nine speakers', QWEN_SPEAKERS.length === 9
  && QWEN_SPEAKERS.includes('vivian'));
check('the sets do not overlap',
  SUPERTONIC_VOICES.every((v) => !QWEN_SPEAKERS.includes(v)));

check('default voices belong to their engine',
  voiceForEngine('supertonic', ENGINE_DEFAULT_VOICE.supertonic) === 'M1'
  && voiceForEngine('qwen', ENGINE_DEFAULT_VOICE.qwen) === 'vivian');
check('supertonic accepts its own voice', voiceForEngine('supertonic', 'M1') === 'M1');
check('supertonic rejects a qwen speaker', voiceForEngine('supertonic', 'vivian') === '');
check('qwen rejects a supertonic voice', voiceForEngine('qwen', 'M1') === '');
check('empty and whitespace resolve to default',
  voiceForEngine('qwen', '') === '' && voiceForEngine('qwen', '  ') === '');
check('values are trimmed before validating', voiceForEngine('qwen', ' vivian ') === 'vivian');
check('an unknown engine uses the supertonic set',
  voicesForEngine('nope') === SUPERTONIC_VOICES && voicesForEngine(undefined) === SUPERTONIC_VOICES);

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall voiceShared checks passed');
