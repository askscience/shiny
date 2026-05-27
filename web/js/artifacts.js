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

const panel = document.getElementById('travel-panel');
const backdrop = document.getElementById('travel-panel-backdrop');
const dock = document.getElementById('artifact-dock');
let currentArtifact = null;
let activeArtifactId = null;

const TYPE_ICONS = {
  monument_info: 'monument.svg',
  site_info: 'site.svg',
  poi_list: 'poi-list.svg',
  route_preview: 'route.svg',
  tour_plan: 'tour.svg',
  travel_plan: 'plan.svg',
};

const THEME_ICONS = {
  overview: 'plan.svg',
  nightlife: 'nightlife.svg',
  food: 'food.svg',
  culture: 'culture.svg',
};

const THEME_LABELS = {
  overview: 'Journey',
  nightlife: 'After dark',
  food: 'Eat & drink',
  culture: 'Culture',
};

const PLAN_TYPES = new Set(['travel_plan', 'tour_plan']);
const MAX_VISIBLE = 8;

export function iconForArtifact(item) {
  if (item?.theme && THEME_ICONS[item.theme]) {
    return `/icons/artifacts/${THEME_ICONS[item.theme]}`;
  }
  const type = item?.type || item?.artifact_type || 'site_info';
  const file = TYPE_ICONS[type] || 'default.svg';
  return `/icons/artifacts/${file}`;
}

export function iconForType(type) {
  return iconForArtifact({ type });
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

function isBulletList(text) {
  if (!text) return false;
  const lines = text.split('\n').map((l) => l.trim()).filter(Boolean);
  if (lines.length < 2) return /^\d{1,2}[:.]|^[-•*]\s/.test(text);
  return lines.filter((l) => /^[-•*]\s|^\d{1,2}[:.]/.test(l)).length >= lines.length * 0.4;
}

function appendProse(parent, text, className = 'panel-prose') {
  if (!text?.trim()) return;
  const blocks = text.split(/\n\n+/).map((p) => p.trim()).filter(Boolean);
  blocks.forEach((para, i) => {
    const p = document.createElement('p');
    p.className = className;
    p.style.animationDelay = `${0.06 + i * 0.05}s`;
    p.textContent = para.replace(/\n/g, ' ');
    parent.appendChild(p);
  });
}

function normalizeDays(artifact) {
  if (artifact.days?.length) {
    return artifact.days.map((d) => {
      if (d.story) return d;
      if (d.items?.length === 1 && !isBulletList(d.items[0])) {
        return { ...d, story: d.items[0], items: [] };
      }
      const joined = (d.items || []).join('\n');
      if (d.items?.length > 1 && !isBulletList(joined)) {
        return { ...d, story: joined, items: [] };
      }
      return d;
    });
  }

  const days = [];
  const dayRe = /^(GIORNO|DAY|JOUR|DÍA|DIA)\s*(\d+)/i;

  for (const sec of artifact.sections || []) {
    const m = sec.label?.match(dayRe);
    if (m) {
      const story = sec.value?.trim() || '';
      const entry = {
        day: parseInt(m[2], 10),
        title: sec.label.replace(dayRe, '').trim() || `Day ${m[2]}`,
        story: isBulletList(story) ? null : story,
        items: isBulletList(story)
          ? story.split(/[•\n;]/).map((s) => s.trim()).filter(Boolean)
          : [],
      };
      days.push(entry);
    } else if (days.length) {
      const last = days[days.length - 1];
      if (last.story) {
        last.story += `\n\n${sec.label}: ${sec.value}`;
      } else {
        last.items.push(`${sec.label}: ${sec.value}`);
      }
    }
  }
  return days;
}

function eyebrowFor(artifact) {
  if (artifact.theme && THEME_LABELS[artifact.theme]) {
    return THEME_LABELS[artifact.theme];
  }
  return (artifact.type || 'guide').replace(/_/g, ' ');
}

function renderDayBlocks(days, scroll) {
  days.forEach((day, i) => {
    const block = document.createElement('article');
    block.className = 'panel-day';
    block.style.animationDelay = `${0.08 + i * 0.07}s`;

    const num = document.createElement('div');
    num.className = 'panel-day-num';
    num.textContent = String(day.day).padStart(2, '0');
    block.appendChild(num);

    const title = document.createElement('h2');
    title.className = 'panel-day-title';
    title.textContent = day.title || `Day ${day.day}`;
    block.appendChild(title);

    if (day.story) {
      appendProse(block, day.story, 'panel-day-story');
    } else if (day.items?.length) {
      const ul = document.createElement('ul');
      ul.className = 'panel-day-items';
      day.items.forEach((item) => {
        const li = document.createElement('li');
        li.textContent = item;
        ul.appendChild(li);
      });
      block.appendChild(ul);
    }

    scroll.appendChild(block);
  });
}

function openPanel() {
  if (!panel) return;
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

function renderHero(artifact, scroll) {
  const eyebrow = document.createElement('div');
  eyebrow.className = 'panel-eyebrow';
  eyebrow.textContent = eyebrowFor(artifact);
  scroll.appendChild(eyebrow);

  const hero = document.createElement('h1');
  hero.className = 'panel-hero-title';
  hero.textContent = artifact.title;
  scroll.appendChild(hero);

  if (artifact.subtitle) {
    const sub = document.createElement('p');
    sub.className = 'panel-hero-sub';
    sub.textContent = artifact.subtitle;
    scroll.appendChild(sub);
  }
}

function renderPlanPanel(artifact, scroll) {
  renderHero(artifact, scroll);

  if (artifact.route) {
    const meta = document.createElement('div');
    meta.className = 'panel-route-meta';
    meta.innerHTML = `
      <div class="panel-route-stat"><span>Distance</span>${artifact.route.distance_km.toFixed(0)} km</div>
      <div class="panel-route-stat"><span>Drive</span>${Math.round(artifact.route.duration_min)} min</div>
    `;
    scroll.appendChild(meta);
  }

  if (artifact.narrative) {
    const lead = document.createElement('div');
    lead.className = 'panel-lead';
    appendProse(lead, artifact.narrative);
    scroll.appendChild(lead);
  }

  const days = normalizeDays(artifact);
  if (days.length) {
    const dayHead = document.createElement('h3');
    dayHead.className = 'panel-section-head';
    dayHead.textContent = days.length === 1 ? 'Your day' : 'Day by day';
    scroll.appendChild(dayHead);
    renderDayBlocks(days, scroll);
  }

  const dayRe = /^(GIORNO|DAY|JOUR|DÍA|DIA)\s*\d/i;
  const extra = (artifact.sections || []).filter((sec) => !dayRe.test(sec.label || ''));
  if (extra.length && !artifact.narrative) {
    extra.forEach((sec) => {
      if (!isBulletList(sec.value)) {
        const h = document.createElement('h3');
        h.className = 'panel-section-head';
        h.textContent = sec.label;
        scroll.appendChild(h);
        appendProse(scroll, sec.value);
      } else {
        const row = document.createElement('div');
        row.className = 'panel-section';
        row.innerHTML = `<div class="panel-section-label">${esc(sec.label)}</div><div class="panel-section-value">${esc(sec.value)}</div>`;
        scroll.appendChild(row);
      }
    });
  }
}

function renderGuidePanel(artifact, scroll) {
  renderHero(artifact, scroll);
  if (artifact.narrative) {
    const lead = document.createElement('div');
    lead.className = 'panel-lead';
    appendProse(lead, artifact.narrative);
    scroll.appendChild(lead);
  }
  const days = normalizeDays(artifact);
  if (days.length) renderDayBlocks(days, scroll);

  const proseSections = (artifact.sections || []).filter((sec) => !isBulletList(sec.value));
  proseSections.forEach((sec) => {
    const h = document.createElement('h3');
    h.className = 'panel-section-head';
    h.textContent = sec.label;
    scroll.appendChild(h);
    appendProse(scroll, sec.value);
  });

  const listSections = (artifact.sections || []).filter((sec) => isBulletList(sec.value));
  listSections.forEach((sec) => {
    const h = document.createElement('h3');
    h.className = 'panel-section-head';
    h.textContent = sec.label;
    scroll.appendChild(h);
    const ul = document.createElement('ul');
    ul.className = 'panel-day-items';
    sec.value.split(/[•\n;]/).map((s) => s.trim()).filter(Boolean).forEach((item) => {
      const li = document.createElement('li');
      li.textContent = item;
      ul.appendChild(li);
    });
    scroll.appendChild(ul);
  });
}

function renderGenericPanel(artifact, scroll) {
  if (artifact.narrative || artifact.theme) {
    renderGuidePanel(artifact, scroll);
    return;
  }
  renderHero(artifact, scroll);
  (artifact.sections || []).forEach((sec) => {
    if (!isBulletList(sec.value)) {
      const h = document.createElement('h3');
      h.className = 'panel-section-head';
      h.textContent = sec.label;
      scroll.appendChild(h);
      appendProse(scroll, sec.value);
    } else {
      const row = document.createElement('div');
      row.className = 'panel-section';
      row.innerHTML = `<div class="panel-section-label">${esc(sec.label)}</div><div class="panel-section-value">${esc(sec.value)}</div>`;
      scroll.appendChild(row);
    }
  });
}

export function renderArtifact(artifact, { focus = true } = {}) {
  if (!panel) return;

  const normalized = normalizeArtifact(artifact);
  currentArtifact = normalized;
  activeArtifactId = normalized.id;
  cacheArtifactLocal(normalized);
  panel.innerHTML = '';

  const closeBtn = document.createElement('button');
  closeBtn.className = 'panel-close';
  closeBtn.setAttribute('aria-label', 'Close');
  closeBtn.innerHTML = '&times;';
  closeBtn.addEventListener('click', clearArtifacts);
  panel.appendChild(closeBtn);

  const scroll = document.createElement('div');
  scroll.className = 'panel-scroll';

  const type = normalized.type;
  if (PLAN_TYPES.has(type) || normalized.theme === 'overview') {
    renderPlanPanel(normalized, scroll);
  } else if (normalized.narrative || normalized.theme) {
    renderGuidePanel(normalized, scroll);
  } else {
    renderGenericPanel(normalized, scroll);
  }

  const navBtn = document.createElement('button');
  navBtn.className = 'panel-action-btn';
  navBtn.textContent = normalized.coordinates ? 'Show route on map' : 'Map';
  navBtn.addEventListener('click', () => handleNavigate(normalized));
  const actions = document.createElement('div');
  actions.className = 'panel-actions';
  actions.appendChild(navBtn);

  (normalized.actions || []).forEach((act) => {
    if (act.tool === 'map_route' && act.label.toLowerCase().includes('route')) return;
    const btn = document.createElement('button');
    btn.className = 'panel-action-btn';
    btn.textContent = act.label;
    btn.addEventListener('click', () => handleAction(act, normalized));
    actions.appendChild(btn);
  });
  scroll.appendChild(actions);

  panel.appendChild(scroll);

  if (focus) {
    openPanel();
    applyMapForArtifact(normalized);
  }

  renderArtifactDock(getDockSummaries());
}

async function handleNavigate(artifact) {
  const result = await navigateToDestination(artifact);
  if (result?.ok) {
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
  if (!dock) return;
  dock.innerHTML = '';

  const list = artifacts || [];
  if (!list.length) {
    dock.classList.add('hidden');
    return;
  }

  dock.classList.remove('hidden');
  const visible = list.slice(0, MAX_VISIBLE);
  const overflow = list.length - visible.length;

  visible.forEach((item) => {
    const btn = document.createElement('button');
    btn.className = 'artifact-dock-btn';
    if (item.id === activeArtifactId) btn.classList.add('active');
    const type = item.type || item.artifact_type || 'site_info';
    const dockLabel = item.theme && THEME_LABELS[item.theme]
      ? `${THEME_LABELS[item.theme]}: ${item.title || type}`
      : (item.title || type);
    btn.title = dockLabel;
    btn.setAttribute('aria-label', dockLabel);

    const img = document.createElement('img');
    img.src = iconForArtifact(item);
    img.alt = '';
    img.className = 'artifact-dock-icon';
    btn.appendChild(img);

    btn.addEventListener('click', () => openSavedArtifact(item.id));
    dock.appendChild(btn);
  });

  if (overflow > 0) {
    const more = document.createElement('button');
    more.className = 'artifact-dock-btn artifact-dock-overflow';
    more.textContent = `+${overflow}`;
    more.title = `${overflow} more saved cards`;
    more.addEventListener('click', () => {
      openSavedArtifact(list[MAX_VISIBLE].id);
    });
    dock.appendChild(more);
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
