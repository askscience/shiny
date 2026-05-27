/**
 * Insight card state: in-memory cards + dismissed IDs per destination (localStorage).
 */

const STORAGE_KEY = 'insights.dismissed';
const MAX_CARDS = 5;

/** @typedef {{ id: string, kind: string, title: string, body: string, icon: string }} InsightCard */

let activeDestination = '';
/** @type {InsightCard[]} */
let cards = [];

function loadDismissedMap() {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY) || '{}');
  } catch {
    return {};
  }
}

function saveDismissedMap(map) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(map));
}

function dismissedForDestination(destKey) {
  const map = loadDismissedMap();
  return new Set(map[destKey] || []);
}

function persistDismiss(destKey, cardId) {
  const map = loadDismissedMap();
  const set = new Set(map[destKey] || []);
  set.add(cardId);
  map[destKey] = [...set];
  saveDismissedMap(map);
}

export function getActiveInsightDestination() {
  return activeDestination;
}

/** Cards currently visible (already filtered by dismiss). */
export function getVisibleCards() {
  return cards;
}

/**
 * Replace cards for a destination; filters out previously dismissed IDs.
 * @param {string} destination
 * @param {InsightCard[]} incoming
 */
export function setInsightCards(destination, incoming) {
  activeDestination = (destination || '').trim();
  const key = activeDestination.toLowerCase();
  const dismissed = dismissedForDestination(key);

  cards = (incoming || [])
    .filter((c) => c?.id && !dismissed.has(c.id))
    .slice(0, MAX_CARDS);

  window.dispatchEvent(new CustomEvent('insights:updated', { detail: cards }));
}

export function dismissInsightCard(cardId) {
  const key = activeDestination.toLowerCase();
  if (!key) return;
  persistDismiss(key, cardId);
  cards = cards.filter((c) => c.id !== cardId);
  window.dispatchEvent(new CustomEvent('insights:updated', { detail: cards }));
}

export function clearInsightCards() {
  cards = [];
  activeDestination = '';
  window.dispatchEvent(new CustomEvent('insights:updated', { detail: [] }));
}
