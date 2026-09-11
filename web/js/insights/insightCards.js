/**
 * Renders persistent notification-style insight cards (weather, events, places).
 * Cards are built with the ui.insightCard composite — always in the active theme.
 */

import { apiFetch } from '../api.js';
import { getOllamaModel } from '../preferences.js';
import { setDockStep, clearDockStep } from '../dockStep.js';
import { insightCard, reveal } from '../../ui/index.js';
import {
  setInsightCards,
  getVisibleCards,
  dismissInsightCard,
  clearInsightCards,
} from './insightStore.js';

const container = document.getElementById('insight-cards');

/** Default icon names when the API omits `icon`. */
const ICON_FALLBACK = {
  weather: 'insights/weather-cloud',
  event: 'insights/event',
  place: 'insights/place-landmark',
};

/** Resolve a theme icon name from the API `icon` field or kind. */
function iconName(card) {
  const stem = card.icon || ICON_FALLBACK[card.kind]?.replace('insights/', '') || 'place-landmark';
  return stem.startsWith('insights/') ? stem : `insights/${stem}`;
}

function render() {
  if (!container) return;
  container.innerHTML = '';

  const list = getVisibleCards();
  if (!list.length) {
    container.classList.add('hidden');
    return;
  }

  container.classList.remove('hidden');

  list.forEach((card, i) => {
    const el = insightCard({
      icon: iconName(card),
      kind: card.kind || 'place',
      title: card.title,
      body: card.body,
      onDismiss: () => dismissInsightCard(card.id),
    });
    el.dataset.id = card.id;
    el.dataset.reveal = String(Math.min(i * 70, 350));
    container.appendChild(el);
  });
  reveal(container);
}

/**
 * Fetch context insights from the backend and show cards.
 * @param {string} destination
 * @param {number} lat
 * @param {number} lon
 */
// Guards against overlapping context loads: when the user moves on to a new
// destination, a slow older fetch must not overwrite the newer cards.
let insightFetchToken = 0;

export async function loadContextInsights(destination, lat, lon) {
  if (!destination || lat == null || lon == null) return;
  const token = ++insightFetchToken;

  setDockStep('Loading local insights…');

  try {
    const q = new URLSearchParams({
      destination,
      lat: String(lat),
      lon: String(lon),
    });
    const model = getOllamaModel();
    if (model) q.set('ollama_model', model);
    setDockStep('Researching events & places…');
    const res = await apiFetch(`/api/insights/context?${q}`);
    if (token !== insightFetchToken) return; // a newer context won
    setInsightCards(destination, res.data || []);
  } catch (e) {
    if (e.status !== 401) {
      console.warn('Insights fetch failed:', e);
    }
    if (token === insightFetchToken) clearInsightCards();
  } finally {
    if (token === insightFetchToken) clearDockStep();
  }
}

export function initInsightCards() {
  window.addEventListener('insights:updated', () => render());
  render();
}
