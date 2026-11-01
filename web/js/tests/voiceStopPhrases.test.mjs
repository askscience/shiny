import { readFileSync } from 'node:fs';
import { isStopPhrase } from '../voice/wake.js';

let failed = 0;
const check = (ok, msg) => { console.log(`${ok ? '  ok ' : '  FAIL'} ${msg}`); if (!ok) failed++; };

// Whole utterance only, punctuation/case/accents ignored, any script.
check(isStopPhrase('Stop.'), 'English "Stop." ends the conversation');
check(isStopPhrase('Basta così!'), 'Italian "Basta così!" (accented) ends it');
check(isStopPhrase('basta cosi'), 'Italian without the accent ends it');
check(isStopPhrase('Хватит'), 'Cyrillic phrase survives normalization');
check(isStopPhrase('Σταμάτα!'), 'Greek phrase survives normalization');
check(isStopPhrase("C'est tout"), 'French apostrophe phrase ends it');
check(!isStopPhrase('stop the music'), '"stop the music" still reaches the agent');
check(!isStopPhrase('ok'), '"ok" never ends a conversation');
check(!isStopPhrase(''), 'empty transcript is not a stop phrase');
check(!isStopPhrase('seis'), '"seis" (six in es/pt) is not a stop phrase');

// Every catalogue's conversation hint must quote a phrase the code recognizes.
const dir = new URL('../../i18n/', import.meta.url);
const { locales } = JSON.parse(readFileSync(new URL('index.json', dir), 'utf8'));
for (const { code } of locales) {
  const hint = JSON.parse(readFileSync(new URL(`${code}.json`, dir), 'utf8'))['settings.voice.conversationHint'];
  if (!hint) continue;
  const quoted = [...hint.matchAll(/[“„«"]([^”“»"]+)[”“»"]/g)].map((m) => m[1]);
  const phrase = quoted.find((q) => isStopPhrase(q));
  check(Boolean(phrase), `${code}: conversation hint quotes a recognized stop phrase (${phrase ?? quoted.join(' | ')})`);
}

if (failed) { console.error(`\n${failed} stop-phrase check(s) failed`); process.exit(1); }
console.log('\nall stop-phrase checks passed');
