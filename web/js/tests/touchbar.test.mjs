/**
 * Touch Bar mapping test.
 *
 * The Touch Bar is driven from three places that must never drift apart:
 *
 *   1. `touchbarShared.js`  — the action vocabulary the page runs on.
 *   2. `scripts/touchbar/shiny-touchbar.toml` — the tiny-dfr row (Linux T2).
 *   3. `crates/peakd/src/touchbar.rs` — the native macOS `NSTouchBar` buttons.
 *
 * This pins (1) and then parses (2) and (3) to prove they agree: the same
 * action names, the same Ctrl+Alt+Shift+<digit> combos, the same order.
 *
 * Run:  node web/js/tests/touchbar.test.mjs
 */
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import {
  TOUCHBAR_ACTIONS,
  actionForEvent,
  actionLabel,
  codeForAction,
  comboForAction,
  isTouchBarActive,
  normalizeTouchBarSetting,
} from '../touchbarShared.js';

let failures = 0;
const check = (name, condition, detail = '') => {
  console.log(`  ${condition ? 'ok  ' : 'FAIL'} ${name}${detail ? ` — ${detail}` : ''}`);
  if (!condition) failures += 1;
};

const here = (relative) => fileURLToPath(new URL(relative, import.meta.url));

/** A synthetic keydown event for `actionForEvent`. */
const key = (code, mods = { ctrlKey: true, altKey: true, shiftKey: true }) => ({ code, ...mods });

console.log('touch bar mapping');

/* ── The shared vocabulary ──────────────────────────────────── */

// DOM KeyboardEvent.code -> the input-linux key name tiny-dfr sends.
const DOM_TO_INPUT = {
  Digit0: 'Num0', Digit1: 'Num1', Digit2: 'Num2', Digit3: 'Num3', Digit4: 'Num4',
  Digit5: 'Num5', Digit6: 'Num6', Digit7: 'Num7', Digit8: 'Num8', Digit9: 'Num9',
  Minus: 'Minus', Equal: 'Equal', Comma: 'Comma', Period: 'Dot',
};

check('thirteen buttons are defined', TOUCHBAR_ACTIONS.length === 13);
check(
  'action names are unique',
  new Set(TOUCHBAR_ACTIONS.map((a) => a.action)).size === TOUCHBAR_ACTIONS.length,
);
check(
  'DOM codes are unique',
  new Set(TOUCHBAR_ACTIONS.map((a) => a.code)).size === TOUCHBAR_ACTIONS.length,
);
check(
  'every combo is Ctrl+Alt+Shift + the key for its DOM code',
  TOUCHBAR_ACTIONS.every((a) => a.combo.length === 4
    && a.combo[0] === 'LeftCtrl' && a.combo[1] === 'LeftAlt' && a.combo[2] === 'LeftShift'
    && a.combo[3] === DOM_TO_INPUT[a.code]),
  TOUCHBAR_ACTIONS.map((a) => `${a.code}:${a.combo[3]}`).join(' '),
);

check('a full combo resolves', actionForEvent(key('Digit1')) === 'talk');
check('the last combo resolves', actionForEvent(key('Digit9')) === 'settings');
check('the screen-dim combo resolves', actionForEvent(key('Comma')) === 'screen-down');
check('the screen-bright combo resolves', actionForEvent(key('Period')) === 'screen-up');
check('the kbd-dim combo resolves', actionForEvent(key('Minus')) === 'kbd-backlight-down');
check('the kbd-bright combo resolves', actionForEvent(key('Equal')) === 'kbd-backlight-up');
check('a missing shift does not match', actionForEvent(key('Digit1', { ctrlKey: true, altKey: true })) === null);
check('a missing alt does not match', actionForEvent(key('Digit1', { ctrlKey: true, shiftKey: true })) === null);
check('a missing ctrl does not match', actionForEvent(key('Digit1', { altKey: true, shiftKey: true })) === null);
check('super also held does not match', actionForEvent({ ...key('Digit1'), metaKey: true }) === null);
check('an unmapped key does not match', actionForEvent(key('Digit0')) === null);
check('a bare key does not match', actionForEvent({ code: 'Digit1' }) === null);
check('no event is handled', actionForEvent(null) === null);

check('codeForAction round-trips', TOUCHBAR_ACTIONS.every((a) => codeForAction(a.action) === a.code));
check('comboForAction round-trips', TOUCHBAR_ACTIONS.every((a) => comboForAction(a.action)?.length === 4));
check('codeForAction of junk is null', codeForAction('nope') === null);
check('actionLabel reads the label', actionLabel('talk') === 'Ask');
check('actionLabel falls back to the name', actionLabel('nope') === 'nope');

/* ── The enablement rule ────────────────────────────────────── */

check('an unknown setting normalises to auto', normalizeTouchBarSetting('wat') === 'auto');
check('a missing setting normalises to auto', normalizeTouchBarSetting(null) === 'auto');
check('auto + no host bar is inert', isTouchBarActive('auto', false) === false);
check('auto + host bar is active', isTouchBarActive('auto', true) === true);
check('auto + unknown host is inert', isTouchBarActive('auto', undefined) === false);
check('on forces the buttons', isTouchBarActive('on', false) === true);
check('off disables even a real bar', isTouchBarActive('off', true) === false);
check('a garbage setting behaves as auto', isTouchBarActive('wat', false) === false);

/* ── The tiny-dfr config (Linux T2) ─────────────────────────── */

const toml = readFileSync(here('../../../scripts/touchbar/shiny-touchbar.toml'), 'utf8');
const tomlButtons = [...toml.matchAll(/\{\s*Icon\s*=\s*"([^"]+)"\s*,\s*Action\s*=\s*\[([^\]]+)\]\s*\}/g)]
  .map((m) => ({
    icon: m[1],
    combo: (m[2].match(/"([^"]+)"/g) || []).map((s) => s.replace(/"/g, '')),
  }));

check('the tiny-dfr row has every button', tomlButtons.length === TOUCHBAR_ACTIONS.length);
check(
  'the tiny-dfr row matches icon, combo and order',
  tomlButtons.every((b, i) => b.icon === TOUCHBAR_ACTIONS[i].icon
    && b.combo.join('+') === TOUCHBAR_ACTIONS[i].combo.join('+')),
  JSON.stringify(tomlButtons),
);
check('the tiny-dfr config replaces the primary layer', /PrimaryLayerKeys\s*=/.test(toml));
check('the tiny-dfr config keeps the media layer', !/MediaLayerKeys\s*=/.test(toml));

// Every icon we ship must exist on disk, or tiny-dfr would render a blank
// slot; and every icon the config references must resolve either to one of
// ours or to a tiny-dfr built-in.
const shipped = new Set(
  readdirSync(here('../../../scripts/touchbar/icons')).map((f) => f.replace(/\.svg$/, '')),
);
const BUILTIN_ICONS = new Set([
  'volume_off', 'volume_down', 'volume_up', 'fast_rewind', 'fast_forward',
  'backlight_low', 'backlight_high', 'brightness_low', 'brightness_high',
]);
check(
  'every custom icon is shipped',
  TOUCHBAR_ACTIONS.map((a) => a.icon)
    .filter((name) => name.startsWith('shiny_'))
    .every((name) => shipped.has(name)),
  [...shipped].join(', '),
);
check(
  'every referenced icon resolves',
  TOUCHBAR_ACTIONS.every((a) => shipped.has(a.icon) || BUILTIN_ICONS.has(a.icon)),
);

/* ── The native macOS buttons ───────────────────────────────── */

const rust = readFileSync(here('../../../crates/peakd/src/touchbar.rs'), 'utf8');
const rustButtons = [...rust.matchAll(/\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)/g)]
  .map((m) => ({ action: m[1], label: m[2] }));

check('the native bar has every button', rustButtons.length === TOUCHBAR_ACTIONS.length);
check(
  'the native bar matches action and order',
  rustButtons.every((b, i) => b.action === TOUCHBAR_ACTIONS[i].action
    && b.label === TOUCHBAR_ACTIONS[i].label),
  JSON.stringify(rustButtons),
);

if (failures) {
  console.error(`\n${failures} failure(s)`);
  process.exit(1);
}
console.log('\nall touch bar checks passed');
