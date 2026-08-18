import {
  getArtifact,
  getDockSummaries,
  getCachedArtifact,
  normalizeArtifact,
  removeSummary,
  cacheArtifactLocal,
  destinationKeyForArtifact,
  setActiveDestination,
} from './artifactStore.js';
import { navigateToDestination, previewDestination } from './map.js';
import { loadContextInsights } from './insights/insightCards.js';
import { isPluginActive } from './activePlugins.js';
import { artifactPanel, dockButton, iconForArtifact, labelForArtifact, reveal } from '../ui/index.js';

const panel = document.getElementById('travel-panel');
const backdrop = document.getElementById('travel-panel-backdrop');
const dock = document.getElementById('artifact-dock');
const dockIcons = document.getElementById('artifact-dock-icons');
let currentArtifact = null;
let activeArtifactId = null;

/** One dock icon per topic slot (overview + 3 themes). */
const MAX_VISIBLE = 4;

function openPanel() {
  if (!panel) return;
  document.getElementById('app')?.classList.add('panel-open');
  panel.classList.remove('hidden');
  backdrop?.classList.remove('hidden');
  requestAnimationFrame(() => {
    panel.classList.add('visible');
    backdrop?.classList.add('visible');
    window.dispatchEvent(new Event('map:resize'));
  });
}

function closePanel() {
  panel?.classList.remove('visible');
  backdrop?.classList.remove('visible');
  document.getElementById('app')?.classList.remove('panel-open');
  setTimeout(() => {
    panel?.classList.add('hidden');
    backdrop?.classList.add('hidden');
    if (panel) panel.innerHTML = '';
    window.dispatchEvent(new Event('map:resize'));
  }, 400);
}

function applyMapForArtifact(artifact) {
  previewDestination(artifact);
}

export function renderArtifact(artifact, { focus = true } = {}) {
  if (!panel) return;

  const normalized = normalizeArtifact(artifact);
  currentArtifact = normalized;
  activeArtifactId = normalized.id;
  cacheArtifactLocal(normalized);
  panel.innerHTML = '';

  const content = artifactPanel(normalized, {
    onClose: clearArtifacts,
    onNavigate: handleNavigate,
    onAction: handleAction,
  });
  panel.appendChild(content);
  reveal(panel);

  if (focus) {
    openPanel();
    applyMapForArtifact(normalized);
  }

  renderArtifactDock(getDockSummaries());
}

async function handleNavigate(artifact) {
  const result = await navigateToDestination(artifact);
  if (result?.ok) {
    closePanel();
    const msg = result.mode === 'direct'
      ? 'Straight line from your location — full driving route could not be loaded'
      : 'Driving route from your location — pinch or drag the map to explore';
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: msg, type: 'info' },
    }));
  } else if (artifact.coordinates || artifact.actions?.some((a) => a.tool === 'map_route')) {
    const msg = artifact._routeError || 'Could not load driving route — try again in a moment';
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: msg, type: 'error' },
    }));
  } else {
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: 'No destination coordinates for this plan', type: 'error' },
    }));
  }
}

async function handleAction(action, artifact) {
  if (action.tool === 'map_route') {
    await handleNavigate(artifact);
  }
}

export function clearArtifacts() {
  closePanel();
  currentArtifact = null;
  activeArtifactId = null;
  renderArtifactDock(getDockSummaries());
  window.dispatchEvent(new CustomEvent('artifact:clear'));
}

export async function openSavedArtifact(id) {
  if (currentArtifact?.id === id) {
    renderArtifact(currentArtifact);
    return;
  }

  const cached = getCachedArtifact(id);
  if (cached) {
    renderArtifact(cached);
    return;
  }

  try {
    const artifact = await getArtifact(id);
    const destKey = destinationKeyForArtifact(artifact);
    if (destKey) setActiveDestination(destKey);
    if (artifact.coordinates?.lat != null && artifact.coordinates?.lon != null) {
      const dest = artifact.destination || artifact.title;
      void loadContextInsights(dest, artifact.coordinates.lat, artifact.coordinates.lon);
    }
    renderArtifact(artifact);
  } catch (e) {
    if (e.status === 404) {
      removeSummary(id);
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: 'That guide is no longer saved — plan the trip again', type: 'error' },
      }));
    } else {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: e.message || 'Could not load saved card', type: 'error' },
      }));
    }
  }
}

export function renderArtifactDock(artifacts) {
  if (!dock || !dockIcons) return;
  dockIcons.innerHTML = '';

  const list = artifacts || [];
  const composeOpen = document.body.classList.contains('compose-active');

  // Chat-only mode (traveler deactivated): the dock stays hidden — the
  // compose input re-shows it via the compose-active CSS when needed.
  if (!isPluginActive('traveler')) {
    if (!composeOpen) dock.classList.add('hidden');
    return;
  }

  if (!list.length && !composeOpen) {
    dock.classList.add('hidden');
    return;
  }

  if (!composeOpen) {
    dock.classList.remove('hidden');
  }

  if (!list.length) return;
  const visible = list.slice(0, MAX_VISIBLE);
  const overflow = list.length - visible.length;

  visible.forEach((item) => {
    dockIcons.appendChild(dockButton({
      icon: iconForArtifact(item),
      label: labelForArtifact(item),
      active: item.id === activeArtifactId,
      onClick: () => openSavedArtifact(item.id),
    }));
  });

  if (overflow > 0) {
    dockIcons.appendChild(dockButton({
      text: `+${overflow}`,
      label: `${overflow} more saved cards`,
      onClick: () => openSavedArtifact(list[MAX_VISIBLE].id),
    }));
  }
}

export function initArtifactDock() {
  backdrop?.addEventListener('click', clearArtifacts);

  window.addEventListener('artifact:dock', (e) => {
    renderArtifactDock(e.detail);
  });
  window.addEventListener('artifact:saved', () => {
    renderArtifactDock(getDockSummaries());
  });
  window.addEventListener('artifact:updated', (e) => {
    renderArtifactDock(getDockSummaries());
    if (e.detail && e.detail.id === activeArtifactId) {
      renderArtifact(e.detail);
    }
  });
}

export function getCurrentArtifact() {
  return currentArtifact;
}
