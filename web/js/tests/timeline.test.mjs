/**
 * Shared timeline pieces: time scales, tick spacing, timecodes, meter
 * ballistics and waveform bucket picking. Run: node web/js/tests/timeline.test.mjs
 */
import assert from 'node:assert/strict';
import { secondsScale, beatsScale, niceStep, formatTimecode } from '../../ui/components/timeScale.js';
import { meterFrac, gainToDb, METER_FLOOR_DB } from '../../ui/components/levelMeter.js';

let failed = 0;
function check(name, fn) {
  try { fn(); console.log(`ok - ${name}`); } catch (e) { failed++; console.error(`not ok - ${name}\n  ${e.message}`); }
}

check('seconds scale maps both ways', () => {
  const s = secondsScale({ pps: 80 });
  assert.equal(s.toPx(2.5), 200);
  assert.equal(s.fromPx(200), 2.5);
});

check('major ticks never crowd', () => {
  for (const pps of [0.5, 2, 10, 60, 300, 1500]) {
    const s = secondsScale({ pps });
    assert.ok(s.major * pps >= 84 || s.major === 3600, `pps ${pps}: major ${s.major}`);
    const majors = s.ticks(0, 4000 / pps).filter((t) => t.major);
    for (let i = 1; i < majors.length; i++) {
      assert.ok((majors[i].t - majors[i - 1].t) * pps >= 83.9);
    }
  }
});

check('nice steps', () => {
  assert.equal(niceStep(60, 84), 2);
  assert.equal(niceStep(1, 84), 120);
});

check('timecodes', () => {
  assert.equal(formatTimecode(75.5, 1), '1:15.5');
  assert.equal(formatTimecode(3661), '1:01:01');
  assert.equal(formatTimecode(5), '0:05');
  assert.equal(formatTimecode(59.96, 1), '1:00.0');
});

check('labels follow the zoom precision', () => {
  assert.equal(secondsScale({ pps: 3000 }).label(1.25), '0:01.25');
  assert.equal(secondsScale({ pps: 40 }).label(61), '1:01');
});

check('seconds snap to minor ticks only when close', () => {
  const s = secondsScale({ pps: 100 });
  assert.equal(s.snap(1.02), 1);
  assert.equal(s.snap(1.27, 2), 1.27);
});

check('beats scale labels bars', () => {
  const b = beatsScale({ ppb: 24, beatsPerBar: 4 });
  assert.equal(b.toPx(4), 96);
  assert.equal(b.label(8), '3');
  const ticks = b.ticks(0, 8);
  assert.ok(ticks.find((t) => t.t === 4).bar);
});

check('meter range is -60…+6 dBFS', () => {
  assert.equal(meterFrac(0), 0);
  assert.equal(meterFrac(10 ** (METER_FLOOR_DB / 20)), 0);
  assert.ok(Math.abs(meterFrac(1) - 60 / 66) < 1e-9);
  assert.equal(meterFrac(4), 1);
  assert.ok(Math.abs(gainToDb(0.5) + 6.0206) < 1e-3);
});

if (failed) { console.error(`${failed} failed`); process.exit(1); }
