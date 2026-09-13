/**
 * In-app frame benchmark, injected by the browser only when `--benchmark` is
 * passed. It never runs in a normal launch.
 *
 * Why it lives in the page rather than in an external harness: every attempt to
 * drive this from outside (synthetic CGEvents, Accessibility window moves, an
 * AppleScript bridge) turned into a fight with macOS window occlusion, because
 * a background or occluded WKWebView has `requestAnimationFrame` throttled to a
 * near stop. Measuring from inside the page, in the process that actually
 * paints, removes that entire class of flakiness.
 *
 * What it measures, and what it deliberately does not:
 *   - It counts `requestAnimationFrame` callbacks, which is the rate the
 *     compositor is actually presenting frames at. The app's own animation
 *     shares this cadence, so the number is the browser's, not a private loop's.
 *   - It is *passive*: it never touches the app's DOM, styles or timers. The
 *     only global it creates is `__peakdBench`, so it cannot be mistaken for
 *     app state.
 *   - It records window moves and resizes as they happen, so a slide of the
 *     window during the run is visible in the report rather than averaged away.
 *
 * Collapse in the report is the thing to look at, not the mean: a mean hides a
 * 200 ms stall behind a thousand good frames, and a stall is exactly what a
 * user perceives as "it stutters when I drag it fast".
 */
(function () {
  "use strict";

  if (window.__peakdBench) return; // never double-instrument

  var bench = {
    running: true,
    startedAt: 0,
    frames: 0,
    worstFrameMs: 0,
    worstFrameAtMs: 0,
    /** Frames slower than 32 ms (a 60 Hz budget) and 50 ms (visibly janky). */
    over32: 0,
    over50: 0,
    /** Every inter-frame gap above 32 ms, with when it happened. */
    stalls: [],
    /** Window movement, as seen by the page (viewport resizes). */
    resizes: [],
    longTasks: [],
    started: false,
    durationMs: 0,
    /** rAF callbacks not preceded by a single timer tick, in a window where
     *  the main thread was demonstrably alive. */
    visibilityChanges: [],
    hiddenAtStart: false,
    /**
     * A second, independent canary: a 1 ms busy frame driven by `setInterval`.
     *
     * `setInterval` and `requestAnimationFrame` are governed by *different*
     * throttles. If rAF stalls while this keeps ticking, the page is being
     * frame-throttled (WebKit's occlusion/visibility heuristics, or the
     * compositor). If this stalls *too*, the main thread itself is starved.
     * That single comparison is what separates the two causes.
     */
    timerTicks: 0,
    timerWorstMs: 0,
  };

  window.__peakdBench = bench;

  // Long tasks are the other half of the story: a frame can be late because a
  // task held the main thread, which shows up here rather than in the frame
  // deltas.
  try {
    if (window.PerformanceObserver && PerformanceObserver.supportedEntryTypes &&
        PerformanceObserver.supportedEntryTypes.indexOf("longtask") !== -1) {
      new PerformanceObserver(function (list) {
        list.getEntries().forEach(function (e) {
          bench.longTasks.push({
            atMs: Math.round(e.startTime - bench.startedAt),
            ms: Math.round(e.duration),
          });
        });
      }).observe({ entryTypes: ["longtask"] });
    }
  } catch (e) {}

  bench.hiddenAtStart = !!document.hidden;
  document.addEventListener("visibilitychange", function () {
    if (!bench.started) return;
    bench.visibilityChanges.push({
      atMs: Math.round(performance.now() - bench.startedAt),
      state: document.visibilityState,
    });
  });

  // A resize during the run is a strong hint that whoever is measuring is
  // dragging the window; recording it lets the report separate "slow while
  // moving" from "slow in general".
  window.addEventListener("resize", function () {
    if (!bench.started || !bench.running) return;
    var now = performance.now();
    bench.resizes.push({
      atMs: Math.round(now - bench.startedAt),
      w: window.innerWidth,
      h: window.innerHeight,
    });
  });

  // Independent of rAF: a plain timer doing a tiny, fixed amount of work. It
  // answers "was the main thread alive?" for any window in which rAF produced
  // nothing.
  var timerLast = performance.now();
  setInterval(function () {
    if (!bench.started || !bench.running) return;
    var now = performance.now();
    var dt = now - timerLast;
    timerLast = now;
    bench.timerTicks++;
    if (dt > bench.timerWorstMs) bench.timerWorstMs = dt;
    // ~1 ms of work, so this cannot be dismissed as a no-op.
    var until = now + 1;
    while (performance.now() < until) { /* spin */ }
  }, 100);

  var last = 0;

  function tick(now) {
    if (!bench.running) return;
    if (!bench.started) {
      // The first callback establishes the clock; it is not a real frame.
      bench.started = true;
      bench.startedAt = now;
      last = now;
      window.requestAnimationFrame(tick);

      return;
    }

    var dt = now - last;
    last = now;
    bench.frames++;
    bench.durationMs = now - bench.startedAt;

    if (dt > bench.worstFrameMs) {
      bench.worstFrameMs = dt;
      bench.worstFrameAtMs = Math.round(now - bench.startedAt);
    }
    // Ignore the first couple of frames after start: the clock is settling.
    if (bench.frames > 3) {
      if (dt > 32) bench.over32++;
      if (dt > 50) {
        bench.over50++;
        if (bench.stalls.length < 50) {
          bench.stalls.push({
            atMs: Math.round(now - bench.startedAt),
            ms: Math.round(dt),
          });
        }
      }
    }

    window.requestAnimationFrame(tick);

  }

  window.requestAnimationFrame(tick);


  /** Snapshot for the host to collect. */
  window.__peakdBenchReport = function () {
    var b = window.__peakdBench;
    var seconds = b.durationMs / 1000;
    return {
      durationMs: Math.round(b.durationMs),
      frames: b.frames,
      fps: seconds > 0 ? +(b.frames / seconds).toFixed(1) : 0,
      worstFrameMs: +b.worstFrameMs.toFixed(1),
      worstFrameAtMs: b.worstFrameAtMs,
      framesOver32ms: b.over32,
      framesOver50ms: b.over50,
      stalls: b.stalls,
      resizes: b.resizes,
      longTasks: b.longTasks,
      visibilityChanges: b.visibilityChanges,
      hiddenAtStart: b.hiddenAtStart,
      timerTicks: b.timerTicks,
      timerWorstMs: +b.timerWorstMs.toFixed(1),
      // Context so a result is comparable across machines.
      dpr: window.devicePixelRatio,
      viewport: { w: window.innerWidth, h: window.innerHeight },
      canvases: document.querySelectorAll("canvas").length,
      tiles: document.querySelectorAll("#tile-grid .tile").length,
      nodes: document.querySelectorAll("*").length,
    };
  };
})();
