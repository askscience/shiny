import { apiFetch, getToken, ApiError } from './api.js';
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

function buildAgentBody(message, mode, context) {
  const lang = localStorage.getItem('voice.lang') ||
    (navigator.language || 'en').split('-')[0];
  const body = { message, mode, lang, context, ai_name: getAiName(), stream: true };
  const model = getOllamaModel();
  if (model) body.ollama_model = model;
  return body;
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
  window.dispatchEvent(new CustomEvent('agent:actions', { detail: res?.actions_taken || [] }));
  // The AI turned a plugin on/off — re-evaluate plugin windows, keyboard,
  // HUD chrome and traveler surfaces immediately.
  const touchedPlugins = (res?.actions_taken || []).some(
    (a) => a?.action === 'plugin_activate' || a?.action === 'plugin_deactivate',
  );
  if (touchedPlugins) {
    window.dispatchEvent(new CustomEvent('plugins:changed'));
  }
}

export async function sendToAgent(message, mode, context) {
  setAgentAwaiting(true);
  setSphereState('processing');
  handleAgentStep('Thinking…');

  try {
    const res = await requestAgent(
      buildAgentBody(message, mode, context),
      handleAgentStep,
    );

    await ingestAgentArtifacts(res.artifacts);

    await handleNavigation(res, message, context);

    handleFocusPlugin(res);
    dispatchAgentActions(res);

    await syncTripsAfterAgent(res);

    setSphereState('speaking');

    // The reply is the primary surface — show it even if voice playback fails.
    const replyEl = document.getElementById('reply-text');
    if (replyEl) {
      replyEl.textContent = res.reply;
      replyEl.classList.remove('hidden');
      setTimeout(() => replyEl.classList.add('hidden'), 8000);
    }

    try {
      await speak(res.reply, localStorage.getItem('voice.lang') ||
        (navigator.language || 'en').split('-')[0]);
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
    if (replyEl) {
      replyEl.textContent = msg;
      replyEl.classList.remove('hidden');
    }
    setTimeout(() => {
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
      buildAgentBody(message, 'single', context),
      handleAgentStep,
    );

    await streamText(res.reply || '', onStream, 12);

    await ingestAgentArtifacts(res.artifacts);

    await handleNavigation(res, message, context);

    handleFocusPlugin(res);
    dispatchAgentActions(res);

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

export function clearAgentUI() {
  clearArtifacts();
}
