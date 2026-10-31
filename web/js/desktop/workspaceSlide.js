/**
 * workspaceSlide.js — the workspace carousel.
 *
 * A workspace switch slides the whole desktop sideways: the current
 * workspace's windows leave with the fingers (or the animation) while the
 * neighbour's windows come in from the edge, a screen width plus a gap away —
 * wrapping around exactly like `switchWorkspace`.
 *
 * Two entry points, one renderer:
 *
 * - `begin(dir)` — a trackpad swipe. Nothing changes state until the release:
 *   the outgoing tiles are pinned (absolutely positioned at their measured
 *   rects) and the neighbour's tiles are laid out in place by the normal
 *   layout engine, so both sets are on screen at once. A commit restores the
 *   inline styles and calls `switchWorkspace`, whose re-render lands the
 *   incoming windows where they already are (a small FLIP covers any
 *   difference); a rollback restores the styles and re-renders the current
 *   workspace.
 * - `capture()` + `play(outgoing, dir)` — a discrete switch (dots, Alt+number,
 *   Alt+,/.). The state has already changed, so the tile manager captures the
 *   outgoing tiles just before it re-renders, and the slide pins them back on
 *   screen to animate out while the freshly laid-out workspace comes in.
 *
 * While a slide runs, `<body>` carries `workspace-sliding` (native Browser
 * pages hide, they cannot move with the DOM) and the document element carries
 * `--workspace-p`, the signed progress (−1..1, positive = towards the next
 * workspace). The tile manager holds its re-renders until the slide is over.
 */
import {
  workspacesEnabled, getWorkspacesList, activeWorkspaceIndex, switchWorkspace,
} from './workspaces.js';
import { createTransition } from '../gestureTransition.js';

/** Between the leaving and the arriving desktop. */
const SLIDE_GAP = 48;

/**
 * @param {object} deps
 * @param {HTMLElement} deps.grid       the `#tile-grid`
 * @param {() => void} deps.render      the tile manager's full re-render
 * @param {(grid: HTMLElement, items: {name: string, el: HTMLElement}[]) => void} deps.applyLayout
 * @param {() => boolean} deps.isPhone  phone layout: one window, no slide
 */
export function createWorkspaceSlide({ grid, render, applyLayout, isPhone }) {
  let stage = null;
  let committing = false;
  let renderHeld = false;

  const tileOf = (name) => [...grid.querySelectorAll(':scope > .tile[data-plugin]')]
    .find((el) => el.dataset.plugin === name) || null;
  const visibleTiles = () => [...grid.querySelectorAll(':scope > .tile[data-plugin]')]
    .filter((el) => !el.classList.contains('hidden'));
  const measure = (el) => ({ x: el.offsetLeft, y: el.offsetTop, w: el.offsetWidth, h: el.offsetHeight });

  function available() {
    return workspacesEnabled() && !isPhone() && getWorkspacesList().length > 1;
  }

  /** The outgoing tiles as they are drawn now, before anything moves. */
  function capture() {
    return visibleTiles().map((el) => ({ el, rect: measure(el), saved: el.style.cssText }));
  }

  /** Take the outgoing tiles out of the layout, at the rects they had. */
  function pin(outgoing) {
    for (const it of outgoing) {
      const { el, rect } = it;
      el.classList.remove('hidden');
      el.classList.add('tile--sliding');
      el.style.position = 'absolute';
      el.style.gridColumn = '';
      el.style.gridRow = '';
      el.style.margin = '0';
      el.style.left = `${rect.x}px`;
      el.style.top = `${rect.y}px`;
      el.style.width = `${rect.w}px`;
      el.style.height = `${rect.h}px`;
    }
  }

  function apply(p) {
    if (!stage) return;
    const shift = -stage.dir * p * stage.span;
    for (const it of stage.outgoing) {
      it.el.style.transform = `translate3d(${shift.toFixed(1)}px, 0, 0)`;
    }
    for (const it of stage.incoming) {
      it.el.style.transform = `translate3d(${(shift + stage.dir * stage.span).toFixed(1)}px, 0, 0)`;
    }
    document.documentElement.style.setProperty('--workspace-p', (stage.dir * p).toFixed(4));
  }

  function open(dir, outgoing, incoming, extra = {}) {
    stage = {
      dir,
      outgoing,
      incoming,
      span: (grid.clientWidth || window.innerWidth) + SLIDE_GAP,
      transition: null,
      ...extra,
    };
    document.body.classList.add('workspace-sliding');
    document.documentElement.dataset.workspaceSlide = dir > 0 ? 'next' : 'prev';
    pin(outgoing);
    for (const it of incoming) it.el.classList.add('tile--sliding');
  }

  /** Drop every inline change the slide made; returns the stage it closed. */
  function close() {
    const s = stage;
    stage = null;
    for (const it of [...s.outgoing, ...s.incoming]) {
      it.el.classList.remove('tile--sliding');
      it.el.style.cssText = it.saved;
    }
    document.body.classList.remove('workspace-sliding');
    delete document.documentElement.dataset.workspaceSlide;
    document.documentElement.style.removeProperty('--workspace-p');
    return s;
  }

  /** A held re-render (something changed mid-slide) runs now. */
  function releaseRender() {
    if (!renderHeld) return false;
    renderHeld = false;
    render();
    return true;
  }

  /** Settle a gesture slide (see the module comment). */
  function settleGesture(committed) {
    const s = stage;
    const before = committed ? new Map(s.incoming.map((it) => [it.el, it.el.getBoundingClientRect()])) : null;
    close();
    for (const it of s.outgoing) it.el.classList.add('hidden');
    if (!committed) {
      for (const it of s.incoming) if (it.wasHidden) it.el.classList.add('hidden');
      grid.className = s.gridClass;
      grid.style.cssText = s.gridStyle;
      renderHeld = false;
      // Re-lay the current workspace (and its splitters) exactly as it was.
      render();
      return;
    }
    renderHeld = false;
    committing = true;
    try {
      switchWorkspace(s.dir > 0 ? 'next' : 'prev');
    } finally {
      committing = false;
    }
    // Normally the real layout is the one already on screen; if focus or
    // fullscreen made it differ, glide the windows the last few pixels.
    for (const it of s.incoming) {
      const from = before.get(it.el);
      if (!from || it.el.classList.contains('hidden')) continue;
      const to = it.el.getBoundingClientRect();
      if (Math.abs(from.left - to.left) < 1 && Math.abs(from.top - to.top) < 1
          && Math.abs(from.width - to.width) < 1 && Math.abs(from.height - to.height) < 1) continue;
      if (!to.width || !to.height || !it.el.animate) continue;
      it.el.animate([
        {
          transformOrigin: '0 0',
          transform: `translate(${from.left - to.left}px, ${from.top - to.top}px) `
            + `scale(${from.width / to.width}, ${from.height / to.height})`,
        },
        { transformOrigin: '0 0', transform: 'none' },
      ], { duration: 200, easing: 'cubic-bezier(0.16, 1, 0.3, 1)' });
    }
  }

  /**
   * A finger-driven slide towards `dir` (+1 next, −1 previous). Returns a
   * transition for the gesture to drive, or null when there is no other
   * workspace to go to (phone, workspaces off, a single workspace).
   */
  function begin(dir) {
    stage?.transition?.finishNow(true);
    if (!available() || !dir) return null;
    const list = getWorkspacesList();
    const from = activeWorkspaceIndex();
    const to = (from + Math.sign(dir) + list.length) % list.length;
    if (to === from) return null;

    const gridClass = grid.className;
    const gridStyle = grid.style.cssText;
    const outgoing = capture();
    const incoming = (list[to].windows || [])
      .map((name) => tileOf(name))
      .filter(Boolean)
      .map((el) => ({ el, saved: el.style.cssText, wasHidden: el.classList.contains('hidden') }));

    open(Math.sign(dir), outgoing, incoming, { kind: 'gesture', gridClass, gridStyle });
    // Lay the neighbour out by the normal engine, as if it were active; the
    // outgoing tiles are pinned and take no part in it.
    applyLayout(grid, incoming.map((it) => ({ name: it.el.dataset.plugin, el: it.el })));
    grid.classList.toggle('tile-grid--single', incoming.length === 1);
    apply(0);

    const transition = createTransition({
      apply,
      finish: (committed) => settleGesture(committed),
    });
    stage.transition = transition;
    return transition;
  }

  /**
   * Animate a switch that has already happened: `outgoing` was captured just
   * before the re-render, the incoming tiles are the ones visible now.
   */
  function play(outgoing, dir) {
    if (!available() || !dir || (!outgoing?.length && !visibleTiles().length)) return;
    const shown = new Set(outgoing.map((it) => it.el));
    const incoming = visibleTiles()
      .filter((el) => !shown.has(el))
      .map((el) => ({ el, saved: el.style.cssText }));
    open(Math.sign(dir), outgoing, incoming, { kind: 'discrete' });
    apply(0);
    const transition = createTransition({
      apply,
      finish: () => {
        const s = close();
        for (const it of s.outgoing) it.el.classList.add('hidden');
        releaseRender();
      },
    });
    stage.transition = transition;
    transition.play();
  }

  return {
    begin,
    capture,
    play,
    available,
    /** True while a slide owns the tiles; re-renders wait for it. */
    holdsRender() {
      if (!stage) return false;
      renderHeld = true;
      return true;
    },
    /** A slide is committing its own switch: no second animation for it. */
    isCommitting: () => committing,
    /**
     * Clear the stage for a switch that just happened elsewhere: a running
     * animation jumps to its end, a gesture still under the fingers rolls
     * back (and re-renders). Otherwise no held re-render runs — the caller is
     * about to render. True when a gesture was interrupted, so the DOM no
     * longer shows the previous workspace.
     */
    finishNow() {
      if (!stage) return false;
      renderHeld = false;
      const gesture = stage.kind === 'gesture';
      stage.transition?.finishNow(!gesture);
      return gesture;
    },
  };
}
