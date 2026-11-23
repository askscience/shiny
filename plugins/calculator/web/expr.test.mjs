// Run: node plugins/calculator/web/expr.test.mjs
import assert from 'node:assert/strict';
import { prettyExpr, closeParens, toggleSign, negateText, errorKind } from './expr.js';

assert.equal(prettyExpr('2*pi-8/4'), '2×π−8÷4');
assert.equal(prettyExpr(''), '');
assert.equal(prettyExpr('sqrt(2)+cbrt(8)'), '√(2)+∛(8)');

assert.equal(closeParens('sin(30'), 'sin(30)');
assert.equal(closeParens('((2+3)*(4'), '((2+3)*(4))');
assert.equal(closeParens('2+3)'), '2+3)');

// ± flips the number being typed, and flips it back.
const flips = [
  ['5', '-5'],
  ['5+3', '5+(-3)'],
  ['5-3', '5-(-3)'],
  ['2*-5', '2*5'],
  ['(5', '(-5'],
  ['2.5', '-2.5'],
  ['2pi', '2(-pi)'],
  ['(2+3)', '-(2+3)'],
  ['4*(2+3)', '4*(-(2+3))'],
  ['5+', '5+-'],
  ['', '-'],
];
for (const [from, to] of flips) {
  assert.equal(toggleSign(from), to, `± on ${from}`);
  if (from && !from.endsWith('+') && from !== '2*-5') {
    assert.equal(toggleSign(to), from, `± twice on ${from}`);
  }
}

assert.equal(negateText('8'), '-8');
assert.equal(negateText('-8'), '8');

assert.equal(errorKind('division by zero'), 'divideByZero');
assert.equal(errorKind('unexpected end of expression'), 'incomplete');
assert.equal(errorKind("expected ')'"), 'incomplete');
assert.equal(errorKind('sqrt of a negative number'), 'domain');
assert.equal(errorKind('tan is not defined at 90 degrees'), 'domain');
assert.equal(errorKind('factorial argument too large'), 'tooLarge');
assert.equal(errorKind('unknown symbol "x"'), 'invalid');

console.log('calculator expr: all checks passed');
