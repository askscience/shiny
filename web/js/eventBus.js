/**
 * eventBus.js — every live server stream the shell needs, on one connection.
 *
 * Chromium (and so the LiteAura kiosk) opens at most six HTTP/1.1 connections
 * per origin, and an open SSE stream holds one for as long as it lives. With a
 * stream per top-bar chip and one per open Craft window the pool was full at
 * login, and every other request (the power/battery/Wi-Fi menus, the
 * browser's suggestions, an app's WebAssembly) queued for up to a minute.
 *
 * The server multiplexes them on `GET /api/events?topics=a,b,…`; each SSE
 * event is named after its topic (`audio`, `battery`, `bluetooth`, `network`,
 * `craft:<app>`). Modules subscribe to a topic and get each event's `data`
 * string. A changed topic set opens a new connection and drops the old one
 * once the new one answers, so nothing is missed in between.
 *
 * `onState(connected)` tells a subscriber when the stream is up or down, so the
 * HUD chips can fall back to polling while it reconnects.
 */

import { getToken } from './api.js';

const RECONNECT_MS = 5000;
/** Batches the subscriptions made in one tick (boot) into one connection. */
const SETTLE_MS = 30;

/** topic → Set<{ onEvent, onState }> */
const topics = new Map();
let current = null; // { key, controller }
let connected = false;
let reconnectTimer = null;
let settleTimer = null;
let generation = 0;

function eventsUrl(key) {
  // Relative on purpose: the shell can be served under a proxy prefix.
  const url = new URL('../api/events', import.meta.url);
  url.searchParams.set('topics', key);
  return url.href;
}

function topicKey() {
  return [...topics.keys()].sort().join(',');
}

function setConnected(value, force = false) {
  if (connected === value && !force) return;
  connected = value;
  for (const subscribers of topics.values()) {
    for (const subscriber of subscribers) subscriber.onState?.(value);
  }
}

function dispatch(event, data) {
  const subscribers = topics.get(event);
  if (!subscribers) return;
  for (const subscriber of [...subscribers]) {
    try {
      subscriber.onEvent(data);
    } catch (error) {
      console.warn(`event bus: ${event} handler failed`, error);
    }
  }
}

/** Parse SSE frames out of a streaming body; `onFrame(event, data)` per frame. */
export async function readFrames(body, onFrame) {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return;
    buffer += decoder.decode(value, { stream: true }).replace(/\r\n/g, '\n');
    let start = 0;
    let end;
    while ((end = buffer.indexOf('\n\n', start)) !== -1) {
      const frame = buffer.slice(start, end);
      start = end + 2;
      let event = 'message';
      const data = [];
      for (const line of frame.split('\n')) {
        if (line.startsWith('event:')) event = line.slice(6).trim();
        else if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
      }
      if (data.length) onFrame(event, data.join('\n'));
    }
    if (start > 0) buffer = buffer.slice(start);
  }
}

function scheduleSync(delay = SETTLE_MS) {
  clearTimeout(settleTimer);
  settleTimer = setTimeout(() => {
    settleTimer = null;
    sync();
  }, delay);
}

/** Make the open connection match the subscribed topics. */
function sync() {
  const key = topicKey();
  if (current?.key === key) return;
  clearTimeout(reconnectTimer);
  reconnectTimer = null;
  if (!key) {
    current?.controller.abort();
    current = null;
    setConnected(false);
    return;
  }
  void open(key);
}

async function open(key) {
  const mine = ++generation;
  const previous = current;
  const controller = new AbortController();
  current = { key, controller };
  const token = getToken();
  let res;
  try {
    res = await fetch(eventsUrl(key), {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
      credentials: 'same-origin',
      cache: 'no-store',
      signal: controller.signal,
    });
    if (!res.ok || !res.body) throw new Error(`events: ${res.status}`);
  } catch (_) {
    if (controller.signal.aborted || mine !== generation) return;
    previous?.controller.abort();
    lost(mine);
    return;
  }
  // The new stream answered: the old one can go (the server detaches it).
  previous?.controller.abort();
  if (mine !== generation) {
    controller.abort();
    return;
  }
  // Forced: a topic added to a live bus heard `false` until this answered.
  setConnected(true, true);
  try {
    await readFrames(res.body, dispatch);
  } catch (_) { /* dropped */ }
  if (controller.signal.aborted || mine !== generation) return;
  lost(mine);
}

function lost(mine) {
  if (mine !== generation) return;
  current = null;
  setConnected(false);
  clearTimeout(reconnectTimer);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    sync();
  }, RECONNECT_MS);
}

/**
 * Listen to one topic. `onEvent(data)` gets each event's raw `data` string;
 * `onState(connected)` (optional) hears the stream go up and down, and is told
 * the current state right away. Returns the unsubscribe function.
 */
export function subscribe(topic, onEvent, onState) {
  const subscriber = { onEvent, onState };
  let subscribers = topics.get(topic);
  const added = !subscribers;
  if (added) {
    subscribers = new Set();
    topics.set(topic, subscribers);
  }
  subscribers.add(subscriber);
  onState?.(connected && !added);
  if (added) scheduleSync();
  let active = true;
  return () => {
    if (!active) return;
    active = false;
    subscribers.delete(subscriber);
    if (subscribers.size === 0 && topics.get(topic) === subscribers) {
      topics.delete(topic);
      scheduleSync();
    }
  };
}

/** Test hook: the topics the open (or opening) connection carries. */
export function currentTopics() {
  return current?.key ?? '';
}
