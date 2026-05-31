import { getVoiceLang, setVoiceLang } from './api.js';
import { requireAuth } from './auth.js';
import { initMap, getCurrentPosition } from './map.js';
import {
  initSphere, setSphereState, onShortTap, onLongPressStart, onLongPressEnd,
  onDoubleTap, isConversationMode, setConversationMode, setMicLevel, resetMicLevel,
} from './sphere.js';
import { prepareVoice, startListening, cancelListening, isListening, releaseWakeHold, isWakeAwaitingCommand } from './voice.js';
import { sendToAgent, sendToAgentCompose } from './agent.js';
import { startGpsTracking } from './gps.js';
import { initTheme } from './theme.js';
import { initAccent } from './accent.js';
import { initSettings } from './settings.js';
import { initArtifactDock } from './artifacts.js';
import { initInsightCards } from './insights/insightCards.js';
import { initHudLeft } from './hudLeft.js';
import { initNavigator } from './navigator.js';
import { initTextInput, openTextInput, isTextInputOpen, isComposeAwaiting } from './textInput.js';
import { reloadUserSession } from './session.js';

let appInitialized = false;

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
  if (isWakeAwaitingCommand()) return;
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

  window.addEventListener('auth:success', async () => {
    document.getElementById('app')?.classList.remove('hidden');
    if (appInitialized) {
      await reloadUserSession();
      return;
    }
    await initApp();
  });

  if (!(await requireAuth())) return;

  document.getElementById('app').classList.remove('hidden');
  await initApp();
}

async function initApp() {
  if (appInitialized) {
    await reloadUserSession();
    return;
  }
  appInitialized = true;

  initTheme();
  initAccent();
  initMap();
  initSphere();
  initArtifactDock();
  initHudLeft();
  initNavigator();
  initInsightCards();
  initSettings();
  initTextInput(submitTextToAgent);
  startGpsTracking();
  wireSphere();

  setInterval(() => reloadUserSession(), 60000);

  await reloadUserSession();

  prepareVoice();
  wireVoiceResults();
}

function wireSphere() {
  onShortTap(async () => {
    if (!voiceReady() || isTextInputOpen() || isComposeAwaiting()) return;

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
    if (!voiceReady() || isListening() || isComposeAwaiting()) return;
    try {
      await startListening('wake');
    } catch (e) {
      setConversationMode(false);
      setSphereState('error');
      showToast(e.message || 'Microphone unavailable', 'error');
    }
  });

  onLongPressEnd(() => {
    releaseWakeHold();
    if (!isListening()) {
      setConversationMode(false);
      resetMicLevel();
      if (!isTextInputOpen()) setSphereState('idle');
    }
  });

  onDoubleTap(() => {
    if (!voiceReady() || isComposeAwaiting()) return;
    if (isListening() && !isWakeAwaitingCommand()) cancelVoiceInput();
    openTextInput();
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

    setConversationMode(false);
    resetMicLevel();

    try {
      await sendToAgent(text, agentMode, ctx);
    } catch (_) {}

    if (!isTextInputOpen() && !isComposeAwaiting()) setSphereState('idle');
  });

  window.addEventListener('voice:level', (e) => {
    setMicLevel(e.detail);
  });
}

boot();
