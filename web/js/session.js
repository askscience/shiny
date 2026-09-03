import { validateSession } from './api.js';
import { clearArtifacts } from './artifacts.js';
import { resetArtifactStore } from './artifactStore.js';
import { clearInsightCards } from './insights/insightStore.js';
import { resetActiveTrip, refreshActiveTrip } from './gps.js';
import { clearNavigation, loadActiveRoute, refreshGpsPosition } from './map.js';
import { stopNavigator } from './navigator.js';
import { loadArtifacts } from './artifactStore.js';
import { refreshAppearance } from '../ui/index.js';
import { closeTextInput } from './textInput.js';
import { setSphereState } from './sphere.js';
import { getRemember } from './preferences.js';
import { refreshBackground } from './background.js';

export function resetUserSession() {
  closeTextInput(true);
  stopNavigator();
  clearArtifacts();
  clearNavigation();
  resetArtifactStore();
  clearInsightCards();
  resetActiveTrip();
  setSphereState('idle');
  document.getElementById('travel-panel')?.classList.add('hidden');
  document.getElementById('travel-panel-backdrop')?.classList.add('hidden');
  document.getElementById('app')?.classList.remove('panel-open');
}

export async function reloadUserSession() {
  await validateSession();
  refreshAppearance();
  refreshBackground();
  await refreshGpsPosition();
  const trip = await refreshActiveTrip();
  if (trip?.id) {
    await loadActiveRoute(trip.id);
  } else {
    clearNavigation();
  }
  // Fresh mode: don't restore saved cards — the dock starts empty. The cards
  // stay saved server-side and come back when "Remember workspace" is on.
  if (getRemember()) {
    await loadArtifacts();
  }
}
