/**
 * Drives a synthetic drag of the first floating plugin window, so the benchmark
 * can measure what dragging a *plugin window* costs.
 *
 * This is the case that matters. An earlier investigation measured the native
 * app window (moving the whole OS window), which is a different thing entirely:
 * plugin windows are DOM elements inside the desktop, so dragging one runs the
 * app's own pointermove handler, style writes and layout on the main thread.
 *
 * A synthetic pointer sequence is reliable here for a reason it was not for the
 * native window: these are ordinary DOM events delivered straight to the
 * element, with no window server, no occlusion heuristics and no title-bar
 * hit-testing to satisfy.
 *
 * Each `pointermove` deliberately does nothing but dispatch. The cost being
 * measured is the *app's* handler, not the harness's.
 */
(function () {
  "use strict";

  if (window.__peakdTileDrag) return 'already';

  var tile = document.querySelector('#tile-grid .tile');
  if (!tile) return 'no-tile';
  var header = tile.querySelector('.tile-header') || tile;

  var state = { moves: 0, done: false, startedAt: 0 };
  window.__peakdTileDrag = state;

  function pointer(type, x, y) {
    var ev;
    try {
      ev = new PointerEvent(type, {
        bubbles: true, cancelable: true, composed: true,
        pointerId: 1, pointerType: 'mouse', isPrimary: true,
        buttons: type === 'pointerup' ? 0 : 1,
        clientX: x, clientY: y,
      });
    } catch (e) {
      ev = new MouseEvent(type.replace('pointer', 'mouse'), {
        bubbles: true, cancelable: true, clientX: x, clientY: y,
      });
    }
    header.dispatchEvent(ev);
  }

  var rect = header.getBoundingClientRect();
  var baseX = rect.left + rect.width / 2;
  var baseY = rect.top + rect.height / 2;

  pointer('pointerdown', baseX, baseY);
  state.startedAt = performance.now();

  // Drive well above any display rate. If cost scales with event count rather
  // than with frames, that is the finding: pointermove can fire several times
  // per presented frame.
  var i = 0;
  var timer = setInterval(function () {
    if (state.done) return;
    // No layout reads in here: `getBoundingClientRect` per iteration would make
    // the harness itself the bottleneck and hide what the drag actually costs.
    // Coordinates are computed from constants captured before the drag.
    for (var k = 0; k < 4; k++) {
      i++;
      var x = baseX + 120 * Math.sin(i * 0.08);
      var y = baseY + 60 * Math.cos(i * 0.08);
      pointer('pointermove', x, y);
      state.moves++;
    }
  }, 1000 / 60);

  // Stop on our own, so a caller that forgets cannot leave the tile dragging.
  setTimeout(function () {
    if (state.done) return;
    state.done = true;
    clearInterval(timer);
    pointer('pointerup', baseX, baseY);
  }, 60000);

  window.__peakdTileDragStop = function () {
    if (state.done) return state.moves;
    state.done = true;
    clearInterval(timer);
    pointer('pointerup', baseX, baseY);
    return state.moves;
  };

  return 'started';
})()
