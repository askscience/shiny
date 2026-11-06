/**
 * Unit checks for overviewGrid.js — where the window overview puts its cards
 * and how they travel per motion language. Run:
 *   node web/js/tests/overviewGrid.test.mjs
 */

import { gridLayout, motionFor, blurFor, MOTION, LABEL_H, LABEL_H_PHONE } from '../overviewGrid.js';

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail === '' ? '' : ` — ${detail}`}`);
  }
}

const overlaps = (a, b) => a.x < b.x + b.w - 0.5 && b.x < a.x + a.w - 0.5
  && a.y < b.y + b.h + LABEL_H - 0.5 && b.y < a.y + a.h + LABEL_H - 0.5;
const noOverlap = (cards) => cards.every((a, i) => cards.every((b, j) => i === j || !overlaps(a, b)));
const inside = (cards, W) => cards.every((c) => c.x >= -0.5 && c.x + c.w <= W + 0.5);

/* ── Phone ────────────────────────────────────────────────────── */

{
  const W = 390, H = 760;
  const g = gridLayout(5, W, H, { phone: true });
  check('phone portrait: two columns', g.cols === 2, `cols=${g.cols}`);
  check('phone portrait: three rows for five windows', g.rows === 3, `rows=${g.rows}`);
  check('phone cards keep the screen shape', Math.abs(g.cards[0].w / g.cards[0].h - W / H) < 0.01);
  check('phone cards never overlap', noOverlap(g.cards));
  check('phone cards stay inside the width', inside(g.cards, W));
  check('a short last row is centred', Math.abs((g.cards[4].x + g.cards[4].w / 2) - W / 2) < 1,
    `x=${g.cards[4].x}`);
}

{
  const g = gridLayout(4, 844, 390, { phone: true });
  check('phone landscape: three columns', g.cols === 3, `cols=${g.cols}`);
}

{
  const W = 390, H = 760;
  const one = gridLayout(1, W, H, { phone: true });
  const two = gridLayout(2, W, H, { phone: true });
  check('one window keeps the two-column card size', Math.abs(one.cards[0].w - two.cards[0].w) < 0.5);
  check('one window is centred', Math.abs(one.cards[0].x + one.cards[0].w / 2 - W / 2) < 1);
}

{
  const g = gridLayout(12, 390, 760, { phone: true });
  check('many phone windows scroll', g.maxScroll > 0, `maxScroll=${g.maxScroll}`);
  check('the scroll reaches the last row', g.cards[11].y + g.cards[11].h + LABEL_H_PHONE <= 760 + g.maxScroll + 1);
  check('a phone label has room for round buttons', LABEL_H_PHONE >= 44);
}

/* ── Desktop ──────────────────────────────────────────────────── */

{
  const W = 1600, H = 900;
  for (const n of [1, 2, 3, 4, 6, 9]) {
    const g = gridLayout(n, W, H);
    check(`desktop ${n}: fits without scrolling`, g.maxScroll === 0, `maxScroll=${g.maxScroll}`);
    check(`desktop ${n}: no overlap`, noOverlap(g.cards));
    check(`desktop ${n}: inside the width`, inside(g.cards, W));
  }
  check('desktop: one window does not fill the screen', gridLayout(1, W, H).cards[0].w <= W * 0.42 + 0.5);
  const four = gridLayout(4, W, H);
  check('desktop: four windows make a 2×2', four.cols === 2 && four.rows === 2, `${four.cols}×${four.rows}`);
  const centred = gridLayout(2, W, H);
  const mid = centred.cards[0].y + (centred.cards[0].h + LABEL_H) / 2;
  check('desktop: a grid that fits is centred vertically', Math.abs(mid - H / 2) < 2, `mid=${mid}`);
}

{
  const g = gridLayout(40, 1280, 720);
  check('desktop: a crowd keeps a readable card and scrolls', g.cards[0].w >= 199 && g.maxScroll > 0,
    `w=${g.cards[0].w} maxScroll=${g.maxScroll}`);
  check('desktop: a crowd does not overlap', noOverlap(g.cards));
}

check('no windows, no cards', gridLayout(0, 800, 600).cards.length === 0);

/* ── Motion ───────────────────────────────────────────────────── */

for (const name of Object.keys(MOTION)) {
  const m = MOTION[name];
  check(`${name}: starts at 0`, Math.abs(m.ease(0)) < 1e-6, String(m.ease(0)));
  check(`${name}: lands at 1`, Math.abs(m.ease(1) - 1) < 1e-6, String(m.ease(1)));
}
check('fluid overshoots a little', [0.6, 0.7, 0.8, 0.9].some((k) => MOTION.fluid.ease(k) > 1));
check('kinetic and depth never overshoot',
  [0.2, 0.5, 0.8, 0.95].every((k) => MOTION.kinetic.ease(k) <= 1 && MOTION.depth.ease(k) <= 1));
check('an unknown motion falls back to kinetic', motionFor('nope') === MOTION.kinetic);
check('depth blurs on the way', blurFor('depth', 0.3) > 0);
check('depth lands sharp', blurFor('depth', 1) === 0);
check('the other languages never blur', blurFor('kinetic', 0.3) === 0 && blurFor('fluid', 0.3) === 0);

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall overviewGrid checks passed');
