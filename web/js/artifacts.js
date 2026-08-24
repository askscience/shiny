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
import { previewDestination } from './map.js';
import { loadContextInsights } from './insights/insightCards.js';
import { isPluginActive } from './activePlugins.js';
import { openArtifactInTile } from './tiles.js';
import { dockButton, iconForArtifact, labelForArtifact } from '../ui/index.js';

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
  const normalized = normalizeArtifact(artifact);
  currentArtifact = normalized;
  activeArtifactId = normalized.id;
  cacheArtifactLocal(normalized);

  if (focus) {
    // Radio is the card: its tile hero already shows everything an artifact
    // would (art, title, transport) — skip the sheet, just focus the window.
    if (normalized.plugin === 'radio' || normalized.type === 'radio_station') {
      window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: 'radio' } }));
    } else {
      // Plugin output is contained inside its own window — the tile sheet.
      const plugin = normalized.plugin || 'traveler';
      void openArtifactInTile(plugin, normalized);
    }
    applyMapForArtifact(normalized);
  }

  renderArtifactDock(getDockSummaries());
}

export function clearArtifacts() {
  closePanel();
  currentArtifact = null;
  activeArtifactId = null;
  // Close any artifact sheet open inside a plugin window.
  document.querySelectorAll('.tile-sheet').forEach((el) => el.remove());
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
    if (artifact.type === 'radio_station' || artifact.plugin === 'radio') {
      window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: 'radio' } }));
      return;
    }
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
  const list = artifacts || [];
  const composeOpen = document.body.classList.contains('compose-active');

  // The traveler plugin window hosts its own dock (top-right of the tile);
  // fall back to the chrome-bottom dock when the tile isn't mounted (chat-only
  // mode) or while composing (the compose input pill lives in #artifact-dock).
  const tileDock = document.getElementById('map-tile-dock');
  const tileDockIcons = document.getElementById('map-tile-dock-icons');
  const inTile = !!tileDock && tileDock.isConnected && isPluginActive('traveler') && !composeOpen;
  document.body.classList.toggle('tile-dock-active', inTile);
  if (tileDock) tileDock.classList.toggle('hidden', !inTile || !list.length);

  const container = inTile ? tileDock : dock;
  const iconsEl = inTile ? tileDockIcons : dockIcons;
  if (!container || !iconsEl) return;
  iconsEl.innerHTML = '';

  // Chat-only mode (traveler deactivated): the chrome dock stays hidden —
  // the compose input re-shows it via the compose-active CSS when needed.
  if (!inTile && !isPluginActive('traveler')) {
    if (!composeOpen) dock.classList.add('hidden');
    return;
  }

  if (!list.length) {
    if (!inTile && !composeOpen) dock.classList.add('hidden');
    return;
  }

  if (!inTile && !composeOpen) {
    dock.classList.remove('hidden');
  }

  const visible = list.slice(0, MAX_VISIBLE);
  const overflow = list.length - visible.length;

  visible.forEach((item) => {
    iconsEl.appendChild(dockButton({
      icon: iconForArtifact(item),
      label: labelForArtifact(item),
      active: item.id === activeArtifactId,
      onClick: () => openSavedArtifact(item.id),
    }));
  });

  if (overflow > 0) {
    iconsEl.appendChild(dockButton({
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
