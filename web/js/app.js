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
import { refreshActivePlugins } from './activePlugins.js';
import { initArtifactDock } from './artifacts.js';
import { initInsightCards } from './insights/insightCards.js';
import { initHudLeft } from './hudLeft.js';
import { initNavigator } from './navigator.js';
import { initTextInput, openTextInput, isTextInputOpen, isComposeAwaiting } from './textInput.js';
import { reloadUserSession } from './session.js';

let appInitialized = false;
let travelerActive = false;

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
      await applyTravelerActivation();
      return;
    }
    await initApp();
  });

  // Plugins panel may toggle the traveler plugin — re-evaluate and re-render
  // the map / navigator / saved-trips HUD when that happens.
  window.addEventListener('plugins:changed', async () => {
    await applyTravelerActivation();
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
  initSphere();
  initArtifactDock();
  initSettings();
  initTextInput(submitTextToAgent);

  await applyTravelerActivation();

  setInterval(() => reloadUserSession(), 60000);
  await reloadUserSession();

  prepareVoice();
  wireSphere();
  wireVoiceResults();
}

/// Decide whether the OSM map / navigator / saved-trips HUD should render
/// based on whether the current user has the `traveler` plugin activated.
async function applyTravelerActivation() {
  const active = await refreshActivePlugins();
  const newTravelerActive = active.has('traveler');
  if (newTravelerActive === travelerActive) return;
  travelerActive = newTravelerActive;

  const mapStage = document.getElementById('map-stage');
  const mapVignette = document.getElementById('map-vignette');
  const navPuck = document.getElementById('nav-puck');
  const navBanner = document.getElementById('nav-banner');
  const hudSavedTrips = document.getElementById('hud-saved-trips');
  const hudSavedTripsMobile = document.getElementById('hud-saved-trips-mobile');
  const travelPanel = document.getElementById('travel-panel');
  const travelPanelBackdrop = document.getElementById('travel-panel-backdrop');
  const insightCards = document.getElementById('insight-cards');

  const hide = (el) => el?.classList.add('hidden');
  const show = (el) => el?.classList.remove('hidden');

  if (travelerActive) {
    if (!document.getElementById('map')?.childElementCount) {
      initMap();
    }
    show(mapStage);
    show(mapVignette);
    initHudLeft();
    initNavigator();
    initInsightCards();
    startGpsTracking();
  } else {
    hide(mapStage);
    hide(mapVignette);
    hide(navPuck);
    hide(navBanner);
    hide(hudSavedTrips);
    hudSavedTrips?.classList.add('empty');
    hide(hudSavedTripsMobile);
    hudSavedTripsMobile?.classList.add('empty');
    hide(travelPanel);
    hide(travelPanelBackdrop);
    hide(insightCards);
  }
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
  const ctx = travelerActive ? getCurrentPosition() : null;
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
    const ctx = travelerActive ? getCurrentPosition() : null;
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