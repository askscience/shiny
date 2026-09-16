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
import { isPluginActive } from './activePlugins.js';
import { openArtifactInTile } from './tiles.js';
import { dockButton, iconForArtifact, labelForArtifact } from '../ui/index.js';
import { artifactBelongsTo as belongsTo } from './artifactStore.js';

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

/**
 * Plugins whose window shows the artifact itself.
 *
 * Each of these renders its own subject directly — the player, the artwork and
 * transport, the live page — so an artifact sheet duplicates it. Keep this in
 * step with the windows that handle their own `artifact:saved` event.
 */
const SELF_REPRESENTING = new Set(['radio', 'youtube', 'browser']);

/** The same idea for a card whose *type* identifies the window. */
const SELF_REPRESENTING_TYPES = new Set(['radio_station', 'youtube_video', 'browser_page']);

export function renderArtifact(artifact, { focus = true } = {}) {
  const normalized = normalizeArtifact(artifact);
  currentArtifact = normalized;
  activeArtifactId = normalized.id;
  cacheArtifactLocal(normalized);

  if (focus) {
    // Some windows *are* their own card. Opening a card sheet over them is
    // noise at best and misleading at worst — the browser sheet, for instance,
    // showed the URL the address bar already showed, with a "Map" button that
    // could not work. For these, just bring the window forward.
    const selfRepresenting = SELF_REPRESENTING.has(normalized.plugin)
      || SELF_REPRESENTING_TYPES.has(normalized.type);
    if (selfRepresenting) {
      window.dispatchEvent(
        new CustomEvent('plugin:focus', { detail: { name: normalized.plugin } }),
      );
    } else {
      // Plugin output is contained inside its own window — the tile sheet.
      const plugin = normalized.plugin || 'traveler';
      void openArtifactInTile(plugin, normalized);
    }
    // A window that draws its own content must not also drive the map: a
    // browser page is not a destination, and applying one would zoom the map
    // to whatever text happened to be in the card.
    if (!selfRepresenting) applyMapForArtifact(normalized);
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
    if (artifact.type === 'youtube_video' || artifact.plugin === 'youtube') {
      // Tapping a saved video card plays it in the YouTube window.
      const play = artifact.actions?.find((a) => a.tool === 'youtube_play');
      if (play) {
        window.dispatchEvent(new CustomEvent('artifact:action', { detail: { action: play, artifact } }));
      } else {
        window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: 'youtube' } }));
      }
      return;
    }
    const destKey = destinationKeyForArtifact(artifact);
    if (destKey) setActiveDestination(destKey);
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

/** True while a live agent status line is showing (dockStep.js owns it). */
function dockStepLive() {
  const stepEl = document.getElementById('artifact-dock-step');
  return !!stepEl && !stepEl.classList.contains('hidden') && !!stepEl.textContent.trim();
}

export function renderArtifactDock(artifacts) {
  // The dock lives INSIDE the traveler window whenever the traveler plugin
  // is active — even on phones where another window is currently shown (the
  // dock stays with its window instead of jumping under the AI sphere).
  // The chrome-bottom dock is only the chat-only fallback.
  const tileDock = document.getElementById('map-tile-dock');
  const tileDockIcons = document.getElementById('map-tile-dock-icons');
  const inTile = !!tileDock && isPluginActive('traveler');

  // A dock inside a window shows that window's own cards and nothing else. The
  // docklist comes from the store, which holds every plugin's artifacts, so an
  // unfiltered render put a browser page (labelled with its raw URL, since that
  // is a browser card's title) inside the traveler window. Whoever draws a dock
  // states whose it is; `null` means the unscoped chrome dock.
  const dockPlugin = inTile ? 'traveler' : null;
  const list = (dockPlugin
    ? (artifacts || []).filter((a) => belongsTo(a, dockPlugin))
    : artifacts) || [];
  document.body.classList.toggle('tile-dock-active', inTile);
  if (tileDock) tileDock.classList.toggle('hidden', !inTile || !list.length);

  const container = inTile ? tileDock : dock;
  const iconsEl = inTile ? tileDockIcons : dockIcons;
  if (!container || !iconsEl) return;
  iconsEl.innerHTML = '';

  const composeOpen = document.body.classList.contains('compose-active');

  // The chrome dock is also the status bubble hanging under the orb
  // (dockStep.js), so a live step line keeps it up whether the saved cards
  // happen to live in this dock or in the traveler window's own dock.
  const keepForStep = dockStepLive();

  if (!inTile) {
    // Chat-only mode (traveler deactivated): the chrome dock stays hidden —
    // the compose input re-shows it via the compose-active CSS when needed.
    if (!isPluginActive('traveler')) {
      if (!composeOpen && !keepForStep) dock.classList.add('hidden');
      return;
    }
    if (!list.length) {
      if (!composeOpen && !keepForStep) dock.classList.add('hidden');
      return;
    }
    if (!composeOpen) {
      dock.classList.remove('hidden');
    }
  } else if (keepForStep || composeOpen) {
    // Traveler mode: the cards are the tile dock's, but the bubble and the
    // composer are still the chrome dock's, and neither is this render's to
    // hide. Without this a composer opened while idle stays hidden away.
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
