/**
 * hudStream.js — the live feed + poll fallback shared by the top-bar chips.
 *
 * Each chip listens to its topic on the shell's single event stream
 * (`eventBus.js`, `GET /api/events`) and falls back to a slow `<area>/status`
 * poll whenever that stream is down (the kiosk proxy may drop an idle stream;
 * the bus reconnects by itself). The module owns only `apply(status)`.
 *
 * The chips used to hold one connection each, which with Craft windows open
 * filled Chromium's six-per-origin pool and stalled every other request at
 * login; on the shared stream they cost nothing extra, so they stay live even
 * while the page is hidden or an agent turn is running.
 */

import { apiFetch } from './api.js';
import { subscribe } from './eventBus.js';

export function createHudStream({ topic, statusUrl, pollMs, label, apply }) {
  let unsubscribe = null;
  let pollTimer = null;

  function startPolling() {
    if (pollTimer) return;
    void poll();
    pollTimer = setInterval(() => void poll(), pollMs);
  }

  function stopPolling() {
    if (!pollTimer) return;
    clearInterval(pollTimer);
    pollTimer = null;
  }

  async function poll() {
    try {
      const res = await apiFetch(statusUrl, { authRedirect: false });
      apply(res?.data);
    } catch (_) { /* server restarting — keep the last state */ }
  }

  function onEvent(data) {
    try {
      apply(JSON.parse(data));
    } catch (error) {
      console.warn(`${label} event: bad payload`, error);
    }
  }

  function onState(up) {
    if (up) stopPolling();
    else startPolling();
  }

  function start() {
    if (unsubscribe) return;
    unsubscribe = subscribe(topic, onEvent, onState);
  }

  function stop() {
    unsubscribe?.();
    unsubscribe = null;
    stopPolling();
  }

  return { start, stop };
}
