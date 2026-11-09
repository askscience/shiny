/**
 * timeline — tracks, clips, a ruler and a playhead, for any editor.
 *
 *   const tl = timeline({
 *     scale: secondsScale({ pps: 60 }),
 *     makeScale: (zoom) => secondsScale({ pps: zoom }),
 *     zoom: 60, minZoom: 2, maxZoom: 600,
 *     tracks: [{ id: 'v1', label: 'Video', height: 56 }],
 *     items: [{ id: 'c1', track: 'v1', start: 0, end: 4, label: 'Intro' }],
 *     onSelect: (id, e) => {}, onMove: (id, { start, track }) => {},
 *     onTrim: (id, { start, end }) => {}, onSeek: (t) => {},
 *     renderItem: (el, item, { scale }) => {},   // waveforms, thumbnails
 *     renderHead: (track) => Node,
 *     contextMenu: (id, e) => {},
 *   });
 *   tl.setItems(items); tl.setPlayhead(t); tl.zoomToFit(); tl.destroy();
 *
 * Drag a clip to move it (across tracks too), its edges to trim it; the
 * ruler and empty lanes seek. Ctrl+wheel and every kind of pinch zoom
 * around the pointer ([`wireZoomGestures`], shared with Studio's arranger).
 * Only clips in view are laid out, so long projects stay light.
 */

const HEAD_W = 132;
const EDGE = 7;

/**
 * Pinch and Ctrl+wheel zoom for a scrolling element, by every path a pinch
 * can arrive: Ctrl+wheel (mouse, and Chromium's trackpad pinch), WebKit
 * `gesture*` events, two touch pointers, the LiteAura shell's
 * `trackpad:pinch` event (Qt on X11 never hands touchpad pinches to the
 * page), and as a last resort the visual viewport's scale.
 *
 * @param {HTMLElement} el
 * @param {{ getZoom: () => number, setZoom: (z: number, clientX: number) => void, centerX?: () => number }} o
 * @returns {() => void} remover
 */
export function wireZoomGestures(el, { getZoom, setZoom, centerX }) {
  let pointer = { x: null, inside: false };
  const touches = new Map();
  let base = null; // { zoom, x, dist? }
  const px = () => {
    if (pointer.x != null) return pointer.x;
    if (centerX) return centerX();
    const r = el.getBoundingClientRect();
    return r.left + r.width / 2;
  };
  const begin = (x) => { base = { zoom: getZoom(), x }; };
  const scale = (s) => {
    if (!base || !Number.isFinite(s) || s <= 0) return;
    setZoom(base.zoom * s, base.x);
  };
  const on = [];
  const add = (target, type, fn, opts) => { target.addEventListener(type, fn, opts); on.push([target, type, fn, opts]); };
  add(el, 'pointermove', (e) => { pointer = { x: e.clientX, inside: true }; }, { passive: true });
  add(el, 'pointerleave', () => { pointer.inside = false; }, { passive: true });
  add(el, 'wheel', (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    e.preventDefault();
    const factor = Math.exp(-e.deltaY * (e.deltaMode === 1 ? 0.05 : 0.004));
    setZoom(getZoom() * factor, e.clientX);
  }, { passive: false });
  add(el, 'gesturestart', (e) => { e.preventDefault(); begin(e.clientX ?? px()); }, { passive: false });
  add(el, 'gesturechange', (e) => { e.preventDefault(); scale(e.scale); }, { passive: false });
  add(el, 'gestureend', (e) => { e.preventDefault(); base = null; }, { passive: false });
  const dist = () => { const [a, b] = [...touches.values()]; return Math.hypot(a.x - b.x, a.y - b.y); };
  add(el, 'pointerdown', (e) => {
    if (e.pointerType !== 'touch') return;
    touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (touches.size === 2) {
      const [a, b] = [...touches.values()];
      begin((a.x + b.x) / 2);
      base.dist = Math.max(1, dist());
    }
  }, true);
  add(el, 'pointermove', (e) => {
    if (e.pointerType !== 'touch' || !touches.has(e.pointerId)) return;
    touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (touches.size === 2 && base?.dist) { e.preventDefault(); scale(dist() / base.dist); }
  }, true);
  const lift = (e) => { if (touches.delete(e.pointerId) && touches.size < 2) base = null; };
  add(el, 'pointerup', lift, true);
  add(el, 'pointercancel', lift, true);
  add(window, 'trackpad:pinch', (e) => {
    if (!el.isConnected || !pointer.inside) return;
    const d = e.detail || {};
    if (d.phase === 'begin') begin(px());
    else if (d.phase === 'update') { if (!base) begin(px()); scale(Number(d.scale)); }
    else base = null;
  });
  let lastScale = window.visualViewport?.scale || 1;
  if (window.visualViewport) {
    add(window.visualViewport, 'resize', () => {
      const sc = window.visualViewport.scale || 1;
      if (sc === lastScale) return;
      const ratio = sc / lastScale;
      lastScale = sc;
      if (pointer.inside && el.isConnected) setZoom(getZoom() * ratio, px());
    });
  }
  return () => { for (const [t, type, fn, opts] of on) t.removeEventListener(type, fn, opts); };
}

function div(cls) {
  const d = document.createElement('div');
  d.className = cls;
  return d;
}

export function timeline(o = {}) {
  const makeScale = o.makeScale;
  let zoom = o.zoom ?? o.scale?.pps ?? 60;
  const minZoom = o.minZoom ?? 1;
  const maxZoom = o.maxZoom ?? 800;
  let scale = o.scale || makeScale(zoom);
  let tracks = o.tracks || [];
  let items = o.items || [];
  let duration = o.duration ?? 0;
  let playhead = 0;
  let selected = new Set(o.selected || []);

  const root = div(`ui-timeline${o.className ? ` ${o.className}` : ''}`);
  root.style.setProperty('--tl-head', `${HEAD_W}px`);
  root.tabIndex = 0;
  const scroller = div('ui-tl-scroll');
  const content = div('ui-tl-content');
  const ruler = div('ui-tl-ruler');
  const rulerTicks = div('ui-tl-ruler-ticks');
  const corner = div('ui-tl-corner');
  ruler.append(corner, rulerTicks);
  const lanes = div('ui-tl-lanes');
  const head = div('ui-tl-playhead');
  head.setAttribute('aria-hidden', 'true');
  const marks = div('ui-tl-marks');
  content.append(ruler, lanes, marks, head);
  scroller.appendChild(content);
  root.appendChild(scroller);

  const laneEls = new Map();
  const itemEls = new Map();

  const viewEnd = () => Math.max(duration, ...items.map((i) => i.end), 0) + Math.max(10, scale.fromPx((scroller.clientWidth || 800) * 0.5));
  const xOf = (t) => HEAD_W + scale.toPx(t);
  const tOf = (clientX) => {
    const r = scroller.getBoundingClientRect();
    return Math.max(0, scale.fromPx(clientX - r.left + scroller.scrollLeft - HEAD_W));
  };

  function layoutRuler() {
    const w = scale.toPx(viewEnd());
    content.style.width = `${HEAD_W + w}px`;
    const from = scale.fromPx(Math.max(0, scroller.scrollLeft - 40));
    const to = scale.fromPx(scroller.scrollLeft + (scroller.clientWidth || 800) + 40);
    rulerTicks.textContent = '';
    for (const tk of scale.ticks(from, to)) {
      const d = div(`ui-tl-tick${tk.major ? ' is-major' : ''}`);
      d.style.transform = `translateX(${scale.toPx(tk.t)}px)`;
      if (tk.label) {
        const l = document.createElement('span');
        l.textContent = tk.label;
        d.appendChild(l);
      }
      rulerTicks.appendChild(d);
    }
  }

  function layoutLanes() {
    const keep = new Set(tracks.map((t) => t.id));
    for (const [id, el] of laneEls) if (!keep.has(id)) { el.remove(); laneEls.delete(id); }
    tracks.forEach((tr, i) => {
      let lane = laneEls.get(tr.id);
      if (!lane) {
        lane = div('ui-tl-lane');
        lane.dataset.track = tr.id;
        const h = div('ui-tl-head');
        const body = div('ui-tl-lane-body');
        lane.append(h, body);
        lane.head = h;
        lane.body = body;
        laneEls.set(tr.id, lane);
      }
      lane.style.height = `${tr.height || 52}px`;
      lane.classList.toggle('is-muted', !!tr.muted);
      lane.classList.toggle('is-hidden', !!tr.hidden);
      lane.dataset.kind = tr.kind || '';
      lane.head.textContent = '';
      const custom = o.renderHead?.(tr);
      if (custom) lane.head.appendChild(custom);
      else lane.head.textContent = tr.label || tr.id;
      if (lanes.children[i] !== lane) lanes.insertBefore(lane, lanes.children[i] || null);
    });
  }

  function layoutItems() {
    const from = scale.fromPx(scroller.scrollLeft - 200);
    const to = scale.fromPx(scroller.scrollLeft + (scroller.clientWidth || 800) + 200);
    const live = new Set();
    for (const it of items) {
      const lane = laneEls.get(it.track);
      if (!lane || it.end < from || it.start > to) continue;
      live.add(it.id);
      let el = itemEls.get(it.id);
      if (!el) {
        el = div('ui-tl-clip');
        el.dataset.id = it.id;
        el.tabIndex = -1;
        const label = div('ui-tl-clip-label');
        const body = div('ui-tl-clip-body');
        const l = div('ui-tl-edge ui-tl-edge--l');
        const r = div('ui-tl-edge ui-tl-edge--r');
        el.append(body, label, l, r);
        el.labelEl = label;
        el.bodyEl = body;
        itemEls.set(it.id, el);
      }
      if (el.parentNode !== lane.body) lane.body.appendChild(el);
      const x = scale.toPx(it.start);
      const w = Math.max(3, scale.toPx(it.end - it.start));
      el.style.transform = `translateX(${x}px)`;
      el.style.width = `${w}px`;
      el.className = `ui-tl-clip${it.className ? ` ${it.className}` : ''}${selected.has(it.id) ? ' is-selected' : ''}${it.ghost ? ' is-ghost' : ''}${it.locked ? ' is-locked' : ''}`;
      el.labelEl.textContent = it.label || '';
      const key = `${w.toFixed(1)}|${it.start}|${it.end}|${it.version ?? ''}|${scale.pps}`;
      if (o.renderItem && el.dataset.drawn !== key) {
        el.dataset.drawn = key;
        o.renderItem(el.bodyEl, it, { scale, width: w });
      }
    }
    for (const [id, el] of itemEls) if (!live.has(id)) { el.remove(); itemEls.delete(id); }
  }

  function layoutMarks() {
    marks.textContent = '';
    for (const m of o.markers?.() || []) {
      const d = div(`ui-tl-mark${m.className ? ` ${m.className}` : ''}`);
      d.style.transform = `translateX(${xOf(m.t)}px)`;
      if (m.end != null) d.style.width = `${Math.max(2, scale.toPx(m.end - m.t))}px`;
      d.title = m.label || '';
      marks.appendChild(d);
    }
  }

  function placeHead() {
    head.style.transform = `translateX(${xOf(playhead)}px)`;
  }

  function render() {
    layoutLanes();
    layoutRuler();
    layoutItems();
    layoutMarks();
    placeHead();
  }

  let scrollFrame = 0;
  scroller.addEventListener('scroll', () => {
    if (scrollFrame) return;
    scrollFrame = requestAnimationFrame(() => { scrollFrame = 0; layoutRuler(); layoutItems(); });
  }, { passive: true });

  function setZoom(next, clientX) {
    const z = Math.min(maxZoom, Math.max(minZoom, next));
    if (Math.abs(z - zoom) < 1e-3) return;
    const r = scroller.getBoundingClientRect();
    const x = (clientX ?? r.left + HEAD_W + (r.width - HEAD_W) / 2) - r.left;
    const t = scale.fromPx(scroller.scrollLeft + x - HEAD_W);
    zoom = z;
    scale = makeScale(zoom);
    render();
    scroller.scrollLeft = Math.max(0, scale.toPx(t) + HEAD_W - x);
    o.onZoom?.(zoom);
  }
  const unwire = makeScale ? wireZoomGestures(scroller, {
    getZoom: () => zoom,
    setZoom,
    centerX: () => { const r = scroller.getBoundingClientRect(); return r.left + HEAD_W + (r.width - HEAD_W) / 2; },
  }) : () => {};

  // Pointer: seek, select, move, trim.
  let drag = null;
  scroller.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || (e.pointerType === 'touch' && !e.isPrimary)) return;
    const clipEl = e.target.closest('.ui-tl-clip');
    if (e.target.closest('.ui-tl-head')) return;
    if (!clipEl) {
      if (e.target.closest('.ui-tl-ruler, .ui-tl-lane-body, .ui-tl-lanes')) {
        selected.clear();
        o.onSelect?.(null, e);
        const t = tOf(e.clientX);
        o.onSeek?.(t);
        drag = { kind: 'seek' };
        scroller.setPointerCapture(e.pointerId);
        render();
      }
      return;
    }
    const id = clipEl.dataset.id;
    const it = items.find((x) => x.id === id);
    if (!it) return;
    const r = clipEl.getBoundingClientRect();
    const edge = e.clientX - r.left < EDGE ? 'l' : r.right - e.clientX < EDGE ? 'r' : null;
    if (e.shiftKey) { selected.has(id) ? selected.delete(id) : selected.add(id); }
    else if (!selected.has(id)) { selected = new Set([id]); }
    o.onSelect?.(id, e);
    if (it.locked) { render(); return; }
    drag = { kind: edge ? `trim-${edge}` : 'move', id, x0: e.clientX, y0: e.clientY, start: it.start, end: it.end, track: it.track, moved: false };
    scroller.setPointerCapture(e.pointerId);
    render();
  });
  scroller.addEventListener('pointermove', (e) => {
    if (!drag) {
      const clipEl = e.target.closest?.('.ui-tl-clip');
      if (clipEl) {
        const r = clipEl.getBoundingClientRect();
        const edge = e.clientX - r.left < EDGE || r.right - e.clientX < EDGE;
        clipEl.style.cursor = edge ? 'ew-resize' : '';
      }
      return;
    }
    if (drag.kind === 'seek') { o.onSeek?.(tOf(e.clientX)); return; }
    const it = items.find((x) => x.id === drag.id);
    if (!it) return;
    const dt = scale.fromPx(e.clientX - drag.x0);
    if (!drag.moved && Math.abs(e.clientX - drag.x0) < 3 && Math.abs(e.clientY - drag.y0) < 3) return;
    drag.moved = true;
    const snap = (t) => (o.snap ? o.snap(t, it) : scale.snap(t));
    if (drag.kind === 'move') {
      const len = drag.end - drag.start;
      it.start = Math.max(0, snap(drag.start + dt));
      it.end = it.start + len;
      const over = document.elementFromPoint(e.clientX, e.clientY)?.closest?.('.ui-tl-lane');
      if (over && over.dataset.track && (!o.canDrop || o.canDrop(it, over.dataset.track))) it.track = over.dataset.track;
    } else if (drag.kind === 'trim-l') {
      it.start = Math.min(drag.end - 0.05, Math.max(o.minStart?.(it, drag) ?? 0, snap(drag.start + dt)));
    } else {
      it.end = Math.max(drag.start + 0.05, Math.min(o.maxEnd?.(it, drag) ?? Infinity, snap(drag.end + dt)));
    }
    layoutItems();
  });
  const endDrag = (e) => {
    if (!drag) return;
    const d = drag;
    drag = null;
    try { scroller.releasePointerCapture(e.pointerId); } catch (_) { /* noop */ }
    if (d.kind === 'seek' || !d.moved) return;
    const it = items.find((x) => x.id === d.id);
    if (!it) return;
    if (d.kind === 'move') o.onMove?.(d.id, { start: it.start, track: it.track, from: { start: d.start, track: d.track } });
    else o.onTrim?.(d.id, { start: it.start, end: it.end, edge: d.kind === 'trim-l' ? 'start' : 'end', from: { start: d.start, end: d.end } });
  };
  scroller.addEventListener('pointerup', endDrag);
  scroller.addEventListener('pointercancel', endDrag);
  scroller.addEventListener('contextmenu', (e) => {
    const clipEl = e.target.closest('.ui-tl-clip');
    if (!clipEl || !o.contextMenu) return;
    e.preventDefault();
    if (!selected.has(clipEl.dataset.id)) selected = new Set([clipEl.dataset.id]);
    render();
    o.contextMenu(clipEl.dataset.id, e);
  });
  root.addEventListener('keydown', (e) => o.onKey?.(e));

  const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(() => render()) : null;
  ro?.observe(scroller);

  render();
  return {
    el: root,
    scroller,
    get zoom() { return zoom; },
    get scale() { return scale; },
    get selected() { return [...selected]; },
    setItems(next) { items = next; render(); },
    setTracks(next) { tracks = next; render(); },
    setDuration(d) { duration = d; layoutRuler(); },
    setPlayhead(t, { follow = false } = {}) {
      playhead = Math.max(0, t);
      placeHead();
      if (follow) {
        const x = scale.toPx(playhead);
        const w = scroller.clientWidth - HEAD_W;
        if (x < scroller.scrollLeft || x > scroller.scrollLeft + w * 0.9) scroller.scrollLeft = Math.max(0, x - w * 0.25);
      }
    },
    select(ids) { selected = new Set(Array.isArray(ids) ? ids : [ids].filter(Boolean)); render(); },
    zoomBy(f) { setZoom(zoom * f); },
    setZoom,
    zoomToFit() {
      const end = Math.max(1, duration, ...items.map((i) => i.end));
      const w = Math.max(100, scroller.clientWidth - HEAD_W - 24);
      zoom = Math.min(maxZoom, Math.max(minZoom, w / end));
      scale = makeScale(zoom);
      render();
      scroller.scrollLeft = 0;
      o.onZoom?.(zoom);
    },
    redraw(id) {
      if (id) { const el = itemEls.get(id); if (el) delete el.dataset.drawn; }
      else for (const el of itemEls.values()) delete el.dataset.drawn;
      layoutItems();
    },
    render,
    timeAt: tOf,
    destroy() { ro?.disconnect(); unwire(); if (scrollFrame) cancelAnimationFrame(scrollFrame); root.remove(); },
  };
}
