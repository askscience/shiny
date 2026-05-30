import { apiFetch } from './api.js';
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
import { getAiName } from './preferences.js';
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
        message: `${artifacts.length} guides ready — tap the icons below the orb`,
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
  return { message, mode, lang, context, ai_name: getAiName() };
}

export async function sendToAgent(message, mode, context) {
  setSphereState('processing');

  try {
    const res = await apiFetch('/api/agent', {
      method: 'POST',
      body: JSON.stringify(buildAgentBody(message, mode, context)),
    });

    await ingestAgentArtifacts(res.artifacts);

    await handleNavigation(res, message, context);

    await syncTripsAfterAgent(res);

    setSphereState('speaking');
    await speak(res.reply, localStorage.getItem('voice.lang') ||
      (navigator.language || 'en').split('-')[0]);
    setSphereState('idle');

    const replyEl = document.getElementById('reply-text');
    if (replyEl) {
      replyEl.textContent = res.reply;
      replyEl.classList.remove('hidden');
      setTimeout(() => replyEl.classList.add('hidden'), 8000);
    }

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
  }
}

/** Text compose mode: streams reply into compose panel */
export async function sendToAgentCompose(message, context, { onStream, onDone, onError }) {
  onStream?.('');
  setSphereState('processing');

  try {
    const res = await apiFetch('/api/agent', {
      method: 'POST',
      body: JSON.stringify(buildAgentBody(message, 'single', context)),
    });

    await streamText(res.reply || '', onStream, 12);

    await ingestAgentArtifacts(res.artifacts);

    await handleNavigation(res, message, context);

    await syncTripsAfterAgent(res);

    onDone?.(res);
    return res;
  } catch (e) {
    const msg = e.message || 'Agent unavailable';
    setSphereState('error');
    setTimeout(() => setSphereState('compose'), 2000);
    onError?.(msg);
    return null;
  }
}

export function clearAgentUI() {
  clearArtifacts();
}
