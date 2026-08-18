/**
 * composites — app-flavored components built from the primitives.
 * These are the surfaces plugin content renders on: artifact panels,
 * the artifact dock, and insight cards. Plugins produce JSON artifacts;
 * core renders them here, always in the active theme.
 */

import { icon } from '../icon.js';
import { button } from './button.js';
import { stat } from './data.js';

/* ── Artifact → themed icon mapping ─────────────────────────── */

const TYPE_ICONS = {
  monument_info: 'artifacts/monument',
  site_info: 'artifacts/site',
  poi_list: 'artifacts/poi-list',
  route_preview: 'artifacts/route',
  tour_plan: 'artifacts/tour',
  travel_plan: 'artifacts/plan',
};

const THEME_ICONS = {
  overview: 'artifacts/plan',
  nightlife: 'artifacts/nightlife',
  food: 'artifacts/food',
  culture: 'artifacts/culture',
};

const THEME_LABELS = {
  overview: 'Journey',
  nightlife: 'After dark',
  food: 'Eat & drink',
  culture: 'Culture',
};

const PLAN_TYPES = new Set(['travel_plan', 'tour_plan']);

/** Theme icon name for an artifact (or summary). */
export function iconForArtifact(item) {
  if (item?.theme && THEME_ICONS[item.theme]) return THEME_ICONS[item.theme];
  const type = item?.type || item?.artifact_type || 'site_info';
  return TYPE_ICONS[type] || 'artifacts/default';
}

export function labelForArtifact(item) {
  const type = item?.type || item?.artifact_type || 'site_info';
  return item?.theme && THEME_LABELS[item.theme]
    ? `${THEME_LABELS[item.theme]}: ${item.title || type}`
    : (item?.title || type);
}

/* ── Dock button ────────────────────────────────────────────── */

export function dockButton(o = {}) {
  const el = document.createElement('button');
  el.type = 'button';
  el.className = 'ui-dock-btn glass-blur';
  if (o.active) el.classList.add('is-active');
  if (o.label) {
    el.title = o.label;
    el.setAttribute('aria-label', o.label);
  }
  if (o.text) {
    el.textContent = o.text;
    el.classList.add('ui-dock-btn--text');
  } else {
    el.appendChild(icon(o.icon || 'artifacts/default', { size: 19 }));
  }
  if (o.onClick) el.addEventListener('click', o.onClick);
  el.setActive = (v) => el.classList.toggle('is-active', v);
  return el;
}

/* ── Insight card ───────────────────────────────────────────── */

export function insightCard(o = {}) {
  const el = document.createElement('div');
  el.className = 'ui-insight glass-blur';
  el.dataset.reveal = '';
  if (o.kind) el.classList.add(`ui-insight--${o.kind}`);

  el.appendChild(icon(o.icon || 'ui/info', { size: 18 }));

  const main = document.createElement('div');
  main.className = 'ui-insight-main';
  const title = document.createElement('p');
  title.className = 'ui-insight-title';
  title.textContent = o.title || '';
  main.appendChild(title);
  if (o.body) {
    const body = document.createElement('p');
    body.className = 'ui-insight-body';
    if (typeof o.body === 'string') body.textContent = o.body;
    else body.appendChild(o.body);
    main.appendChild(body);
  }
  el.appendChild(main);

  if (o.onDismiss) {
    const close = document.createElement('button');
    close.type = 'button';
    close.className = 'ui-insight-close';
    close.setAttribute('aria-label', 'Dismiss');
    close.textContent = '×';
    close.addEventListener('click', () => o.onDismiss(el));
    el.appendChild(close);
  }
  return el;
}

/* ── Artifact panel ─────────────────────────────────────────── */

function isBulletList(text) {
  if (!text) return false;
  const lines = text.split('\n').map((l) => l.trim()).filter(Boolean);
  if (lines.length < 2) return /^\d{1,2}[:.]|^[-•*]\s/.test(text);
  return lines.filter((l) => /^[-•*]\s|^\d{1,2}[:.]/.test(l)).length >= lines.length * 0.4;
}

function prose(text, className = 'ui-artifact-prose') {
  const frag = document.createDocumentFragment();
  if (!text?.trim()) return frag;
  const blocks = text.split(/\n\n+/).map((p) => p.trim()).filter(Boolean);
  for (const para of blocks) {
    const p = document.createElement('p');
    p.className = className;
    p.textContent = para.replace(/\n/g, ' ');
    frag.appendChild(p);
  }
  return frag;
}

function bulletItems(text) {
  return text.split(/[•\n;]/).map((s) => s.trim().replace(/^[-•*]\s+/, '')).filter(Boolean);
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
      days.push({
        day: parseInt(m[2], 10),
        title: sec.label.replace(dayRe, '').trim() || `Day ${m[2]}`,
        story: isBulletList(story) ? null : story,
        items: isBulletList(story) ? bulletItems(story) : [],
      });
    } else if (days.length) {
      const last = days[days.length - 1];
      if (last.story) last.story += `\n\n${sec.label}: ${sec.value}`;
      else last.items.push(`${sec.label}: ${sec.value}`);
    }
  }
  return days;
}

function eyebrowFor(artifact) {
  if (artifact.theme && THEME_LABELS[artifact.theme]) return THEME_LABELS[artifact.theme];
  return (artifact.type || 'guide').replace(/_/g, ' ');
}

function hero(artifact) {
  const el = document.createElement('div');
  el.className = 'ui-artifact-hero';

  const eyebrow = document.createElement('div');
  eyebrow.className = 'ui-artifact-eyebrow';
  eyebrow.appendChild(icon(iconForArtifact(artifact), { size: 14 }));
  const label = document.createElement('span');
  label.className = 'ui-eyebrow';
  label.textContent = eyebrowFor(artifact);
  eyebrow.append(label);
  el.appendChild(eyebrow);

  const title = document.createElement('h1');
  title.className = 'ui-artifact-title';
  title.textContent = artifact.title || '';
  el.appendChild(title);

  if (artifact.subtitle) {
    const sub = document.createElement('p');
    sub.className = 'ui-artifact-sub';
    sub.textContent = artifact.subtitle;
    el.appendChild(sub);
  }

  if (artifact.route) {
    const stats = document.createElement('div');
    stats.className = 'ui-artifact-stats';
    stats.append(
      stat({ value: String(Math.round(artifact.route.distance_km)), unit: 'km', label: 'Distance' }),
      stat({ value: String(Math.round(artifact.route.duration_min)), unit: 'min', label: 'Drive' }),
    );
    el.appendChild(stats);
  }
  return el;
}

function dayBlock(day, index) {
  const block = document.createElement('article');
  block.className = 'ui-artifact-day';
  block.dataset.reveal = String(Math.min(index * 60, 360));

  const num = document.createElement('div');
  num.className = 'ui-artifact-day-num';
  num.textContent = String(day.day).padStart(2, '0');
  block.appendChild(num);

  const body = document.createElement('div');
  body.className = 'ui-artifact-day-body';
  const title = document.createElement('h2');
  title.className = 'ui-artifact-day-title';
  title.textContent = day.title || `Day ${day.day}`;
  body.appendChild(title);

  if (day.story) {
    body.appendChild(prose(day.story, 'ui-artifact-day-items'));
  } else if (day.items?.length) {
    const ul = document.createElement('ul');
    ul.className = 'ui-artifact-day-items';
    for (const item of day.items) {
      const li = document.createElement('li');
      li.textContent = item;
      ul.appendChild(li);
    }
    body.appendChild(ul);
  }
  block.appendChild(body);
  return block;
}

function sectionHead(text) {
  const h = document.createElement('h3');
  h.className = 'ui-eyebrow ui-artifact-section-head';
  h.textContent = text;
  return h;
}

function sectionRow(sec) {
  const row = document.createElement('div');
  row.className = 'ui-artifact-section';
  const label = document.createElement('div');
  label.className = 'ui-artifact-section-label';
  label.textContent = sec.label;
  const value = document.createElement('div');
  value.className = 'ui-artifact-section-value';
  value.textContent = sec.value;
  row.append(label, value);
  return row;
}

const DAY_RE = /^(GIORNO|DAY|JOUR|DÍA|DIA)\s*\d/i;

function buildBody(artifact, body) {
  const type = artifact.type;
  const isPlan = PLAN_TYPES.has(type) || artifact.theme === 'overview';

  if (artifact.narrative) body.appendChild(prose(artifact.narrative));

  const days = normalizeDays(artifact);
  if (days.length) {
    if (isPlan) body.appendChild(sectionHead(days.length === 1 ? 'Your day' : 'Day by day'));
    const list = document.createElement('div');
    list.className = 'ui-artifact-days';
    days.forEach((day, i) => list.appendChild(dayBlock(day, i)));
    body.appendChild(list);
  }

  const sections = (artifact.sections || []).filter((sec) => !DAY_RE.test(sec.label || ''));
  if (isPlan && artifact.narrative) return; // plan + narrative: days already told the story
  for (const sec of sections) {
    if (!isBulletList(sec.value)) {
      body.appendChild(sectionHead(sec.label));
      body.appendChild(prose(sec.value));
    } else if (isPlan) {
      body.appendChild(sectionRow(sec));
    } else {
      body.appendChild(sectionHead(sec.label));
      const ul = document.createElement('ul');
      ul.className = 'ui-artifact-day-items';
      for (const item of bulletItems(sec.value)) {
        const li = document.createElement('li');
        li.textContent = item;
        ul.appendChild(li);
      }
      body.appendChild(ul);
    }
  }
}

/**
 * Full artifact panel (plan / guide / generic — chosen from the artifact).
 * @param {object} artifact  normalized artifact JSON (plugin content)
 * @param {object} o
 * @param {() => void} [o.onClose]
 * @param {(action: object, artifact: object) => void} [o.onAction]
 * @param {(artifact: object) => void} [o.onNavigate]
 */
export function artifactPanel(artifact, o = {}) {
  const el = document.createElement('div');
  el.className = 'ui-artifact';

  if (o.onClose) {
    const close = document.createElement('button');
    close.type = 'button';
    close.className = 'ui-artifact-close glass-blur';
    close.setAttribute('aria-label', 'Close');
    close.textContent = '×';
    close.addEventListener('click', o.onClose);
    el.appendChild(close);
  }

  const scroll = document.createElement('div');
  scroll.className = 'ui-artifact-scroll';
  scroll.appendChild(hero(artifact));

  const body = document.createElement('div');
  body.className = 'ui-artifact-body';
  buildBody(artifact, body);
  scroll.appendChild(body);

  const actions = document.createElement('div');
  actions.className = 'ui-artifact-actions';
  if (o.onNavigate) {
    actions.appendChild(button({
      label: artifact.coordinates ? 'Show route on map' : 'Map',
      variant: 'ghost',
      icon: 'artifacts/route',
      onClick: () => o.onNavigate(artifact),
    }));
  }
  for (const act of artifact.actions || []) {
    if (act.tool === 'map_route' && act.label?.toLowerCase().includes('route')) continue;
    actions.appendChild(button({
      label: act.label,
      variant: 'ghost',
      onClick: () => o.onAction?.(act, artifact),
    }));
  }
  if (actions.childElementCount) scroll.appendChild(actions);

  el.appendChild(scroll);
  return el;
}
