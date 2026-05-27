import { getVoiceLang, setVoiceLang } from './api.js';
import { requireAuth } from './auth.js';
import { initMap, getCurrentPosition, loadActiveRoute, refreshGpsPosition } from './map.js';
import {
  initSphere, setSphereState, onShortTap, onLongPressStart, onLongPressEnd,
  onDoubleTap, isConversationMode, setConversationMode, setMicLevel, resetMicLevel,
} from './sphere.js';
import { prepareVoice, startListening, cancelListening, isListening } from './voice.js';
import { sendToAgent, sendToAgentCompose } from './agent.js';
import { startGpsTracking, refreshActiveTrip } from './gps.js';
import { initTheme } from './theme.js';
import { initSettings } from './settings.js';
import { initTrips } from './trips.js';
import { loadArtifacts } from './artifactStore.js';
import { initArtifactDock } from './artifacts.js';
import { initInsightCards } from './insights/insightCards.js';
import { initHudLeft } from './hudLeft.js';
import { initTextInput, openTextInput, isTextInputOpen } from './textInput.js';

function showToast(message, type = 'info') {
  const container = document.getElementById('toast-container');
  if (!container) return;
  const toast = document.createElement('div');
  toast.className = `toast${type === 'error' ? ' error' : ''}`;
  toast.textContent = message;
  container.appendChild(toast);
  setTimeout(() => {
    toast.style.opacity = '0';
    setTimeout(() => toast.remove(), 300);
  }, 4000);
}

function cancelVoiceInput() {
  cancelListening();
  setConversationMode(false);
  resetMicLevel();
  setSphereState('idle');
}

async function boot() {
  if (!localStorage.getItem('voice.lang')) {
    setVoiceLang((navigator.language || 'en-US').split('-')[0]);
  }

  window.addEventListener('app:toast', (e) => {
    showToast(e.detail.message, e.detail.type);
  });

  if (!(await requireAuth())) {
    window.addEventListener('auth:success', boot, { once: true });
    return;
  }

  initTheme();
  document.getElementById('app').classList.remove('hidden');
  initMap();
  await refreshGpsPosition();
  initSphere();
  initArtifactDock();
  initHudLeft();
  initInsightCards();
  initSettings();
  initTrips();
  initTextInput(submitTextToAgent);
  startGpsTracking();
  wireSphere();

  const trip = await refreshActiveTrip();
  if (trip) await loadActiveRoute(trip.id);
  setInterval(async () => {
    const t = await refreshActiveTrip();
    if (t) await loadActiveRoute(t.id);
  }, 60000);

  await loadArtifacts();

  await prepareVoice();
  wireVoiceResults();
}

function wireSphere() {
  onShortTap(async () => {
    if (!voiceReady() || isTextInputOpen()) return;

    if (isListening()) {
      cancelVoiceInput();
      return;
    }

    try {
      await startListening('single');
    } catch (e) {
      setSphereState('error');
      showToast(e.message || 'Microphone unavailable', 'error');
      setTimeout(() => setSphereState('idle'), 2000);
    }
  });

  onLongPressStart(async () => {
    if (!voiceReady() || isListening()) return;
    try {
      await startListening('continuous');
    } catch (e) {
      setConversationMode(false);
      setSphereState('error');
      showToast(e.message || 'Microphone unavailable', 'error');
    }
  });

  onLongPressEnd(() => {
    cancelVoiceInput();
  });

  onDoubleTap(() => {
    if (!voiceReady()) return;
    if (isListening()) cancelVoiceInput();
    openTextInput();
    setSphereState('idle');
  });
}

async function submitTextToAgent(text, handlers) {
  const ctx = getCurrentPosition();
  try {
    await sendToAgentCompose(text, ctx, handlers);
  } catch (_) {}
}

function voiceReady() {
  return document.getElementById('sphere-container') &&
    !document.getElementById('sphere-container').classList.contains('disabled');
}

function wireVoiceResults() {
  window.addEventListener('voice:result', async (e) => {
    const { text, mode } = e.detail;
    const ctx = getCurrentPosition();
    const agentMode = mode === 'continuous' ? 'continuous' : 'single';

    try {
      await sendToAgent(text, agentMode, ctx);
    } catch (_) {}

    if (isConversationMode() && mode === 'continuous') {
      setTimeout(async () => {
        if (isConversationMode()) {
          try {
            await startListening('continuous');
          } catch (_) {
            setSphereState('idle');
          }
        }
      }, 500);
    }
  });

  window.addEventListener('voice:level', (e) => {
    setMicLevel(e.detail);
  });
}

boot();
