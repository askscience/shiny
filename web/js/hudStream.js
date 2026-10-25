/**
 * hudStream.js — the SSE stream + poll fallback shared by the top-bar chips.
 *
 * Each chip stays live from `<area>/events` and falls back to a slow
 * `<area>/status` poll whenever the stream is down (the kiosk proxy may drop
 * an idle stream). The stream reconnects forever; the poll keeps the chip
 * honest during the wait. The module owns only `apply(status)`.
 *
 * The stream is also paused while the document is hidden: four permanent HUD
 * connections would otherwise hold most of the browser's HTTP/1.1 connection
 * budget even when nothing is on screen.
 */

import { apiFetch, getToken } from './api.js';

const RECONNECT_MS = 5000;

export function createHudStream({ eventsUrl, statusUrl, pollMs, label, apply }) {
  let abort = null;
  let pollTimer = null;
  let started = false;
  let stopped = false;
  let paused = false;
  let generation = 0;

  const canObserveVisibility = typeof document !== 'undefined'
    && typeof document.addEventListener === 'function';
  // Four permanent event streams hold most of Chromium's six HTTP/1.1
  // connections per origin. During an agent turn the chat's SSE and the
  // approval POST need one too — with the terminal window open the pool is
  // full and every other request (a window, a HUD menu) sits in "loading"
  // forever. The chips keep their last state and reconnect when the turn ends.
  const canObserveBusy = canObserveVisibility
    && typeof MutationObserver === 'function'
    && document.body;
  let bodyObserver = null;

  function isHidden() {
    return canObserveVisibility && document.hidden;
  }

  function isBusy() {
    const body = canObserveBusy ? document.body : null;
    return !!body?.classList?.contains('agent-awaiting');
  }

  function shouldPause() {
    return isHidden() || isBusy();
  }

  function headers() {
    const token = getToken();
    return token ? { Authorization: `Bearer ${token}` } : {};
  }

  function sleep(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  async function readStream(res) {
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    for (;;) {
      const { done, value } = await reader.read();
      if (done) return;
      buffer += decoder.decode(value, { stream: true });

      const chunks = buffer.split('\n\n');
      buffer = chunks.pop() || '';

      for (const chunk of chunks) {
        const dataLine = chunk.split('\n').find((line) => line.startsWith('data:'));
        if (!dataLine) continue;
        try {
          apply(JSON.parse(dataLine.replace(/^data:\s*/, '')));
        } catch (_) { /* keep-alive comment or partial frame */ }
      }
    }
  }

  function startPolling() {
    if (pollTimer || shouldPause()) return;
    void poll();
    pollTimer = setInterval(() => void poll(), pollMs);
  }

  function stopPolling() {
    if (!pollTimer) return;
    clearInterval(pollTimer);
    pollTimer = null;
  }

  async function poll() {
    if (shouldPause()) return;
    try {
      const res = await apiFetch(statusUrl, { authRedirect: false });
      apply(res?.data);
    } catch (_) { /* server restarting — keep the last state */ }
  }

  function stopStream() {
    if (abort) {
      abort.abort();
      abort = null;
    }
    stopPolling();
  }

  async function subscribe() {
    // A resume starts a new generation; the old loop may still be sleeping in
    // the reconnect delay and must not start a second concurrent stream.
    const generationAtStart = ++generation;
    while (!stopped && !paused && generationAtStart === generation) {
      const controller = new AbortController();
      abort = controller;
      try {
        const res = await fetch(eventsUrl, {
          headers: headers(),
          credentials: 'same-origin',
          signal: controller.signal,
        });
        if (!res.ok || !res.body) throw new Error(`${label} events: ${res.status}`);
        stopPolling();
        await readStream(res);
      } catch (_) {
        if (controller.signal.aborted) return;
      }
      if (stopped || paused || generationAtStart !== generation) return;
      // Stream ended or failed: keep the chip honest with the poll while we
      // wait to reconnect (the kiosk proxy may drop an idle stream).
      startPolling();
      await sleep(RECONNECT_MS);
    }
  }

  function syncPause() {
    if (!started || stopped) return;
    if (shouldPause()) {
      if (paused) return;
      paused = true;
      stopStream();
      return;
    }
    if (!paused) return;
    paused = false;
    void subscribe();
  }

  function start() {
    if (started) return;
    started = true;
    if (canObserveVisibility) document.addEventListener('visibilitychange', syncPause);
    if (canObserveBusy) {
      bodyObserver = new MutationObserver(syncPause);
      bodyObserver.observe(document.body, { attributes: true, attributeFilter: ['class'] });
    }
    if (shouldPause()) {
      paused = true;
      return;
    }
    void subscribe();
  }

  function stop() {
    if (stopped) return;
    stopped = true;
    paused = true;
    generation += 1;
    bodyObserver?.disconnect();
    bodyObserver = null;
    if (canObserveVisibility) document.removeEventListener('visibilitychange', syncPause);
    stopStream();
  }

  return { start, stop };
}
