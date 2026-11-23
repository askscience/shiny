// Run: node plugins/studio/web/macro.test.mjs
// The same cases as engine.rs: the knob and the sound agree.
import assert from 'node:assert/strict';
import { macroTarget } from './macro.js';

assert.equal(macroTarget(1000, 0.5, 1, 20, 20000), 1000);
assert.equal(macroTarget(0.5, 1, 0.25, 0, 1), 0.75);
assert.equal(macroTarget(0.5, 0, 0.25, 0, 1), 0.25);
assert.equal(macroTarget(0.5, 1, -0.5, 0, 1), 0);
assert.equal(macroTarget(0, 0.75, 0.5, -1, 1), 0.5);
assert.equal(macroTarget(0.9, 1, 1, 0, 1), 1);
console.log('studio macro: all checks passed');
