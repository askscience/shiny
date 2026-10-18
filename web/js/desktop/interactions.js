/** Floating window drag and resize; geometry persistence is supplied by the layout controller. */
export function createWindowInteractions({ getDesktopLayout, raiseWindow, geomFor, clampNum, flushGeom, saveGeomSoon }) {
  const WIN_MIN_W = 280;
  const WIN_MIN_H = 200;
  const WIN_TITLE_H = 36; // keep in sync with --tile-header-height in tiles.css

  /* A merged title bar hosts the plugin's own toolbar, so its controls must not
     start a window drag: only empty space in the bar is a drag handle. */
  const DRAG_IGNORE = 'button, a, input, textarea, select, label, '
    + '[contenteditable], [role="button"], [role="textbox"], [role="slider"], '
    + '[role="tab"], [role="menu"], .ui-btn';

  function wireWindowInteractions(el, name) {
    if (el.__windowWired) return;
    el.__windowWired = true;

    const windowsMode = () => getDesktopLayout().mode === 'windows';
    // A fullscreen window owns the screen *of its own workspace*: dragging or
    // resizing would fight the fullscreen geometry (fullscreen.js drives it, and
    // CSS pins the chrome). `fs-active` is that workspace-scoped state — the
    // browser's own fullscreen says nothing about the workspace on view, so
    // floating windows stay draggable on a normal one.
    const locked = () => document.body.classList.contains('fs-active');

    // Clicking anywhere on the window raises it to the front (windows mode only).
    el.addEventListener('pointerdown', () => {
      if (windowsMode()) raiseWindow(name, el);
    }, { capture: true });

    const header = el.querySelector(':scope > .tile-header');
    const resize = el.querySelector(':scope > .tile-resize');

    if (header) {
      header.addEventListener('pointerdown', (e) => {
        if (locked() || !windowsMode() || e.button !== 0 || e.target.closest(DRAG_IGNORE)) return;
        e.preventDefault();
        startWindowDrag(e, el, name, header);
      });
    }
    if (resize) {
      resize.addEventListener('pointerdown', (e) => {
        if (locked() || !windowsMode() || e.button !== 0) return;
        e.preventDefault();
        e.stopPropagation();
        startWindowResize(e, el, name);
      });
    }
  }

  function startWindowDrag(e, el, name, header) {
    const g = geomFor(name, 0);
    const grid = el.parentElement;
    const W = grid.clientWidth || window.innerWidth;
    const H = grid.clientHeight || window.innerHeight;
    const startX = e.clientX;
    const startY = e.clientY;
    const origX = g.x;
    const origY = g.y;

    header.setPointerCapture?.(e.pointerId);
    header.classList.add('is-dragging');
    el.classList.add('is-dragging');

    // Drag the window on the compositor, one style update per frame.
    //
    // Two things were measured here and both were wrong. First, writing `left`
    // and `top` invalidates layout, so every write forced the whole tile subtree
    // to be re-laid-out and its blurred panes re-rasterised; a compositor-only
    // `transform` moves the already-painted layer instead. Second, the writes ran
    // once per `pointermove`, and pointermove fires several times per presented
    // frame (1768 events in one 12 s drag here) — so the work was a multiple of
    // what any display could show. Coalescing into one rAF tick bounds it to at
    // most one update per frame.
    //
    // Result of doing both: dragging a plugin window went from 4.6 fps with 50
    // stalls over 50 ms to a steady 30 fps with none.
    let dx = 0;
    let dy = 0;
    let pending = null;
    let queued = false;

    const apply = () => {
      queued = false;
      if (pending === null) return;
      const { x, y } = pending;
      pending = null;
      // The element is positioned at (origX, origY) by `left`/`top`; the drag is
      // expressed as an offset from there, which the compositor can apply without
      // touching layout.
      //
      // Two-dimensional `translate`, deliberately not `translate3d`. The 3D form
      // forces the tile into its own GPU layer, and this window carries an ambient
      // blurred glow (`.tile-glow`) that overhangs the window by 6% and sits at
      // z-index -1 inside it. Promoting the tile puts that overhanging layer
      // through a different compositing path than it was authored for, which is a
      // real visual change for a performance win that `translate` already gives
      // (a 2D transform is still a compositor-only operation).
      el.style.transform = `translate(${x}px, ${y}px)`;
    };

    const move = (ev) => {
      dx = clampNum(origX + (ev.clientX - startX), 0, Math.max(0, W - g.w)) - origX;
      dy = clampNum(origY + (ev.clientY - startY), 0, Math.max(0, H - WIN_TITLE_H)) - origY;
      pending = { x: dx, y: dy };
      if (!queued) {
        queued = true;
        requestAnimationFrame(apply);
      }
    };
    const up = () => {
      header.removeEventListener('pointermove', move);
      header.removeEventListener('pointerup', up);
      header.removeEventListener('pointercancel', up);
      header.classList.remove('is-dragging');
      el.classList.remove('is-dragging');

      // Drain any queued frame synchronously (rather than cancelling it by id we
      // would have to track), so the committed position is the final one.
      if (queued) apply();
      pending = null;
      g.x = clampNum(origX + dx, 0, Math.max(0, W - g.w));
      g.y = clampNum(origY + dy, 0, Math.max(0, H - WIN_TITLE_H));
      el.style.transform = '';
      el.style.left = `${g.x}px`;
      el.style.top = `${g.y}px`;
      flushGeom();
    };
    header.addEventListener('pointermove', move);
    header.addEventListener('pointerup', up);
    header.addEventListener('pointercancel', up);
  }

  function startWindowResize(e, el, name) {
    const g = geomFor(name, 0);
    const grid = el.parentElement;
    const W = grid.clientWidth || window.innerWidth;
    const H = grid.clientHeight || window.innerHeight;
    const startX = e.clientX;
    const startY = e.clientY;
    const origW = g.w;
    const origH = g.h;

    el.setPointerCapture?.(e.pointerId);
    el.classList.add('is-resizing');

    const move = (ev) => {
      g.w = clampNum(origW + (ev.clientX - startX), WIN_MIN_W, Math.max(WIN_MIN_W, W - g.x));
      g.h = clampNum(origH + (ev.clientY - startY), WIN_MIN_H, Math.max(WIN_MIN_H, H - g.y));
      el.style.width = `${g.w}px`;
      el.style.height = `${g.h}px`;
      saveGeomSoon();
    };
    const up = () => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
      el.removeEventListener('pointercancel', up);
      el.classList.remove('is-resizing');
      flushGeom();
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
  }


  return wireWindowInteractions;
}
