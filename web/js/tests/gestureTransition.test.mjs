/**
 * Unit checks for gestureTransition.js — the commit/rollback decision, the
 * settle timing and the transition driver, with a fake frame clock. Run:
 *   node web/js/tests/gestureTransition.test.mjs
 */

import {
  shouldCommit, settleDuration, rubberBand, animateValue, createTransition,
  COMMIT_PROGRESS, FLICK_VELOCITY, FLICK_MIN_PROGRESS, SETTLE_MIN_MS, SETTLE_MAX_MS,
} from '../gestureTransition.js';

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail === '' ? '' : ` — ${detail}`}`);
  }
}

/* ── shouldCommit ─────────────────────────────────────────────── */

check('half-way commits', shouldCommit(COMMIT_PROGRESS, 0));
check('just short of half-way rolls back', !shouldCommit(COMMIT_PROGRESS - 0.01, 0));
check('a full swipe commits', shouldCommit(1, 0));
check('nothing travelled rolls back', !shouldCommit(0, 0));
check('a forward flick commits early', shouldCommit(0.2, FLICK_VELOCITY + 0.5));
check('a flick below the minimum travel does not', !shouldCommit(FLICK_MIN_PROGRESS - 0.02, 10));
check('a slow release short of half-way rolls back', !shouldCommit(0.4, FLICK_VELOCITY - 0.1));
check('a backward flick rolls back from far along', !shouldCommit(0.8, -FLICK_VELOCITY - 0.5));
check('a gentle backward drift past half-way still commits', shouldCommit(0.7, -0.3));
check('garbage input rolls back', !shouldCommit(undefined, NaN));

/* ── settleDuration ───────────────────────────────────────────── */

const full = settleDuration(0, 1, 0);
const half = settleDuration(0.5, 1, 0);
const tiny = settleDuration(0.97, 1, 0);
check('a full distance takes the longest', full === SETTLE_MAX_MS, String(full));
check('half the distance is quicker', half < full && half > SETTLE_MIN_MS, String(half));
check('almost there is about the minimum', tiny >= SETTLE_MIN_MS && tiny < SETTLE_MIN_MS + 20, String(tiny));
check('rolling back is timed the same way', settleDuration(1, 0, 0) === full);
check('a fast release towards the end shortens it', settleDuration(0.3, 1, 6) < settleDuration(0.3, 1, 0));
check('a release away from the end does not', settleDuration(0.3, 1, -6) === settleDuration(0.3, 1, 0));

/* ── rubberBand ───────────────────────────────────────────────── */

check('inside 0..1 is untouched', rubberBand(0.42) === 0.42);
check('past the end is damped', rubberBand(1.5) > 1 && rubberBand(1.5) < 1.12, String(rubberBand(1.5)));
check('before the start is damped', rubberBand(-0.5) < 0 && rubberBand(-0.5) > -0.12, String(rubberBand(-0.5)));
check('the damping is monotonic', rubberBand(2) > rubberBand(1.5));

/* ── a fake frame clock ───────────────────────────────────────── */

function fakeClock() {
  let time = 0;
  let queue = [];
  let next = 1;
  return {
    raf: (fn) => {
      const id = next++;
      queue.push({ id, fn });
      return id;
    },
    cancel: (id) => { queue = queue.filter((f) => f.id !== id); },
    now: () => time,
    /** Advance by `ms` and run the frames that were due. */
    step(ms = 16) {
      time += ms;
      const due = queue;
      queue = [];
      for (const f of due) f.fn(time);
    },
    run(ms = 2000) {
      for (let t = 0; t < ms; t += 16) this.step(16);
    },
    get pending() { return queue.length; },
  };
}

/* ── animateValue ─────────────────────────────────────────────── */

{
  const clock = fakeClock();
  const frames = [];
  let done = 0;
  animateValue({
    from: 0, to: 1, duration: 300, onFrame: (v) => frames.push(v), onDone: () => { done += 1; },
    raf: clock.raf, cancel: clock.cancel, now: clock.now, reduced: false,
  });
  clock.run(400);
  check('animateValue ends exactly on the target', frames[frames.length - 1] === 1);
  check('animateValue moves monotonically', frames.every((v, i) => i === 0 || v >= frames[i - 1]));
  check('animateValue calls onDone once', done === 1);
}

{
  const frames = [];
  let done = 0;
  animateValue({ from: 0.3, to: 0, duration: 300, onFrame: (v) => frames.push(v), onDone: () => { done += 1; }, reduced: true });
  check('reduced motion jumps straight to the target', frames.length === 1 && frames[0] === 0 && done === 1);
}

/* ── createTransition ─────────────────────────────────────────── */

function recorder() {
  const applied = [];
  const finished = [];
  return {
    applied,
    finished,
    target: { apply: (p) => applied.push(p), finish: (c) => finished.push(c) },
  };
}

{
  const clock = fakeClock();
  const r = recorder();
  const tr = createTransition(r.target, clock);
  tr.update(0.1);
  tr.update(0.2);
  tr.update(0.3);
  check('updates wait for a frame', r.applied.length === 0);
  clock.step();
  check('updates coalesce to one apply per frame', r.applied.length === 1 && r.applied[0] === 0.3, JSON.stringify(r.applied));
  tr.update(0.62);
  tr.end(0);
  clock.run(600);
  check('a release past half-way commits', r.finished.length === 1 && r.finished[0] === true, JSON.stringify(r.finished));
  check('the commit lands on 1', r.applied[r.applied.length - 1] === 1);
  check('the transition reports settled', tr.settled);
}

{
  const clock = fakeClock();
  const r = recorder();
  const tr = createTransition(r.target, clock);
  tr.update(0.35);
  clock.step();
  tr.end(0);
  clock.run(600);
  check('a short release rolls back', r.finished[0] === false);
  check('the rollback lands on 0', r.applied[r.applied.length - 1] === 0);
}

{
  const clock = fakeClock();
  const r = recorder();
  const tr = createTransition(r.target, clock);
  tr.update(0.25);
  clock.step();
  tr.end(FLICK_VELOCITY * 2);
  clock.run(600);
  check('a flick commits from a quarter of the way', r.finished[0] === true);
}

{
  const clock = fakeClock();
  const r = recorder();
  const tr = createTransition(r.target, clock);
  tr.update(0.9);
  clock.step();
  tr.cancel();
  clock.run(600);
  check('cancel always rolls back', r.finished[0] === false);
}

{
  const clock = fakeClock();
  const r = recorder();
  const tr = createTransition(r.target, clock);
  tr.play();
  clock.run(600);
  check('play runs the whole way and commits', r.finished[0] === true && r.applied[r.applied.length - 1] === 1);
  const before = r.applied.length;
  tr.update(0.2);
  tr.end(0);
  clock.run(100);
  check('a settled transition ignores late input', r.applied.length === before && r.finished.length === 1);
}

{
  const clock = fakeClock();
  const r = recorder();
  const tr = createTransition(r.target, clock);
  tr.play();
  clock.step();
  tr.finishNow(true);
  check('finishNow jumps to the end and finishes once', r.finished.length === 1 && r.applied[r.applied.length - 1] === 1);
  clock.run(600);
  check('the interrupted animation does not finish again', r.finished.length === 1);
}

{
  const clock = fakeClock();
  const r = recorder();
  const tr = createTransition(r.target, clock);
  tr.update(1.6);
  clock.step();
  check('overshoot is rubber-banded', r.applied[0] > 1 && r.applied[0] < 1.12, String(r.applied[0]));
  tr.update(0.55);
  tr.end(-FLICK_VELOCITY * 2);
  clock.run(600);
  check('a backward flick after overshoot rolls back', r.finished[0] === false);
}

if (failures) {
  console.error(`\n${failures} gestureTransition check(s) failed`);
  process.exit(1);
}
console.log('\ngestureTransition checks passed');
