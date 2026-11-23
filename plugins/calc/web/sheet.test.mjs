// Run: node plugins/calc/web/sheet.test.mjs
import assert from 'node:assert/strict';
import {
  separators, canonicalNumber, toStored, toEditable, cellText, localizePlain,
  formatNumber, shiftFormula, toTsv, parseTsv, parseDelimited, csvDelimiter,
} from './sheet.js';

const it = separators('it');
const en = separators('en');
assert.deepEqual(it, { decimal: ',', group: '.' });
assert.equal(en.decimal, '.');

// Typing numbers in Italian.
const storedIt = [
  ['2,50', '2.50'], ['1.20', '1.20'], ['1.000', '1000'], ['1.000,50', '1000.50'],
  ['-3,5', '-3.5'], ['10%', '10%'], ['12,5%', '12.5%'], ['€ 5', '€ 5'], ['5,00 €', '5.00 €'],
  ['Pane', 'Pane'], ['15/03/2026', '15/03/2026'], ['333 1234567', '333 1234567'],
  ['1.2.3', '1.2.3'], ['02115', '02115'], ['  7 ', '7'], ['1 000', '1000'],
];
for (const [typed, stored] of storedIt) assert.equal(toStored(typed, it), stored, `it: ${typed}`);
// …and in English.
for (const [typed, stored] of [['1,000', '1000'], ['2.50', '2.50'], ['2,50', '2.50'], ['1,000.5', '1000.5']]) {
  assert.equal(toStored(typed, en), stored, `en: ${typed}`);
}
assert.equal(canonicalNumber('1.2.3', it), null);

// Formulas: the user's separators in, canonical stored, and back.
const formulas = [
  ['=ROUND(B5;1)', '=ROUND(B5,1)', '=ROUND(B5;1)'],
  ['=B2*1,22', '=B2*1.22', '=B2*1,22'],
  ['=SOMMA.SE(A1:A4;">2")', '=SOMMA.SE(A1:A4,">2")', '=SOMMA.SE(A1:A4;">2")'],
  ['=SE(A1>1;"a;b, c";0,5)', '=SE(A1>1,"a;b, c",0.5)', '=SE(A1>1;"a;b, c";0,5)'],
  ['=E1+1', '=E1+1', '=E1+1'],
];
for (const [typed, stored, shown] of formulas) {
  assert.equal(toStored(typed, it), stored, `formula in: ${typed}`);
  assert.equal(toEditable(stored, it), shown, `formula back: ${stored}`);
}
assert.equal(toStored('=ROUND(B5;1)', en), '=ROUND(B5,1)');
assert.equal(toEditable('=ROUND(B5,1.5)', en), '=ROUND(B5,1.5)');
assert.equal(toEditable('2.50', it), '2,50');
assert.equal(toEditable('€ 5.00', it), '€ 5,00');
assert.equal(toEditable('Pane', it), 'Pane');

// What a cell shows.
assert.equal(cellText('2.50', { t: 'n', v: 2.5 }, 'it', it), '2,50');
assert.equal(cellText('1000000', { t: 'n', v: 1e6 }, 'it', it), '1.000.000');
assert.equal(cellText('10%', { t: 'n', v: 0.1 }, 'it', it), '10%');
assert.equal(cellText('=0.1+0.2', { t: 'n', v: 0.3 }, 'it', it), '0,3');
assert.equal(cellText('=B5', { t: 'n', v: 6.8 }, 'en', en), '6.8');
assert.equal(cellText('=1/0', { t: 'e', v: '#DIV/0!' }, 'it', it), '#DIV/0!');
assert.equal(cellText('=A1>1', { t: 'b', v: true }, 'it', it, ['VERO', 'FALSO']), 'VERO');
assert.equal(cellText('=A1', undefined, 'it', it), '');
assert.equal(cellText('Pane', { t: 's', v: 'Pane' }, 'it', it), 'Pane');
assert.equal(localizePlain('02115', 'it', it), '02115');
assert.equal(localizePlain('2,50', 'it', it), '2,50'); // an older value
assert.equal(formatNumber(1 / 3, 'it'), '0,3333333333');
assert.equal(formatNumber(1.2e20, 'en'), '1.2E20');

// Pasting a formula elsewhere moves its relative refs.
assert.equal(shiftFormula('=SUM(B2:B4)*$C$1', 1, 1), '=SUM(C3:C5)*$C$1');
assert.equal(shiftFormula('=A$1+$A2', 2, 3), '=D$1+$A4');
assert.equal(shiftFormula('=A1&"A1"', 1, 0), '=A2&"A1"');
assert.equal(shiftFormula('=A1', -1, 0), '=#REF!');
assert.equal(shiftFormula('=LOG10(A1)', 1, 0), '=LOG10(A2)');
assert.equal(shiftFormula('plain A1', 1, 0), 'plain A1');

// Clipboard text.
assert.equal(toTsv([['a', 'b c'], ['1', 'x\ty']]), 'a\tb c\n1\t"x\ty"');
assert.deepEqual(parseTsv('a\tb\r\n1\t2\n'), [['a', 'b'], ['1', '2']]);
assert.deepEqual(parseTsv('"x\ty"\tz'), [['x\ty', 'z']]);
assert.deepEqual(parseDelimited('a;"b;c";1,5\n', ';'), [['a', 'b;c', '1,5']]);
assert.equal(csvDelimiter('Nome;Importo;"a,b"\nPane;2,50'), ';');
assert.equal(csvDelimiter('name,amount\nbread,2.5'), ',');

console.log('calc sheet: all checks passed');
