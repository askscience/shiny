import { apiFetch } from './api.js';

const cache = new Map();
let summaries = [];
let activeDestinationKey = localStorage.getItem('dock.destination') || '';

export function normalizeDestinationKey(value) {
  if (!value) return '';
  return String(value)
    .toLowerCase()
    .replace(/^trip to\s+/i, '')
    .trim();
}

export function destinationKeyForSummary(s) {
  if (!s) return '';
  if (s.destination) return normalizeDestinationKey(s.destination);
  if (s.theme && s.subtitle) return normalizeDestinationKey(s.subtitle);
  if ((s.type || s.artifact_type) === 'travel_plan') {
    return normalizeDestinationKey(s.title);
  }
  return '';
}

export function destinationKeyForArtifact(artifact) {
  if (!artifact) return '';
  if (artifact.destination) return normalizeDestinationKey(artifact.destination);
  if (artifact.theme && artifact.subtitle) return normalizeDestinationKey(artifact.subtitle);
  if (artifact.type === 'travel_plan') return normalizeDestinationKey(artifact.title);
  return '';
}

export function getActiveDestination() {
  return activeDestinationKey;
}

export function setActiveDestination(key) {
  activeDestinationKey = normalizeDestinationKey(key);
  if (activeDestinationKey) {
    localStorage.setItem('dock.destination', activeDestinationKey);
  } else {
    localStorage.removeItem('dock.destination');
  }
  window.dispatchEvent(new CustomEvent('artifact:dock', { detail: getDockSummaries() }));
}

/** Keep only guides for the active city in the dock */
export function applyDockDestinationGroup(destinationKey, artifactIds = []) {
  const key = normalizeDestinationKey(destinationKey);
  if (!key) return;
  activeDestinationKey = key;
  localStorage.setItem('dock.destination', key);
  const idSet = new Set(artifactIds);
  summaries = summaries.filter((s) => {
    if (idSet.has(s.id)) return true;
    return destinationKeyForSummary(s) === key;
  });
  summaries = dedupeSummaries(summaries);
  window.dispatchEvent(new CustomEvent('artifact:dock', { detail: getDockSummaries() }));
}

export function getDockSummaries() {
  if (!activeDestinationKey) {
    return summaries.slice(0, 5);
  }
  const filtered = summaries.filter((s) => destinationKeyForSummary(s) === activeDestinationKey);
  return filtered.length ? filtered : summaries.slice(0, 5);
}

function pickDefaultDestination(list) {
  const plan = list.find((s) => (s.type || s.artifact_type) === 'travel_plan');
  if (plan) return destinationKeyForSummary(plan);
  return destinationKeyForSummary(list[0]);
}

export function normalizeArtifact(raw) {
  if (!raw) return raw;
  const type = raw.type || raw.artifact_type || 'site_info';
  return {
    ...raw,
    type,
    theme: raw.theme || null,
    destination: raw.destination || null,
    narrative: raw.narrative || null,
    days: raw.days || [],
    geometry: raw.geometry || [],
    route: raw.route || null,
    sections: raw.sections || [],
    actions: raw.actions || [],
    coordinates: raw.coordinates || null,
  };
}

function dedupeSummaries(list) {
  const byId = new Map();
  for (const s of list) {
    const existing = byId.get(s.id);
    if (!existing || (s.updated_at || '') > (existing.updated_at || '')) {
      byId.set(s.id, s);
    }
  }
  const byTitle = new Map();
  for (const s of byId.values()) {
    const dest = destinationKeyForSummary(s);
    const key = `${dest}:${s.type || 'unknown'}:${s.theme || ''}:${(s.title || '').toLowerCase()}`;
    const prev = byTitle.get(key);
    if (!prev || (s.updated_at || '') > (prev.updated_at || '')) {
      byTitle.set(key, s);
    }
  }
  return [...byTitle.values()]
    .map((s) => ({ ...s, type: s.type || s.artifact_type || 'site_info' }))
    .sort((a, b) => (b.updated_at || '').localeCompare(a.updated_at || ''));
}

export function getSummaries() {
  return summaries;
}

/** Display name for a saved destination chip (city the user asked for). */
export function destinationLabel(summary) {
  if (!summary) return '';
  if (summary.destination) return formatPlaceName(summary.destination);
  if ((summary.type || summary.artifact_type) === 'travel_plan') {
    return formatPlaceName(summary.title);
  }
  if (summary.subtitle) return formatPlaceName(summary.subtitle);
  return formatPlaceName(summary.title);
}

function formatPlaceName(raw) {
  if (!raw) return '';
  return String(raw)
    .replace(/^trip to\s+/i, '')
    .replace(/\s+travel plan$/i, '')
    .trim();
}

function summaryPriority(s) {
  const type = s.type || s.artifact_type || '';
  if (type === 'travel_plan') return 3;
  if (s.theme === 'overview') return 2;
  return 1;
}

/**
 * Unique saved destinations for the left HUD (one chip per city).
 * @returns {{ key: string, label: string, artifactId: string }[]}
 */
export function getSavedDestinations() {
  const entries = new Map();
  for (const s of summaries) {
    const key = destinationKeyForSummary(s);
    if (!key) continue;
    const label = destinationLabel(s);
    if (!label) continue;
    const score = summaryPriority(s);
    const prev = entries.get(key);
    if (!prev || score > prev.score || (s.updated_at || '') > (prev.updated_at || '')) {
      entries.set(key, {
        key,
        label,
        artifactId: s.id,
        score,
        updated_at: s.updated_at || '',
      });
    }
  }
  return [...entries.values()]
    .sort((a, b) => (b.updated_at || '').localeCompare(a.updated_at || ''))
    .map(({ key, label, artifactId }) => ({ key, label, artifactId }));
}

export function getCachedArtifact(id) {
  return cache.get(id) || null;
}

export function cacheArtifactLocal(artifact) {
  const normalized = normalizeArtifact(artifact);
  cache.set(normalized.id, normalized);
  return normalized;
}

export function removeSummary(id) {
  summaries = summaries.filter((s) => s.id !== id);
  cache.delete(id);
  window.dispatchEvent(new CustomEvent('artifact:dock', { detail: getDockSummaries() }));
}

export async function loadArtifacts() {
  try {
    const res = await apiFetch('/api/artifacts');
    summaries = dedupeSummaries(res.data || []);
    if (!activeDestinationKey) {
      const def = pickDefaultDestination(summaries);
      if (def) activeDestinationKey = def;
    }
    const dock = getDockSummaries();
    await Promise.all(
      dock.map(async (s) => {
        try {
          await getArtifact(s.id);
        } catch (_) {
          /* keep summary; open will retry */
        }
      })
    );
    window.dispatchEvent(new CustomEvent('artifact:dock', { detail: dock }));
    return dock;
  } catch (e) {
    if (e.status !== 401) {
      console.warn('Failed to load artifacts:', e);
    }
    summaries = [];
    window.dispatchEvent(new CustomEvent('artifact:dock', { detail: [] }));
    return [];
  }
}

export async function upsertArtifact(artifact, tripId = null) {
  const normalized = normalizeArtifact(artifact);
  try {
    const res = await apiFetch('/api/artifacts', {
      method: 'POST',
      body: JSON.stringify({ artifact: normalized, trip_id: tripId }),
    });
    const saved = normalizeArtifact(res.data);
    cache.set(saved.id, saved);
    const summary = {
      id: saved.id,
      type: saved.type,
      title: saved.title,
      theme: saved.theme || null,
      destination: saved.destination || null,
      subtitle: saved.subtitle || null,
      updated_at: new Date().toISOString(),
    };
    const idx = summaries.findIndex((s) => s.id === saved.id);
    if (idx >= 0) {
      summaries[idx] = summary;
      window.dispatchEvent(new CustomEvent('artifact:updated', { detail: saved }));
    } else {
      summaries.unshift(summary);
      window.dispatchEvent(new CustomEvent('artifact:saved', { detail: saved }));
    }
    summaries = dedupeSummaries(summaries);
    window.dispatchEvent(new CustomEvent('artifact:dock', { detail: getDockSummaries() }));
    return saved;
  } catch (e) {
    if (e.status !== 401) {
      console.warn('Failed to save artifact:', e);
    }
    cache.set(normalized.id, normalized);
    const summary = {
      id: normalized.id,
      type: normalized.type,
      title: normalized.title,
      theme: normalized.theme || null,
      destination: normalized.destination || null,
      subtitle: normalized.subtitle || null,
      updated_at: new Date().toISOString(),
    };
    if (!summaries.some((s) => s.id === normalized.id)) {
      summaries.unshift(summary);
      summaries = dedupeSummaries(summaries);
      window.dispatchEvent(new CustomEvent('artifact:dock', { detail: getDockSummaries() }));
    }
    return normalized;
  }
}

export async function getArtifact(id) {
  if (cache.has(id)) return normalizeArtifact(cache.get(id));
  const res = await apiFetch(`/api/artifacts/${id}`);
  const artifact = normalizeArtifact(res.data);
  cache.set(id, artifact);
  return artifact;
}
