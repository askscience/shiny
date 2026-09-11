import { apiFetch, getToken, getVoiceLang, getTraveler, ApiError } from './api.js';
import { renderArtifact, clearArtifacts } from './artifacts.js';
import {
  upsertArtifact,
  applyDockDestinationGroup,
  destinationKeyForArtifact,
} from './artifactStore.js';
import { loadContextInsights } from './insights/insightCards.js';
import { speak } from './voice.js';
import { setSphereState } from './sphere.js';
import { refreshActiveTrip } from './gps.js';
import { loadActiveRoute } from './map.js';
import { startNavigator, isNavigatorActive } from './navigator.js';
import { getAiName, getOllamaModel } from './preferences.js';
import { getDesktopSnapshot } from './desktop.js';
import { pluginIconEl } from './pluginIcon.js';
import { setDockStep, clearDockStep } from './dockStep.js';
import {
  fetchNavigationSession,
  looksLikeNavigationRequest,
  extractDestinationFromMessage,
  agentFailedNavigation,
} from './navigationApi.js';

const TRIP_ACTIONS = new Set(['create_trip', 'start_trip', 'end_trip']);

async function syncTripsAfterAgent(res) {
  if (!res?.actions_taken?.some((a) => TRIP_ACTIONS.has(a.action))) return;

  const trip = await refreshActiveTrip();
  if (trip?.id) await loadActiveRoute(trip.id);
  window.dispatchEvent(new CustomEvent('trips:changed'));

  const tripAction = res.actions_taken.find((a) => TRIP_ACTIONS.has(a.action));
  if (tripAction?.result === 'error') {
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: 'Trip action failed', type: 'error' },
    }));
    return;
  }

  if (trip?.name) {
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: `Active trip: ${trip.name}`, type: 'info' },
    }));
  }
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function pickPrimaryArtifact(artifacts) {
  if (!artifacts?.length) return null;
  return (
    artifacts.find((a) => a.theme === 'overview') ||
    artifacts.find((a) => (a.type || a.artifact_type) === 'travel_plan') ||
    artifacts[0]
  );
}

async function ingestAgentArtifacts(artifacts) {
  if (isNavigatorActive()) return;
  if (!artifacts?.length) return;
  const ids = [];
  for (const art of artifacts) {
    const saved = await upsertArtifact(art);
    ids.push(saved.id);
  }
  const primary = pickPrimaryArtifact(artifacts);
  const destKey = destinationKeyForArtifact(primary);
  if (destKey) {
    applyDockDestinationGroup(destKey, ids);
  }
  if (primary) {
    renderArtifact(primary);
    if (primary.coordinates?.lat != null && primary.coordinates?.lon != null) {
      const dest = primary.destination || primary.title;
      void loadContextInsights(dest, primary.coordinates.lat, primary.coordinates.lon);
    }
  }
  if (artifacts.length > 1) {
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: {
        message: `${artifacts.length} guides ready — tap an icon below the orb`,
        type: 'info',
      },
    }));
  }
}

async function streamText(text, onStream, delayMs = 14) {
  if (!text) {
    onStream('');
    return;
  }
  const parts = text.match(/\S+\s*|\s+/g) || [text];
  let acc = '';
  for (const part of parts) {
    acc += part;
    onStream(acc);
    await sleep(delayMs);
  }
}

async function handleNavigation(res, userMessage, context) {
  if (res?.navigation) {
    const started = await startNavigator(res.navigation);
    if (started) {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: `Navigating to ${res.navigation.destination}`, type: 'info' },
      }));
    }
    return started;
  }

  if (!looksLikeNavigationRequest(userMessage) && !agentFailedNavigation(res)) {
    return false;
  }

  const destination = extractDestinationFromMessage(userMessage);
  if (!destination || context?.lat == null || context?.lon == null) {
    return false;
  }

  try {
    const session = await fetchNavigationSession({
      destination,
      from_lat: context.lat,
      from_lon: context.lon,
    });
    const started = await startNavigator(session);
    if (started) {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: `Navigating to ${session.destination}`, type: 'info' },
      }));
    }
    return started;
  } catch (e) {
    console.warn('Navigation fallback failed:', e);
    return false;
  }
}

function buildAgentBody(message, mode, context, voice = false) {
  // Same per-user resolved language as voice (explicit choice or browser
  // default), so the AI replies in the language the user actually hears.
  const lang = getVoiceLang();
  const body = {
    message, mode, lang, context,
    ai_name: getAiName(),
    desktop: getDesktopSnapshot(),
    stream: true,
    // Voice requests (STT in, TTS out) must come back as conversational prose
    // — the model is told to skip markdown and lists so the answer reads well
    // aloud. Typed requests keep normal formatting.
    voice: !!voice,
  };
  const model = getOllamaModel();
  if (model) body.ollama_model = model;
  const conversation = currentConversationId();
  if (conversation) body.conversation_id = conversation;
  return body;
}

/* ── Chat history (resumable conversations) ─────────────────── */

const CONVERSATION_KEY = 'chat.conversation';

/** Conversation is scoped per traveler so switching accounts never leaks a
 *  thread, and each user keeps their own "current" chat. */
function conversationKey() {
  const id = getTraveler()?.id;
  return id ? `${CONVERSATION_KEY}.${id}` : CONVERSATION_KEY;
}

export function currentConversationId() {
  return localStorage.getItem(conversationKey()) || null;
}

export function setCurrentConversation(id) {
  if (id) localStorage.setItem(conversationKey(), id);
  else localStorage.removeItem(conversationKey());
}

/** Start a fresh chat (the next message opens a new conversation). */
export function newChat() {
  setCurrentConversation(null);
}

export async function listConversations() {
  try {
    const res = await apiFetch('/api/chat/conversations');
    return res?.data || [];
  } catch {
    return [];
  }
}

export async function loadConversationMessages(id) {
  try {
    const res = await apiFetch(`/api/chat/conversations/${encodeURIComponent(id)}`);
    return res?.data || [];
  } catch {
    return [];
  }
}

export async function removeConversation(id) {
  try {
    await apiFetch(`/api/chat/conversations/${encodeURIComponent(id)}`, { method: 'DELETE' });
    if (currentConversationId() === id) setCurrentConversation(null);
    return true;
  } catch {
    return false;
  }
}

async function readAgentStream(res, onStep) {
  if (!res.body) {
    throw new ApiError('Agent stream unavailable', 500);
  }

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    const chunks = buffer.split('\n\n');
    buffer = chunks.pop() || '';

    for (const chunk of chunks) {
      const dataLine = chunk.split('\n').find((line) => line.startsWith('data:'));
      if (!dataLine) continue;

      let event;
      try {
        event = JSON.parse(dataLine.replace(/^data:\s*/, ''));
      } catch {
        continue;
      }

      if (event.type === 'step' && event.message) {
        onStep?.(event.message);
      } else if (event.type === 'done') {
        return event.data;
      } else if (event.type === 'error') {
        throw new ApiError(event.message || 'Agent failed', 500);
      }
    }
  }

  throw new ApiError('Agent stream ended unexpectedly', 500);
}

async function requestAgent(body, onStep) {
  const headers = { 'Content-Type': 'application/json' };
  const token = getToken();
  if (token) headers.Authorization = `Bearer ${token}`;

  const res = await fetch('/api/agent', {
    method: 'POST',
    headers,
    body: JSON.stringify(body),
  });

  if (res.status === 401) {
    throw new ApiError('Session expired — please sign in again', 401);
  }

  const contentType = res.headers.get('content-type') || '';
  if (contentType.includes('text/event-stream')) {
    if (!res.ok) {
      const text = await res.text();
      throw new ApiError(text || res.statusText, res.status);
    }
    return readAgentStream(res, onStep);
  }

  const text = await res.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = null;
  }

  if (!res.ok) {
    throw new ApiError(data?.error || text || res.statusText, res.status);
  }
  return data;
}

function handleAgentStep(message) {
  setDockStep(message);
}

/** The AI chose a plugin window to surface (show_plugin tool). */
function handleFocusPlugin(res) {
  if (res?.focus_plugin) {
    window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: res.focus_plugin } }));
  }
}

function setAgentAwaiting(on) {
  document.body.classList.toggle('agent-awaiting', on);
}

/** Plugin windows react to tool outcomes here (e.g. radio stops playback). */
function dispatchAgentActions(res) {
  const actions = res?.actions_taken || [];
  // Park a copy so freshly-loaded plugin surfaces (activated in the same
  // response) can catch up — their wireEvents hook runs after this dispatch.
  window.__lastAgentActions = actions;
  window.dispatchEvent(new CustomEvent('agent:actions', { detail: actions }));
  // The AI turned a plugin on/off — re-evaluate plugin windows, keyboard,
  // HUD chrome and traveler surfaces immediately.
  const touchedPlugins = actions.some(
    (a) => a?.action === 'plugin_activate' || a?.action === 'plugin_deactivate',
  );
  if (touchedPlugins) {
    window.dispatchEvent(new CustomEvent('plugins:changed'));
  }
}

/** Render any `notification` a plugin tool attached to its outcome data. */
function surfaceNotifications(res) {
  const actions = res?.actions_taken || [];
  for (const a of actions) {
    const n = a?.data?.notification;
    if (!n || typeof n !== 'object') continue;
    const detail = { ...n };
    // A plugin notification shows the plugin's own icon (web/icon.svg).
    if (n.plugin) {
      detail.icon = pluginIconEl(n.plugin, { size: 18 });
      if (!detail.app) {
        detail.app = n.plugin.charAt(0).toUpperCase() + n.plugin.slice(1);
      }
    }
    window.dispatchEvent(new CustomEvent('app:notify', { detail }));
  }
}

// Guards the transient reply banner: an early send's hide-timer must not
// blank the reply of a newer send that overtook it.
let replyToken = 0;

export async function sendToAgent(message, mode, context) {
  setAgentAwaiting(true);
  setSphereState('processing');
  handleAgentStep('Thinking…');

  try {
    const res = await requestAgent(
      buildAgentBody(message, mode, context, true),
      handleAgentStep,
    );

    if (res?.conversation_id) setCurrentConversation(res.conversation_id);

    await ingestAgentArtifacts(res.artifacts);

    await handleNavigation(res, message, context);

    handleFocusPlugin(res);
    dispatchAgentActions(res);
    surfaceNotifications(res);

    await syncTripsAfterAgent(res);

    setSphereState('speaking');

    // Voice modes: the answer is spoken and saved to the chat — it is NOT
    // echoed in a bubble. The orb reacting to the playback IS the feedback.
    try {
      await speak(res.reply, getVoiceLang());
    } catch (ttsErr) {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: ttsErr?.message || 'Voice playback unavailable', type: 'error' },
      }));
    }
    setSphereState('idle');

    return res;
  } catch (e) {
    setSphereState('error');
    const msg = e.message || 'Agent unavailable';
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: msg, type: 'error' },
    }));
    const replyEl = document.getElementById('reply-text');
    const token = ++replyToken;
    if (replyEl) {
      replyEl.textContent = msg;
      replyEl.classList.remove('hidden');
    }
    setTimeout(() => {
      // A newer send took over the banner — leave it alone.
      if (token !== replyToken) return;
      setSphereState('idle');
      replyEl?.classList.add('hidden');
    }, 3000);
    throw e;
  } finally {
    clearDockStep();
    setAgentAwaiting(false);
  }
}

/** Text compose mode: streams reply into compose panel */
export async function sendToAgentCompose(message, context, { onStream, onDone, onError }) {
  onStream?.('');
  setAgentAwaiting(true);
  setSphereState('processing');
  handleAgentStep('Thinking…');

  try {
    const res = await requestAgent(
      buildAgentBody(message, 'single', context, false),
      handleAgentStep,
    );

    if (res?.conversation_id) setCurrentConversation(res.conversation_id);

    await streamText(res.reply || '', onStream, 12);

    await ingestAgentArtifacts(res.artifacts);

    await handleNavigation(res, message, context);

    handleFocusPlugin(res);
    dispatchAgentActions(res);
    surfaceNotifications(res);

    await syncTripsAfterAgent(res);

    onDone?.(res);
    return res;
  } catch (e) {
    const msg = e.message || 'Agent unavailable';
    setSphereState('error');
    setTimeout(() => setSphereState('idle'), 2000);
    onError?.(msg);
    return null;
  } finally {
    clearDockStep();
    setAgentAwaiting(false);
  }
}
