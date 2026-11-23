// Run: node plugins/studio/web/rhythm.test.mjs
// The same cases as engine.rs `rhythm_tests`: the grid and the sound agree.
import assert from 'node:assert/strict';
import { rhythmToCells, cellsToRhythm } from './rhythm.js';

const hits = (rhythm, steps) => rhythmToCells(rhythm, steps)
  .map((on, i) => (on ? i : -1)).filter((i) => i >= 0);

assert.deepEqual(hits('e4,0', 16), [0, 4, 8, 12]);
assert.deepEqual(hits('e4', 16), [0, 4, 8, 12]);
assert.deepEqual(hits('e3', 8), [0, 2, 5]);
assert.deepEqual(hits('e5,0', 8), [0, 1, 3, 4, 6]);
assert.deepEqual(hits('e4,2', 16), [2, 6, 10, 14]);
assert.deepEqual(hits('e3,1', 8), [1, 3, 6]);
assert.deepEqual(hits('e0', 16), []);
assert.equal(hits('e16', 16).length, 16);
assert.equal(hits('e20', 16).length, 16);
assert.deepEqual(hits(' e 2 , 1 ', 8), [1, 5]);

assert.deepEqual(hits('x...x...', 8), [0, 4]);
assert.deepEqual(hits('x.x.|x.x.', 8), [0, 2, 4, 6]);
assert.deepEqual(hits('x x . x', 4), [0, 1, 3]);
assert.deepEqual(hits('xxxxxxxxxx', 4), [0, 1, 2, 3]);
assert.deepEqual(hits('', 8), []);

assert.equal(cellsToRhythm([true, false, true, false]), 'x.x.');
console.log('studio rhythm: all checks passed');
