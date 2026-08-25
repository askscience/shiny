import { getVoiceLang, setVoiceLang, getTraveler } from './api.js';
import { requireAuth } from './auth.js';
import { initMap, getCurrentPosition } from './map.js';
import {
  initSphere, setSphereState, onShortTap, onLongPressStart, onLongPressEnd,
  onDoubleTap, isConversationMode, setConversationMode, setMicLevel, resetMicLevel,
  getSphereState,
} from './sphere.js';
import { prepareVoice, startListening, cancelListening, isListening, releaseWakeHold, isWakeAwaitingCommand } from './voice.js';
import { sendToAgent, sendToAgentCompose } from './agent.js';
import { startGpsTracking } from './gps.js';
import {
  initThemeLoader, initAppearance, refreshAppearance,
  wireToastEvents, toast, hydrateIcons, reveal,
} from '../ui/index.js';
import { refreshActivePlugins } from './activePlugins.js';
import { initArtifactDock } from './artifacts.js';
import { initInsightCards } from './insights/insightCards.js';
import { initHudClock, initHudTrips } from './hudLeft.js';
import { initNavigator } from './navigator.js';
import { initTileManager, refreshTiles } from './tiles.js';
import { initTextInput, openTextInput, isTextInputOpen, isComposeAwaiting } from './textInput.js';
import { reloadUserSession } from './session.js';

let appInitialized = false;
// null = not yet evaluated — the first check must always apply show/hide,
// otherwise a fresh load in bare mode leaves the map stack visible.
let travelerActive = null;

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

  // Theme + appearance first: everything renders through these tokens.
  await initThemeLoader();
  initAppearance({ getScope: () => getTraveler()?.id });
  wireToastEvents();
  hydrateIcons();

  window.addEventListener('auth:success', async () => {
    document.getElementById('app')?.classList.remove('hidden');
    if (appInitialized) {
      await reloadUserSession();
      refreshAppearance();
      await applyTravelerActivation();
      return;
    }
    await initApp();
  });

  // Plugins panel may toggle the traveler plugin — re-evaluate and re-render
  // the map / navigator / saved-trips HUD when that happens. The plugins page
  // is separate, so the signal travels via localStorage.
  window.addEventListener('plugins:changed', async () => {
    await applyTravelerActivation();
  });
  window.addEventListener('storage', async (e) => {
    if (e.key === 'plugins.changed') await applyTravelerActivation();
  });

  if (!(await requireAuth())) return;

  document.getElementById('app').classList.remove('hidden');
  await initApp();
}

async function initApp() {
  if (appInitialized) {
    await reloadUserSession();
    refreshAppearance();
    return;
  }
  appInitialized = true;

  initSphere();
  initArtifactDock();
  initTextInput(submitTextToAgent);
  initHudClock(); // core chrome — works with zero plugins
  initTileManager(); // plugin window shell — mounts tiles for any active plugin

  await applyTravelerActivation();

  setInterval(() => reloadUserSession(), 60000);
  await reloadUserSession();
  refreshAppearance();

  prepareVoice();
  wireSphere();
  wireVoiceResults();
  reveal();
}

/// Decide whether the OSM map / navigator / saved-trips HUD should render
/// based on whether the current user has the `traveler` plugin activated.
async function applyTravelerActivation() {
  const active = await refreshActivePlugins();
  const newTravelerActive = active.has('traveler');
  if (newTravelerActive === travelerActive) return;
  travelerActive = newTravelerActive;

  // The tiling window manager owns plugin windows — refresh BEFORE initMap()
  // so the map tile exists in the DOM when the traveler window appears.
  initTileManager();
  refreshTiles();

  const mapVignette = document.getElementById('map-vignette');
  const navPuck = document.getElementById('nav-puck');
  const navBanner = document.getElementById('nav-banner');
  const hudSavedTrips = document.getElementById('hud-saved-trips');
  const travelPanel = document.getElementById('travel-panel');
  const travelPanelBackdrop = document.getElementById('travel-panel-backdrop');
  const insightCards = document.getElementById('insight-cards');

  const hide = (el) => el?.classList.add('hidden');
  const show = (el) => el?.classList.remove('hidden');

  if (travelerActive) {
    if (!document.getElementById('map')?.childElementCount) {
      initMap();
    }
    show(mapVignette);
    initHudTrips();
    initNavigator();
    initInsightCards();
    startGpsTracking();
  } else {
    hide(mapVignette);
    hide(navPuck);
    hide(navBanner);
    hide(hudSavedTrips);
    hudSavedTrips?.classList.add('empty');
    hide(travelPanel);
    hide(travelPanelBackdrop);
    hide(insightCards);
    // Chat-only mode: no artifact chrome.
    hide(document.getElementById('artifact-dock'));
  }
}

function voiceReady() {
  return document.getElementById('sphere-container') &&
    !document.getElementById('sphere-container').classList.contains('disabled');
}

/** Voice gestures while the speech model prepares: feedback, not silence. */
function voiceNotReady() {
  const preparing = getSphereState() === 'downloading';
  toast(
    preparing
      ? 'Voice is preparing — try again in a moment'
      : 'Voice unavailable — you can still double-tap to type',
    { type: 'info' },
  );
}

function wireSphere() {
  onShortTap(async () => {
    if (isTextInputOpen() || isComposeAwaiting()) return;
    if (!voiceReady()) {
      voiceNotReady();
      return;
    }

    if (isListening()) {
      cancelVoiceInput();
      return;
    }

    try {
      await startListening('single');
    } catch (e) {
      setSphereState('error');
      toast(e.message || 'Microphone unavailable', { type: 'error' });
      setTimeout(() => setSphereState('idle'), 2000);
    }
  });

  onLongPressStart(async () => {
    if (isListening() || isComposeAwaiting()) return;
    if (!voiceReady()) {
      voiceNotReady();
      return;
    }
    try {
      await startListening('wake');
    } catch (e) {
      setConversationMode(false);
      setSphereState('error');
      toast(e.message || 'Microphone unavailable', { type: 'error' });
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

  // Double-tap = type to the assistant. Text needs no speech model, so this
  // works even while voice is still preparing (or unavailable).
  onDoubleTap(() => {
    if (isComposeAwaiting()) return;
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
