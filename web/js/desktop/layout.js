import { createWindowInteractions } from './interactions.js';
/** Mutable geometry is owned by the workspace session; rendering owns no user identity. */
export function createLayoutController(state, { getDesktopLayout, setWindowsGeom, syncActiveFocus, persist, setLayout }) {
  const wireWindowInteractions = createWindowInteractions({getDesktopLayout, raiseWindow, geomFor, clampNum, flushGeom, saveGeomSoon});
  const WIN_MIN_W = 280;
  const WIN_MIN_H = 200;
  const WIN_TITLE_H = 36; // keep in sync with --tile-header-height in tiles.css

  function clampNum(n, min, max) {
    return Math.min(max, Math.max(min, n));
  }

  function defaultWindowGeom(index) {
    const step = 34;
    return {
      x: 40 + (index % 6) * step,
      y: 40 + (index % 6) * step,
      w: 560,
      h: 400,
      z: 0,
    };
  }

  function geomFor(name, index) {
    let g = state.windowGeom[name];
    if (!g || typeof g !== 'object'
      || !Number.isFinite(Number(g.x)) || !Number.isFinite(Number(g.y))
      || !Number.isFinite(Number(g.w)) || !Number.isFinite(Number(g.h))) {
      g = defaultWindowGeom(index);
      state.windowGeom[name] = g;
    }
    g.x = Number(g.x);
    g.y = Number(g.y);
    g.w = Number(g.w);
    g.h = Number(g.h);
    g.z = Number(g.z) || 0;
    // Assign a unique stacking order to windows that never had one, so a
    // freshly laid-out window sits deterministically above earlier ones.
    if (g.z <= 0) {
      state.zSeq += 1;
      g.z = state.zSeq;
    }
    return g;
  }

  function flushGeom() {
    if (state.geomTimer) { clearTimeout(state.geomTimer); state.geomTimer = null; }
    setWindowsGeom(state.windowGeom);
  }

  function saveGeomSoon() {
    if (state.geomTimer) return;
    state.geomTimer = setTimeout(() => {
      state.geomTimer = null;
      setWindowsGeom(state.windowGeom);
    }, 250);
  }

  /** Bring a window to the front (z-order) without a DOM pass; the next
   *  layout render applies the stored z-index. */
  function bumpZ(name) {
    if (!name) return;
    const g = geomFor(name, 0);
    state.zSeq += 1;
    g.z = state.zSeq;
    saveGeomSoon();
  }

  function applyWindowsLayout(grid, items, fs) {
    grid.style.display = 'block';
    grid.style.gridTemplateColumns = '';
    grid.style.gridTemplateRows = '';

    const W = grid.clientWidth || window.innerWidth;
    const H = grid.clientHeight || window.innerHeight;

    for (const it of items) {
      const el = it.el;
      el.style.gridColumn = '';
      el.style.gridRow = '';
      el.classList.remove('tile--master', 'tile--stack', 'hidden');
      el.classList.add('tile--window');
      wireWindowInteractions(el, it.name);
    }

    if (fs) {
      for (const it of items) {
        const shown = it.name === fs;
        it.el.classList.toggle('hidden', !shown);
        it.el.classList.toggle('tile--full', shown);
        if (shown) {
          it.el.style.left = '0px';
          it.el.style.top = '0px';
          it.el.style.width = '100%';
          it.el.style.height = '100%';
          it.el.style.zIndex = '1';
        }
      }
      markFocus(items);
      return;
    }

    for (const [i, it] of items.entries()) {
      const el = it.el;
      const g = geomFor(it.name, i);
      g.w = clampNum(g.w, WIN_MIN_W, W);
      g.h = clampNum(g.h, WIN_MIN_H, H);
      g.x = clampNum(g.x, 0, Math.max(0, W - g.w));
      g.y = clampNum(g.y, 0, Math.max(0, H - WIN_TITLE_H));
      el.classList.remove('tile--full');
      // Clear any in-flight drag offset before positioning from geometry: a
      // layout pass can land mid-drag (a workspace switch, a desktop resize), and
      // leaving the offset applied would place the window wrongly.
      if (el.style.transform) el.style.transform = '';
      el.style.left = `${g.x}px`;
      el.style.top = `${g.y}px`;
      el.style.width = `${g.w}px`;
      el.style.height = `${g.h}px`;
      el.style.zIndex = String(g.z || 1);
    }
    markFocus(items);
  }

  /**
   * Apply just the focus state to the mounted windows.
   *
   * A focus click changes one thing: which window carries `tile--focused` (plus,
   * in the floating layout, its z-index, which `bumpZ` already stored). It does
   * not change any window's position, size, the workspace bar, or the dock — yet
   * it used to go through `notify()` and a full `renderTiles()`, which re-applies
   * every window's geometry and rebuilds the whole workspace-bar DOM. That cost
   * 2.5 ms with two windows and 7.6 ms with eleven, entirely for nothing.
   *
   * Exported so the tiles layer can take this path on a focus-only change.
   */
  function repaintFocus() {
    document.querySelectorAll('#tile-grid .tile').forEach((el) => {
      el.classList.toggle('tile--focused', el.dataset.plugin === state.focus);
    });
  }

  /** Focus (raise) a floating window without a full re-render, so an in-flight
   *  drag isn't torn down by the layout pass. */
  function raiseWindow(name, el) {
    if (!name) return;
    if (state.focus !== name) {
      state.focus = name;
      syncActiveFocus();
      persist();
    }
    const g = geomFor(name, 0);
    state.zSeq += 1;
    g.z = state.zSeq;
    saveGeomSoon();
    if (el) {
      el.style.zIndex = String(g.z);
      const grid = el.parentElement;
      if (grid) {
        grid.querySelectorAll('.tile').forEach((t) => {
          t.classList.toggle('tile--focused', t.dataset.plugin === name);
        });
      }
    }
  }

  /* ── Resizable splits (drag any gap) ────────────────────────── */

  const SPLITTER_MIN_HIT = 12; // px of grab area even when the gap is thin
  const RATIO_MIN = 0.25;      // keep in sync with the clamp in preferences.js
  const RATIO_MAX = 0.85;
  const STACK_MIN_SIZE = 80;   // px a stacked window may shrink to

  function isHorizontalSplit(layout) {
    return layout.orientation === 'top' || layout.orientation === 'bottom';
  }

  /** True when the master pane is the first (left/top) track. */
  function masterFirst(layout) {
    return layout.orientation === 'left' || layout.orientation === 'top';
  }

  /** The transparent overlay that hosts every drag handle. */
  function ensureSplitterLayer(grid) {
    let layer = grid.__splitterLayer;
    if (layer && layer.isConnected) return layer;
    layer = document.createElement('div');
    layer.className = 'tile-splitters';
    layer.setAttribute('aria-hidden', 'true');
    layer.__splitters = [];
    grid.__splitterLayer = layer;
    grid.appendChild(layer);
    return layer;
  }

  /** Drop every handle; called for any layout that has no split. */
  function clearSplitters(grid) {
    const layer = ensureSplitterLayer(grid);
    layer.textContent = '';
    layer.__splitters = [];
    grid.__splitCtx = null;
  }

  /** Create one invisible handle with the right resize cursor. */
  function addSplitter(grid, rec) {
    const layer = ensureSplitterLayer(grid);
    const el = document.createElement('div');
    el.className = 'tile-splitter';
    el.style.cursor = rec.axis === 'x' ? 'col-resize' : 'row-resize';
    el.addEventListener('pointerdown', rec.onDown);
    layer.appendChild(el);
    layer.__splitters.push({ ...rec, el });
  }

  function place(el, x, y, w, h) {
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    el.style.width = `${w}px`;
    el.style.height = `${h}px`;
  }

  /**
   * Build the handles for the current split: one on the master↔stack seam and
   * one in every gap between adjacent stacked windows. They are invisible — only
   * the cursor changes over them.
   */
  function buildSplitters(grid, layout, master, stack) {
    clearSplitters(grid);
    if (!master || !stack.length) return;
    grid.__splitCtx = { master, stack };

    // Left/right split → vertical seam dragged horizontally; top/bottom split →
    // horizontal seam dragged vertically. The stacked windows run the other way.
    const horizontal = isHorizontalSplit(layout);
    addSplitter(grid, {
      kind: 'master',
      axis: horizontal ? 'y' : 'x',
      side: layout.orientation,
      onDown: (e) => startMasterSeamDrag(e, grid, master, stack),
    });
    for (let i = 0; i < stack.length - 1; i += 1) {
      addSplitter(grid, {
        kind: 'stack',
        axis: horizontal ? 'x' : 'y',
        index: i,
        onDown: (e) => startStackDrag(e, grid, master, stack, i),
      });
    }

    positionSplitters(grid, layout, master, stack);
  }

  /** Re-position the existing handles from the windows' current boxes. */
  function positionSplitters(grid, layout, master, stack) {
    const layer = grid.__splitterLayer;
    if (!layer || !layer.__splitters.length) return;
    if (!master?.el?.isConnected || stack.some((it) => !it.el.isConnected)) return;

    const base = grid.getBoundingClientRect();
    const rel = (el) => {
      const r = el.getBoundingClientRect();
      return {
        l: r.left - base.left, t: r.top - base.top,
        r: r.right - base.left, b: r.bottom - base.top,
      };
    };
    const hit = Math.max(layout.gap, SPLITTER_MIN_HIT);
    const m = rel(master.el);
    const s = stack.map((it) => rel(it.el));

    for (const rec of layer.__splitters) {
      if (rec.kind === 'master') {
        if (rec.axis === 'x') {
          const cx = rec.side === 'right' ? (m.l + s[0].r) / 2 : (m.r + s[0].l) / 2;
          place(rec.el, cx - hit / 2, 0, hit, base.height);
        } else {
          const cy = rec.side === 'bottom' ? (m.t + s[0].b) / 2 : (m.b + s[0].t) / 2;
          place(rec.el, 0, cy - hit / 2, base.width, hit);
        }
      } else if (rec.axis === 'y') {
        const cy = (s[rec.index].b + s[rec.index + 1].t) / 2;
        const left = Math.min(s[rec.index].l, s[rec.index + 1].l);
        const right = Math.max(s[rec.index].r, s[rec.index + 1].r);
        place(rec.el, left, cy - hit / 2, right - left, hit);
      } else {
        const cx = (s[rec.index].r + s[rec.index + 1].l) / 2;
        const top = Math.min(s[rec.index].t, s[rec.index + 1].t);
        const bottom = Math.max(s[rec.index].b, s[rec.index + 1].b);
        place(rec.el, cx - hit / 2, top, hit, bottom - top);
      }
    }
  }

  /** Re-position after a viewport change without a full re-layout. */
  function repositionSplitters(grid) {
    const ctx = grid.__splitCtx;
    if (!ctx) return;
    positionSplitters(grid, getDesktopLayout(), ctx.master, ctx.stack);
  }

  /** Track weights for the stacked windows, keyed by plugin name. */
  function stackTrackWeights(layout, stack, override) {
    const stored = override || layout.stack_weights || {};
    const known = stack
      .map((it) => Number(stored[it.name]))
      .filter((v) => Number.isFinite(v) && v > 0);
    const fallback = known.length ? known.reduce((a, b) => a + b, 0) / known.length : 1;
    return stack.map((it) => {
      const v = Number(stored[it.name]);
      return Number.isFinite(v) && v > 0 ? v : fallback;
    });
  }

  /** Set the stacked windows' tracks from weights (live drag path). */
  function applyStackWeights(grid, layout, master, stack, weights) {
    const tracks = stackTrackWeights(layout, stack, weights).map((w) => `minmax(0, ${w}fr)`);
    if (isHorizontalSplit(layout)) grid.style.gridTemplateColumns = tracks.join(' ');
    else grid.style.gridTemplateRows = tracks.join(' ');
    positionSplitters(grid, layout, master, stack);
  }

  /** Set the master/stack tracks from a live ratio (live drag path). */
  function applyMasterRatio(grid, layout, ratio, master, stack) {
    const track = (f) => `minmax(0, ${f}fr)`;
    const a = masterFirst(layout) ? ratio : 1 - ratio;
    if (isHorizontalSplit(layout)) grid.style.gridTemplateRows = `${track(a)} ${track(1 - a)}`;
    else grid.style.gridTemplateColumns = `${track(a)} ${track(1 - a)}`;
    positionSplitters(grid, { ...layout, master_ratio: ratio }, master, stack);
  }

  function startMasterSeamDrag(e, grid, master, stack) {
    if (e.button !== 0) return;
    const layout = getDesktopLayout();
    if (layout.mode !== 'master') return;
    e.preventDefault();
    e.stopPropagation();

    const divider = e.currentTarget;
    const axis = isHorizontalSplit(layout) ? 'y' : 'x';
    const base = grid.getBoundingClientRect();
    const origin = axis === 'x' ? base.left : base.top;
    const span = axis === 'x' ? base.width : base.height;
    const gap = layout.gap;
    const avail = span - gap;
    const first = masterFirst(layout);
    let ratio = layout.master_ratio;

    divider.setPointerCapture?.(e.pointerId);
    document.body.classList.add('is-tiling-resize');
    document.body.style.cursor = axis === 'x' ? 'col-resize' : 'row-resize';

    const move = (ev) => {
      if (avail <= 0) return;
      const pos = (axis === 'x' ? ev.clientX : ev.clientY) - origin;
      const frac = clampNum((pos - gap / 2) / avail, 0.05, 0.95);
      ratio = clampNum(first ? frac : 1 - frac, RATIO_MIN, RATIO_MAX);
      applyMasterRatio(grid, layout, ratio, master, stack);
    };

    const up = () => {
      divider.removeEventListener('pointermove', move);
      divider.removeEventListener('pointerup', up);
      divider.removeEventListener('pointercancel', up);
      divider.releasePointerCapture?.(e.pointerId);
      document.body.classList.remove('is-tiling-resize');
      document.body.style.cursor = '';
      if (Math.abs(ratio - layout.master_ratio) > 0.001) setLayout({ master_ratio: ratio });
    };

    divider.addEventListener('pointermove', move);
    divider.addEventListener('pointerup', up);
    divider.addEventListener('pointercancel', up);
  }

  function startStackDrag(e, grid, master, stack, index) {
    if (e.button !== 0) return;
    const layout = getDesktopLayout();
    if (layout.mode !== 'master') return;
    e.preventDefault();
    e.stopPropagation();

    const divider = e.currentTarget;
    const axis = isHorizontalSplit(layout) ? 'x' : 'y'; // drag axis
    const sizes = stack.map((it) => {
      const r = it.el.getBoundingClientRect();
      return axis === 'x' ? r.width : r.height;
    });
    // Snapshot every track so the untouched windows keep their exact sizes.
    const weights = {};
    stack.forEach((it, k) => { weights[it.name] = sizes[k] || 1; });

    const pairTotal = sizes[index] + sizes[index + 1];
    const minSize = Math.min(STACK_MIN_SIZE, pairTotal / 2);
    const before = sizes[index];
    const start = axis === 'x' ? e.clientX : e.clientY;

    divider.setPointerCapture?.(e.pointerId);
    document.body.classList.add('is-tiling-resize');
    document.body.style.cursor = axis === 'x' ? 'col-resize' : 'row-resize';

    const move = (ev) => {
      const delta = (axis === 'x' ? ev.clientX : ev.clientY) - start;
      const newBefore = clampNum(before + delta, minSize, pairTotal - minSize);
      weights[stack[index].name] = newBefore;
      weights[stack[index + 1].name] = pairTotal - newBefore;
      applyStackWeights(grid, layout, master, stack, weights);
    };

    const up = () => {
      divider.removeEventListener('pointermove', move);
      divider.removeEventListener('pointerup', up);
      divider.removeEventListener('pointercancel', up);
      divider.releasePointerCapture?.(e.pointerId);
      document.body.classList.remove('is-tiling-resize');
      document.body.style.cursor = '';
      setLayout({ stack_weights: { ...getDesktopLayout().stack_weights, ...weights } });
    };

    divider.addEventListener('pointermove', move);
    divider.addEventListener('pointerup', up);
    divider.addEventListener('pointercancel', up);
  }

  /**
   * Arrange tile elements inside `grid`. `items` = [{ name, el }] for the active
   * workspace's windows (already mounted). Handles fullscreen, single-window,
   * legacy columns, master/stack, and the floating "Windows" desktop.
   */
  function applyLayout(grid, items) {
    const layout = getDesktopLayout();
    const ratio = layout.master_ratio;
    const ori = layout.orientation;

    // The gap itself is CSS (`--desktop-gap`, written from the layout config) so
    // that the window gaps and the desktop's outer frame always agree.
    grid.classList.remove('tile-grid--master', 'tile-grid--columns', 'tile-grid--windows');
    grid.classList.add(
      layout.mode === 'master' ? 'tile-grid--master'
        : layout.mode === 'windows' ? 'tile-grid--windows'
        : 'tile-grid--columns'
    );
    grid.dataset.layout = layout.mode;
    // The handles are only meaningful for a master/stack split; every other path
    // below leaves the overlay empty.
    clearSplitters(grid);

    // A fullscreen state that does not name a window of THIS workspace is never
    // rendered: a stale pointer is ignored rather than allowed to make whichever
    // window opens next come up fullscreen. (It is repaired by `reconcile()`
    // before the next workspace switch or activation.)
    const fs = state.fullscreen && items.some((i) => i.name === state.fullscreen) ? state.fullscreen : null;

    if (layout.mode === 'windows') {
      applyWindowsLayout(grid, items, fs);
      return;
    }

    // Tiling modes — strip any floating-window geometry/inline styles so the
    // grid/flex engine owns positioning again.
    for (const it of items) {
      it.el.style.left = '';
      it.el.style.top = '';
      it.el.style.width = '';
      it.el.style.height = '';
      it.el.style.zIndex = '';
      it.el.classList.remove('tile--window');
      it.el.style.gridColumn = '';
      it.el.style.gridRow = '';
      it.el.classList.remove('tile--master', 'tile--stack', 'tile--full', 'hidden');
    }

    if (fs) {
      for (const it of items) {
        it.el.classList.toggle('hidden', it.name !== fs);
        if (it.name === fs) it.el.classList.add('tile--full');
      }
      grid.style.display = 'grid';
      grid.style.gridTemplateColumns = 'minmax(0, 1fr)';
      grid.style.gridTemplateRows = 'minmax(0, 1fr)';
      return;
    }

    if (items.length <= 1) {
      if (items.length === 1) items[0].el.classList.add('tile--full');
      grid.style.display = 'grid';
      grid.style.gridTemplateColumns = 'minmax(0, 1fr)';
      grid.style.gridTemplateRows = 'minmax(0, 1fr)';
      return;
    }

    if (layout.mode !== 'master') {
      // Legacy columns: CSS flex-wrap (see tiles.css) owns the layout.
      grid.style.display = 'flex';
      grid.style.gridTemplateColumns = '';
      grid.style.gridTemplateRows = '';
      markFocus(items);
      return;
    }

    // Master/stack — the focused window (or first) is the master.
    const masterName = (state.focus && items.some((i) => i.name === state.focus)) ? state.focus : items[0].name;
    const master = items.find((i) => i.name === masterName) || items[0];
    const stack = items.filter((i) => i !== master);

    grid.style.display = 'grid';
    const stackTracks = stackTrackWeights(layout, stack)
      .map((w) => `minmax(0, ${w}fr)`).join(' ');

    if (ori === 'top' || ori === 'bottom') {
      grid.style.gridTemplateColumns = stackTracks;
      grid.style.gridTemplateRows = ori === 'top'
        ? `minmax(0, ${ratio}fr) minmax(0, ${1 - ratio}fr)`
        : `minmax(0, ${1 - ratio}fr) minmax(0, ${ratio}fr)`;
      const masterRow = ori === 'top' ? '1' : '2';
      master.el.style.gridRow = `${masterRow} / span 1`;
      master.el.style.gridColumn = '1 / -1';
      stack.forEach((it, i) => {
        it.el.style.gridRow = ori === 'top' ? '2' : '1';
        it.el.style.gridColumn = `${i + 1}`;
      });
    } else {
      const masterCol = ori === 'right' ? '2' : '1';
      const stackCol = ori === 'right' ? '1' : '2';
      grid.style.gridTemplateColumns = ori === 'right'
        ? `minmax(0, ${1 - ratio}fr) minmax(0, ${ratio}fr)`
        : `minmax(0, ${ratio}fr) minmax(0, ${1 - ratio}fr)`;
      grid.style.gridTemplateRows = stackTracks;
      master.el.style.gridColumn = `${masterCol}`;
      master.el.style.gridRow = '1 / -1';
      stack.forEach((it, i) => {
        it.el.style.gridColumn = `${stackCol}`;
        it.el.style.gridRow = `${i + 1}`;
      });
    }

    master.el.classList.add('tile--master');
    stack.forEach((it) => it.el.classList.add('tile--stack'));
    buildSplitters(grid, layout, master, stack);
    markFocus(items);
  }

  function markFocus(items) {
    for (const it of items) {
      it.el.classList.toggle('tile--focused', it.name === state.focus);
    }
  }


  return { bumpZ, applyLayout, repaintFocus, flushGeom, repositionSplitters };
}
