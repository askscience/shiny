/**
 * Word's page model: what the Design dialog saves for a margin, and the rule
 * that only a phone drops the paper.
 *
 * Run:  node plugins/word/web/page.test.mjs
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { savedMargin, geometry, DEFAULT_MARGIN } from './page.js';

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

/* ── margins ───────────────────────────────────────────────── */

check('an untouched margin field (0) stays the default', savedMargin(0) === 0);
check('an empty or broken field is the default', savedMargin('') === 0 && savedMargin('abc') === 0 && savedMargin(undefined) === 0);
check('25 mm is saved as the default', savedMargin(DEFAULT_MARGIN) === 0);
check('a chosen margin is kept', savedMargin(18.5) === 18.5 && savedMargin('30') === 30);
check('a margin is kept within 5–80 mm', savedMargin(2) === 5 && savedMargin(200) === 80);
const g = geometry({ margin_top: savedMargin(0), margin_left: savedMargin(30) });
check('a saved default reads back as 25 mm', g.top === 25 && g.left === 30, JSON.stringify(g));

/* ── the paper outside a phone ─────────────────────────────── */

// The rules that drop the paper and reflow the text sit in a container query
// for narrow windows; every one of them must be scoped to a phone, or a
// narrow desktop window turns the page into a strip as tall as its text.
const css = readFileSync(fileURLToPath(new URL('../../../web/ui/responsive.css', import.meta.url)), 'utf8');
const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
  .map(([, selector, body]) => ({ selector: selector.trim(), body }))
  .filter((rule) => /\.word-(sheet|sheets|stage|ruler|guide)\b/.test(rule.selector));
const dropsPaper = rules.filter((rule) => /min-height:\s*0|width:\s*auto|display:\s*none/.test(rule.body)
  || /\.word-(ruler|guide)\b/.test(rule.selector));
check('there are rules that reflow Word on a phone', dropsPaper.length > 0);
const unscoped = dropsPaper
  .flatMap((rule) => rule.selector.split(','))
  .map((selector) => selector.trim())
  .filter((selector) => selector && !selector.startsWith("html[data-phone='1']"));
check('every rule that drops the paper applies to a phone only', unscoped.length === 0, unscoped.join(' | '));

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall Word page checks passed');
