/**
 * Renders persistent notification-style insight cards (weather, events, places).
 */

import { apiFetch } from '../api.js';
import {
  setInsightCards,
  getVisibleCards,
  dismissInsightCard,
  clearInsightCards,
} from './insightStore.js';

const container = document.getElementById('insight-cards');

/** Default icon stems when the API omits `icon`. */
const ICON_FALLBACK = {
  weather: 'weather-cloud',
  event: 'event',
  place: 'place-landmark',
};

/** Resolve `/icons/insights/{stem}.svg` from API `icon` field or kind. */
function iconSrc(card) {
  const stem = card.icon || ICON_FALLBACK[card.kind] || 'place-landmark';
  return `/icons/insights/${stem}.svg`;
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
    const el = document.createElement('article');
    el.className = `insight-card kind-${card.kind || 'place'}`;
    el.style.animationDelay = `${i * 0.07}s`;
    el.dataset.id = card.id;

    const iconWrap = document.createElement('div');
    iconWrap.className = 'insight-card-icon';
    const img = document.createElement('img');
    img.src = iconSrc(card);
    img.alt = '';
    iconWrap.appendChild(img);

    const body = document.createElement('div');
    body.className = 'insight-card-body';
    const title = document.createElement('div');
    title.className = 'insight-card-title';
    title.textContent = card.title;
    const text = document.createElement('div');
    text.className = 'insight-card-text';
    text.textContent = card.body;
    body.appendChild(title);
    body.appendChild(text);

    const close = document.createElement('button');
    close.type = 'button';
    close.className = 'insight-card-close';
    close.setAttribute('aria-label', 'Dismiss');
    close.innerHTML = '&times;';
    close.addEventListener('click', () => dismissInsightCard(card.id));

    el.appendChild(iconWrap);
    el.appendChild(body);
    el.appendChild(close);
    container.appendChild(el);
  });
}

/**
 * Fetch context insights from the backend and show cards.
 * @param {string} destination
 * @param {number} lat
 * @param {number} lon
 */
export async function loadContextInsights(destination, lat, lon) {
  if (!destination || lat == null || lon == null) return;

  try {
    const q = new URLSearchParams({
      destination,
      lat: String(lat),
      lon: String(lon),
    });
    const res = await apiFetch(`/api/insights/context?${q}`);
    setInsightCards(destination, res.data || []);
  } catch (e) {
    if (e.status !== 401) {
      console.warn('Insights fetch failed:', e);
    }
    clearInsightCards();
  }
}

export function initInsightCards() {
  window.addEventListener('insights:updated', () => render());
  render();
}
