/**
 * overviewGrid.js — where the window overview puts its cards.
 *
 * Pure geometry, no DOM: given how many windows there are and the size of the
 * stage, it returns one box per window (the thumbnail's room, a label line
 * under it) in a grid. overview.js fits each window into its box, keeping the
 * window's own shape.
 *
 *  - Phone: two columns in portrait, three in landscape, every card the shape
 *    of the screen; a long list scrolls.
 *  - Desktop: the column count that gives the biggest cards while every
 *    window stays on screen, capped so one or two windows do not fill it;
 *    past a minimum card width the grid keeps that width and scrolls.
 *
 * Rows are centred (a short last row too), and the whole grid is centred
 * vertically when it fits.
 */

/** Height (px) of the label line under each card. */
export const LABEL_H = 30;
/** …and on a phone, where the label holds finger-sized round buttons. */
export const LABEL_H_PHONE = 46;

/** The spacing for a stage: outer padding, gutter and edge room. */
function spacing(W, H, phone) {
  if (phone) return { padX: 16, padY: 16, gap: 14 };
  return {
    padX: Math.max(32, Math.round(W * 0.06)),
    padY: Math.max(28, Math.round(H * 0.08)),
    gap: 28,
  };
}

/** A box's height for its width: the stage's own shape. */
const heightFor = (w, aspect) => w / aspect;

/**
 * Lay `n` cards out on a `W`×`H` stage.
 *
 * @returns {{ cols: number, rows: number, cards: {x:number,y:number,w:number,h:number}[],
 *             contentH: number, maxScroll: number }}
 *   `cards[i]` is the thumbnail box of window `i` (the label sits under it,
 *   `labelH` tall — `LABEL_H`, or `LABEL_H_PHONE` on a phone); `maxScroll`
 *   is how far the grid can scroll.
 */
export function gridLayout(n, W, H, { phone = false, labelH = phone ? LABEL_H_PHONE : LABEL_H } = {}) {
  const count = Math.max(0, n | 0);
  const { padX, padY, gap } = spacing(W, H, phone);
  const aspect = Math.max(0.3, Math.min(3, W / Math.max(1, H)));
  const availW = Math.max(1, W - padX * 2);
  const availH = Math.max(1, H - padY * 2);
  const rowsOf = (cols) => Math.max(1, Math.ceil(count / cols));
  const widthFor = (cols) => (availW - (cols - 1) * gap) / cols;

  let cols;
  let cardW;
  if (phone) {
    cols = W > H ? 3 : 2;
    cardW = widthFor(cols);
  } else {
    // The biggest card that keeps every window on screen.
    const maxW = Math.min(availW, W * 0.42);
    const minW = Math.min(maxW, 200);
    cols = 1;
    cardW = 0;
    for (let c = 1; c <= Math.max(1, count); c += 1) {
      const rows = rowsOf(c);
      const byW = widthFor(c);
      const byH = ((availH - (rows - 1) * gap) / rows - labelH) * aspect;
      const w = Math.min(maxW, byW, byH);
      if (w > cardW + 0.5) { cardW = w; cols = c; }
    }
    if (cardW < minW) {
      // Too many to fit: keep a readable card and scroll.
      cols = Math.max(1, Math.floor((availW + gap) / (minW + gap)));
      cardW = Math.min(minW, widthFor(cols));
    }
  }
  // Fewer windows than columns: the cards keep their size, the row centres.
  cols = Math.max(1, Math.min(cols, Math.max(1, count)));
  const cardH = heightFor(cardW, aspect);
  const rows = rowsOf(cols);
  const rowH = cardH + labelH;
  const gridH = rows * rowH + (rows - 1) * gap;
  const contentH = gridH + padY * 2;
  const top = contentH <= H ? Math.round((H - gridH) / 2) : padY;

  const cards = [];
  for (let i = 0; i < count; i += 1) {
    const row = Math.floor(i / cols);
    const inRow = Math.min(cols, count - row * cols);
    const rowW = inRow * cardW + (inRow - 1) * gap;
    const left = (W - rowW) / 2;
    cards.push({
      x: left + (i % cols) * (cardW + gap),
      y: top + row * (rowH + gap),
      w: cardW,
      h: cardH,
    });
  }
  return { cols, rows, cards, contentH, maxScroll: Math.max(0, Math.ceil(contentH - H)) };
}

/* ── Motion: how a card travels, per motion language ───────────── */

const clamp01 = (v) => Math.min(1, Math.max(0, v));

/**
 * The easing of a card's trip for the user's motion language (`<html
 * data-motion>`, Settings → Appearance → Motion), and the cascade between
 * cards. `k` is the card's own progress (0..1).
 *
 *   kinetic — a quick exponential arrival, a pronounced cascade;
 *   fluid   — a spring that overshoots a little and settles;
 *   depth   — a soft cubic arrival, out of a blur (see `blurFor`).
 */
export const MOTION = {
  kinetic: { stagger: 0.06, ease: (k) => (k >= 1 ? 1 : 1 - 2 ** (-10 * k)) },
  fluid: {
    stagger: 0.035,
    ease: (k) => {
      const c = 1.4;
      const x = clamp01(k) - 1;
      return 1 + (c + 1) * x ** 3 + c * x ** 2;
    },
  },
  depth: { stagger: 0.045, ease: (k) => 1 - (1 - clamp01(k)) ** 3 },
};

export function motionFor(name) {
  return MOTION[name] || MOTION.kinetic;
}

/** Depth's blur (px) for a card at eased progress `e`; none elsewhere. */
export function blurFor(name, e) {
  if (name !== 'depth') return 0;
  const left = 1 - clamp01(e);
  return left < 0.01 ? 0 : Math.round(left * 14 * 10) / 10;
}
