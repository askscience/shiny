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
import { dockButton, iconForArtifact, labelForArtifact, listReveal, cardEnter } from '../ui/index.js';
import { artifactBelongsTo as belongsTo } from './artifactStore.js';
import { t, tp } from './i18n.js';

const panel = document.getElementById('travel-panel');
const backdrop = document.getElementById('travel-panel-backdrop');
const dock = document.getElementById('artifact-dock');
const dockIcons = document.getElementById('artifact-dock-icons');
let currentArtifact = null;
let activeArtifactId = null;

/** One dock icon per topic slot (overview + 3 themes). */
const MAX_VISIBLE = 4;

/** The tile dock's cards at its last render: only newcomers are revealed. */
let tileDockSeen = new Set();

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
        detail: {
          message: t('artifacts.gone', null, 'That guide is no longer saved — plan the trip again'),
          type: 'error',
        },
      }));
    } else {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: {
          message: e.message || t('artifacts.loadFailed', null, 'Could not load saved card'),
          type: 'error',
        },
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
  // dock stays with its window instead of jumping to the chrome dock).
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
  const tileDockWasHidden = !tileDock || tileDock.classList.contains('hidden');
  if (tileDock) tileDock.classList.toggle('hidden', !inTile || !list.length);
  if (tileDock) labelTileDock(tileDock, tileDockIcons, list.length);

  const container = inTile ? tileDock : dock;
  const iconsEl = inTile ? tileDockIcons : dockIcons;
  if (!container || !iconsEl) return;
  iconsEl.innerHTML = '';

  const composeOpen = document.body.classList.contains('compose-active');

  // The chrome dock is also the status bubble floating above the voice bar
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

  const fresh = [];
  visible.forEach((item) => {
    const btn = dockButton({
      icon: iconForArtifact(item),
      label: labelForArtifact(item),
      active: item.id === activeArtifactId,
      onClick: () => openSavedArtifact(item.id),
    });
    iconsEl.appendChild(btn);
    if (!tileDockSeen.has(item.id)) fresh.push(btn);
  });

  if (overflow > 0) {
    iconsEl.appendChild(dockButton({
      text: `+${overflow}`,
      label: tp('artifacts.moreSaved', overflow, null, `${overflow} more saved cards`),
      onClick: () => openSavedArtifact(list[MAX_VISIBLE].id),
    }));
  }

  // Motion for the traveler window's dock only: the dock arriving is a card
  // entering; a card saved into a dock already showing is a list reveal of
  // just the newcomer (not every re-render).
  if (inTile) {
    if (tileDockWasHidden && list.length) {
      void cardEnter(tileDock);
      void listReveal(iconsEl.children);
    } else if (fresh.length) {
      void listReveal(fresh);
    }
    tileDockSeen = new Set(visible.map((item) => item.id));
  }
}

/**
 * The tile dock's accessible name and its mono eyebrow ("SAVED · 3"). The dock
 * element is built by tiles.js; the eyebrow is added here, once.
 */
function labelTileDock(tileDock, iconsEl, count) {
  tileDock.setAttribute('aria-label', t('hud.savedCards', null, 'Saved cards'));
  let eyebrow = tileDock.querySelector(':scope > .map-tile-dock-eyebrow');
  if (!eyebrow) {
    eyebrow = document.createElement('span');
    eyebrow.className = 'map-tile-dock-eyebrow';
    eyebrow.setAttribute('aria-hidden', 'true');
    tileDock.insertBefore(eyebrow, iconsEl || tileDock.firstChild);
  }
  eyebrow.textContent = `${t('artifacts.dockEyebrow', null, 'Saved')} · ${count}`;
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
