/**
 * toolCards.js — the small rich card a finished agent tool leaves in the chat.
 *
 * The core streams one `tool` event per tool call
 * (`{ action, result: 'ok'|'error', plugin?, data?, params?, capped? }`, see
 * src/services/tool_cards.rs) and stores the same record as a `tool` row of
 * the conversation, so the live turn and a reopened chat build identical cards.
 *
 * Every card has one anatomy: a header (the tool's monochrome icon, a short
 * mono label such as "Weather · Milano" and an open arrow that focuses the
 * tool's window), then a body shaped by the result — big numbers where they
 * matter (a calculator result, a route's minutes), rows for lists, a pre block
 * for terminal output. Housekeeping tools (window/workspace/plugin plumbing)
 * and the generated image/song (already their own media blocks) get no card.
 *
 * Classes are semantic (`.tool-card`, `.tool-card-head`, `.tool-card-big`, …,
 * `.tool-card--<family>`): web/css/text-input.css lays them out with theme
 * tokens and each theme may skin them. Model output is never parsed as HTML —
 * every string lands through `textContent`.
 *
 * `web/js/tests/toolCards.test.mjs` renders every family in Node.
 */
import { icon } from '../ui/icon.js';
import { controlButton } from '../ui/components/button.js';
import { toolIcon } from './toolIcons.js';
import { t, tp, formatNumber, getLocale } from './i18n.js';
import { weatherIconStem, weatherLabel, tempText } from './weatherShared.js';

/* ── which tools get which card ─────────────────────────────── */

/** Tools that only move the desktop around, or whose result is already its
 *  own chat element: no card when they succeed. */
const HOUSEKEEPING = [
  'show_plugin', 'list_plugins', 'plugin_', 'desktop_', 'window_', 'workspace_',
  'focus_', 'show_artifact', 'update_artifact', 'theme_', 'icon_', 'settings',
  'generate_image', 'generate_song',
];

/** Longest-specific first: the family a tool's card is drawn as. */
const FAMILIES = [
  ['radio_', 'radio'],
  ['calendar_', 'calendar'],
  ['mail_', 'mail'],
  ['file_', 'files'],
  ['calculator_', 'calculator'],
  ['terminal_', 'terminal'],
  ['web_search', 'web'],
  ['browser_', 'browser'],
  ['youtube_', 'youtube'],
  ['image_', 'image'],
  ['map_route', 'route'],
  ['trip_route', 'route'],
  ['navigate_to', 'route'],
  ['map_', 'places'],
  ['plan_trip', 'trip'],
  ['doc_', 'document'],
  ['slide_', 'document'],
  ['pdf_', 'document'],
  ['calc_', 'document'],
  ['studio_', 'document'],
];

/** The window a tool belongs to, when the record does not name its plugin. */
const PLUGIN_BY_PREFIX = [
  ['radio_', 'radio'], ['calendar_', 'calendar'], ['mail_', 'mail'], ['file_', 'files'],
  ['calculator_', 'calculator'], ['terminal_', 'terminal'], ['browser_', 'browser'],
  ['youtube_', 'youtube'], ['image_', 'image'], ['doc_', 'word'], ['slide_', 'impress'],
  ['pdf_', 'pdf'], ['calc_', 'calc'], ['studio_', 'studio'], ['updates_', 'updates'],
];

/** Header icons for families the shared tool-icon map leaves generic. */
const FAMILY_ICON = {
  terminal: 'apps/terminal',
  weather: 'insights/weather-partly',
  route: 'ui/navigator',
};

/** Plugins without a window of their own: their cards have no open arrow. */
const WINDOWLESS = new Set(['traveler', 'hello', 'keyboard', 'core']);

function matchPrefix(table, action) {
  for (const [prefix, value] of table) {
    if (action.startsWith(prefix)) return value;
  }
  return null;
}

/** Bare tool name: lower-case, a model's "word.doc_write" namespacing dropped. */
function toolName(action) {
  const key = String(action || '').trim().toLowerCase();
  const dot = key.indexOf('.');
  return dot >= 0 ? key.slice(dot + 1) : key;
}

export function isHousekeepingTool(action) {
  const name = toolName(action);
  return HOUSEKEEPING.some((p) => (p.endsWith('_') ? name.startsWith(p) : name === p));
}

/** The card family of a tool (`'generic'` when no renderer knows it). */
export function toolCardFamily(action) {
  return matchPrefix(FAMILIES, toolName(action)) || 'generic';
}

function pluginFor(record) {
  const plugin = record.plugin || matchPrefix(PLUGIN_BY_PREFIX, toolName(record.action));
  return plugin && !WINDOWLESS.has(plugin) ? plugin : null;
}

/* ── small DOM helpers ──────────────────────────────────────── */

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null && text !== '') node.textContent = String(text);
  return node;
}

function str(value) {
  if (value == null) return '';
  if (Array.isArray(value)) return value.map(str).filter(Boolean).join(', ');
  if (typeof value === 'object') return '';
  return String(value).trim();
}

function num(value) {
  if (value == null || value === '') return null;
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

function list(value) {
  return Array.isArray(value) ? value : [];
}

function oneLine(text, max = 140) {
  const flat = str(text).replace(/\s+/g, ' ');
  return flat.length > max ? `${flat.slice(0, max - 1).trimEnd()}…` : flat;
}

function hostOf(url) {
  try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return str(url); }
}

function basename(path) {
  const p = str(path).replace(/\/+$/, '');
  return p.slice(p.lastIndexOf('/') + 1) || p;
}

function dirname(path) {
  const p = str(path).replace(/\/+$/, '');
  const cut = p.lastIndexOf('/');
  return cut > 0 ? p.slice(0, cut) : '';
}

function fmtNumber(n, digits = 2) {
  try { return formatNumber(n, { maximumFractionDigits: digits }); } catch { return String(n); }
}

function fmtBytes(bytes) {
  const n = num(bytes);
  if (n == null) return '';
  if (n < 1024) return `${fmtNumber(n, 0)} B`;
  if (n < 1024 * 1024) return `${fmtNumber(n / 1024, 0)} KB`;
  if (n < 1024 ** 3) return `${fmtNumber(n / 1024 / 1024, 1)} MB`;
  return `${fmtNumber(n / 1024 ** 3, 1)} GB`;
}

/** A `YYYY-MM-DD` (or ISO) date as a local Date at noon, or null. */
function parseDay(value) {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(str(value));
  if (!m) return null;
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), 12);
  return Number.isNaN(d.getTime()) ? null : d;
}

function dateText(value, options) {
  const d = parseDay(value);
  if (!d) return str(value);
  try { return new Intl.DateTimeFormat(getLocale(), options).format(d); } catch { return str(value); }
}

function shortWhen(value) {
  const s = str(value);
  if (!s) return '';
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return s;
  try {
    return new Intl.DateTimeFormat(getLocale(), { day: 'numeric', month: 'short' }).format(d);
  } catch { return s; }
}

/* ── card anatomy ───────────────────────────────────────────── */

function focusPlugin(name, extra = {}) {
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name, ...extra } }));
}

function openUrl(url) {
  import('./linkRouter.js').then((m) => m.openUrl(url)).catch(() => {});
}

/** The card shell: header with icon, mono label and the open arrow. */
function shell(record, family, labelParts) {
  const card = el('article', `tool-card tool-card--${family}`);
  card.dataset.action = toolName(record.action);
  const head = el('header', 'tool-card-head');
  const mark = toolIcon(toolName(record.action));
  const headIcon = (mark === 'ui/puzzle' && FAMILY_ICON[family]) || mark || 'ui/puzzle';
  head.append(icon(headIcon, { size: 14, className: 'tool-card-icon' }));
  const label = labelParts.map(str).filter(Boolean).join(' · ');
  head.append(el('span', 'tool-card-label', label));
  card.setAttribute('aria-label', label);

  const plugin = pluginFor(record);
  if (plugin) {
    const open = controlButton();
    open.className = 'tool-card-open';
    const hint = t('chat.toolCard.open', null, 'Open the window');
    open.title = hint;
    open.setAttribute('aria-label', hint);
    open.append(icon('ui/arrow-up', { size: 12 }));
    open.addEventListener('click', (e) => {
      e.stopPropagation();
      focusPlugin(plugin);
    });
    head.append(open);
  }
  card.append(head);
  return card;
}

function big(value, unit = '', className = '') {
  const wrap = el('div', `tool-card-big${className ? ` ${className}` : ''}`);
  const number = el('b', 'tool-card-value', value);
  // Kinetic rolls big numbers up to their value as the card arrives.
  number.dataset.roll = '';
  wrap.append(number);
  if (unit) wrap.append(el('span', 'tool-card-unit', unit));
  return wrap;
}

function cells(pairs) {
  const shown = pairs.filter(([, v]) => str(v) !== '');
  if (!shown.length) return null;
  const grid = el('div', 'tool-card-cells');
  for (const [k, v] of shown) {
    const cell = el('div', 'tool-card-cell');
    cell.append(el('span', 'tool-card-key', k), el('b', null, str(v)));
    grid.append(cell);
  }
  return grid;
}

function row({ iconName = null, title, meta = '', onClick = null, thumb = null }) {
  const r = el(onClick ? 'button' : 'div', 'tool-card-row');
  if (onClick) {
    r.type = 'button';
    r.dataset.uiControl = 'button';
    r.addEventListener('click', (e) => {
      e.stopPropagation();
      onClick();
    });
  }
  if (thumb) {
    const img = el('img', 'tool-card-thumb');
    img.src = thumb;
    img.alt = '';
    img.loading = 'lazy';
    img.decoding = 'async';
    r.append(img);
  } else if (iconName) {
    r.append(icon(iconName, { size: 14, className: 'tool-card-row-icon' }));
  }
  const main = el('span', 'tool-card-row-main');
  main.append(el('span', 'tool-card-row-title', title || '—'));
  if (meta) main.append(el('span', 'tool-card-row-meta', meta));
  r.append(main);
  return r;
}

/** Up to `max` rows, plus a "+N more" line for the rest. */
function rows(card, items, max, build) {
  const shown = items.slice(0, max);
  for (const item of shown) {
    const r = build(item);
    if (r) card.append(r);
  }
  const total = items.length;
  if (total > shown.length) {
    card.append(el('div', 'tool-card-more', tp('chat.toolCard.more', total - shown.length, null, `+${total - shown.length} more`)));
  }
}

function text(content, className = 'tool-card-text') {
  const s = str(content);
  return s ? el('p', className, s) : null;
}

function meta(content) {
  const s = str(content);
  return s ? el('div', 'tool-card-meta', s) : null;
}

function add(card, ...nodes) {
  for (const n of nodes) if (n) card.append(n);
  return card;
}

/* ── family renderers ───────────────────────────────────────── */

function radioCard(record, d, name) {
  const family = t('chat.toolCard.family.radio', null, 'Radio');
  if (name === 'radio_stop') {
    return shell(record, 'radio', [family, t('chat.toolCard.stopped', null, 'Stopped')]);
  }
  if (name === 'radio_play') {
    const card = shell(record, 'radio', [family, t('chat.toolCard.nowPlaying', null, 'Now playing')]);
    const np = el('div', 'tool-card-now');
    np.append(icon('apps/radio', { size: 22, className: 'tool-card-cover' }));
    const main = el('div', 'tool-card-now-main');
    main.append(el('b', null, str(d.station) || t('chat.toolCard.family.radio', null, 'Radio')));
    main.append(el('span', null, t('chat.toolCard.live', null, 'Live')));
    np.append(main);
    return add(card, np);
  }
  const stations = list(d.stations);
  const count = num(d.count) ?? stations.length;
  const card = shell(record, 'radio', [family, tp('chat.toolCard.stations', count, null, `${count} stations`)]);
  rows(card, stations, 4, (s) => row({
    iconName: 'apps/radio',
    title: str(s.name),
    meta: [str(s.country), oneLine(s.tags, 40), s.bitrate ? `${s.bitrate} kbps` : ''].filter(Boolean).join(' · '),
  }));
  return card;
}

function eventTime(e) {
  if (e.all_day) return t('chat.toolCard.allDay', null, 'All day');
  const start = str(e.start_time);
  const end = str(e.end_time);
  return start && end ? `${start} – ${end}` : start;
}

function calendarCard(record, d, name) {
  const family = t('chat.toolCard.family.calendar', null, 'Calendar');
  if (name === 'calendar_list' || name === 'calendar_get') {
    const events = list(d.events);
    const count = num(d.count) ?? events.length;
    const card = shell(record, 'calendar', [family, tp('chat.toolCard.events', count, null, `${count} events`)]);
    if (!events.length) return add(card, meta(t('chat.toolCard.noEvents', null, 'Nothing scheduled')));
    rows(card, events, 5, (e) => row({
      iconName: 'ui/calendar',
      title: str(e.title),
      meta: [dateText(e.date, { weekday: 'short', day: 'numeric', month: 'short' }), eventTime(e), str(e.location)]
        .filter(Boolean).join(' · '),
    }));
    return card;
  }
  const status = name === 'calendar_delete'
    ? t('chat.toolCard.eventDeleted', null, 'Event deleted')
    : name === 'calendar_update'
      ? t('chat.toolCard.eventUpdated', null, 'Event updated')
      : t('chat.toolCard.eventAdded', null, 'Event added');
  const card = shell(record, 'calendar', [family, status]);
  const ev = el('div', 'tool-card-event');
  if (parseDay(d.date)) {
    const day = el('div', 'tool-card-date');
    day.append(
      el('span', 'tool-card-key', dateText(d.date, { weekday: 'short' })),
      el('b', null, dateText(d.date, { day: 'numeric' })),
    );
    ev.append(day);
  }
  const main = el('div', 'tool-card-event-main');
  main.append(el('b', null, str(d.title) || status));
  const when = [eventTime(d), str(d.location)].filter(Boolean).join(' · ');
  if (when) main.append(el('span', null, when));
  ev.append(main);
  if (name === 'calendar_delete') card.classList.add('is-removed');
  return add(card, ev);
}

function mailCard(record, d, name, p) {
  const family = t('chat.toolCard.family.mail', null, 'Mail');
  if (name === 'mail_send') {
    const sent = d.sent !== false;
    const card = shell(record, 'mail', [family, sent
      ? t('chat.toolCard.mailSent', null, 'Sent')
      : t('chat.toolCard.mailAlreadySent', null, 'Already sent')]);
    const dl = el('dl', 'tool-card-fields');
    const field = (k, v) => {
      if (!str(v)) return;
      dl.append(el('dt', 'tool-card-key', k), el('dd', null, str(v)));
    };
    field(t('chat.toolCard.to', null, 'To'), d.to || p.to);
    field(t('chat.toolCard.subject', null, 'Subject'), p.subject);
    card.append(dl);
    return add(card, text(oneLine(p.body, 220)));
  }
  if (name === 'mail_read') {
    const card = shell(record, 'mail', [family, str(d.folder) || t('chat.toolCard.message', null, 'Message')]);
    const dl = el('dl', 'tool-card-fields');
    const from = str(d.from);
    if (from) dl.append(el('dt', 'tool-card-key', t('chat.toolCard.from', null, 'From')), el('dd', null, from));
    dl.append(el('dt', 'tool-card-key', t('chat.toolCard.subject', null, 'Subject')),
      el('dd', null, str(d.subject) || t('chat.toolCard.noSubject', null, '(no subject)')));
    card.append(dl);
    const atts = list(d.attachments);
    add(card, text(oneLine(d.text, 280)));
    for (const a of atts.slice(0, 3)) {
      card.append(row({ iconName: 'ui/file', title: str(a.filename), meta: fmtBytes(a.size) }));
    }
    return card;
  }
  if (name === 'mail_list' || name === 'mail_search') {
    const messages = list(d.messages);
    const label = name === 'mail_search'
      ? `“${oneLine(d.query, 40)}”`
      : str(d.folder) || 'INBOX';
    const card = shell(record, 'mail', [family, label]);
    if (!messages.length) return add(card, meta(t('chat.toolCard.noMessages', null, 'No messages')));
    const total = num(d.total);
    if (total != null) card.append(big(fmtNumber(total, 0), tp('chat.toolCard.messagesUnit', total, null, 'messages')));
    rows(card, messages, 4, (m) => row({
      iconName: m.seen === false ? 'ui/mail' : null,
      title: str(m.subject) || t('chat.toolCard.noSubject', null, '(no subject)'),
      meta: [oneLine(m.from, 40), shortWhen(m.date)].filter(Boolean).join(' · '),
    }));
    return card;
  }
  if (name === 'mail_status') {
    const accounts = list(d.accounts);
    const card = shell(record, 'mail', [family, d.configured
      ? t('chat.toolCard.mailReady', null, 'Ready')
      : t('chat.toolCard.mailNotSetUp', null, 'Not set up')]);
    rows(card, accounts, 3, (a) => row({ iconName: 'ui/user', title: str(a.email) || str(a.label), meta: str(a.provider) }));
    return card;
  }
  if (name === 'mail_sync') {
    return shell(record, 'mail', [family, t('chat.toolCard.mailSynced', null, 'Synced')]);
  }
  return genericCard(record, d, p);
}

function fileRow(entry) {
  const path = str(entry.path) || str(entry.name);
  const isDir = entry.kind === 'dir';
  return row({
    iconName: isDir ? 'ui/folder' : 'ui/file',
    title: str(entry.name) || basename(path),
    meta: isDir ? dirname(path) : [fmtBytes(entry.size), dirname(path)].filter(Boolean).join(' · '),
    onClick: () => focusPlugin('files', { path: isDir ? path : dirname(path) }),
  });
}

function filesCard(record, d, name) {
  const family = t('chat.toolCard.family.files', null, 'Files');
  if (name === 'file_list' || name === 'file_search') {
    const entries = list(d.entries || d.results);
    const count = num(d.count) ?? entries.length;
    const label = name === 'file_search'
      ? tp('chat.toolCard.found', count, null, `${count} found`)
      : str(d.path) || '~';
    const card = shell(record, 'files', [family, label]);
    if (!entries.length) return add(card, meta(t('chat.toolCard.empty', null, 'Nothing here')));
    rows(card, entries, 5, fileRow);
    return card;
  }
  if (name === 'file_read') {
    const card = shell(record, 'files', [family, basename(d.path)]);
    const pre = el('pre', 'tool-card-pre', str(d.content).split('\n').slice(0, 8).join('\n'));
    return add(card, pre, meta(fmtBytes(d.size)));
  }
  if (name === 'file_move' || name === 'file_copy') {
    const card = shell(record, 'files', [family, name === 'file_move'
      ? t('chat.toolCard.moved', null, 'Moved')
      : t('chat.toolCard.copied', null, 'Copied')]);
    return add(card, row({ iconName: 'ui/file', title: basename(d.to), meta: `${str(d.from)} → ${str(d.to)}` }));
  }
  if (name === 'file_delete') {
    const card = shell(record, 'files', [family, t('chat.toolCard.trashed', null, 'Moved to Trash')]);
    const gone = d.deleted || d.trashed;
    return add(card, row({ iconName: 'ui/trash', title: basename(typeof gone === 'string' ? gone : str(gone?.path || gone?.from)) }));
  }
  const status = {
    file_write: t('chat.toolCard.saved', null, 'Saved'),
    file_mkdir: t('chat.toolCard.folderCreated', null, 'Folder created'),
    file_restore: t('chat.toolCard.restored', null, 'Restored'),
  }[name] || '';
  const card = shell(record, 'files', [family, status]);
  const path = str(d.path) || str(d.restored);
  if (path) {
    card.append(row({
      iconName: name === 'file_mkdir' || d.kind === 'dir' ? 'ui/folder' : 'ui/file',
      title: basename(path),
      meta: [fmtBytes(d.bytes ?? d.size), dirname(path)].filter(Boolean).join(' · '),
      onClick: () => focusPlugin('files', { path: d.kind === 'dir' ? path : dirname(path) }),
    }));
  }
  return card;
}

function calculatorCard(record, d, name, p) {
  const family = t('chat.toolCard.family.calculator', null, 'Calculator');
  if (name !== 'calculator_eval') return genericCard(record, d, p, family);
  const card = shell(record, 'calculator', [family]);
  const expr = str(d.expression) || str(p.expression);
  const value = str(d.result_text) || (num(d.result) != null ? fmtNumber(num(d.result), 10) : str(d.result));
  return add(card, expr ? el('div', 'tool-card-expr', expr) : null, big(value || '—'));
}

function terminalCard(record, d, p) {
  const family = t('chat.toolCard.family.terminal', null, 'Terminal');
  const card = shell(record, 'terminal', [family, d.timed_out ? t('chat.toolCard.timedOut', null, 'Timed out') : '']);
  const command = str(d.command) || str(p.command);
  const output = str(d.output).replace(/\s+$/, '');
  const lines = output ? output.split('\n') : [];
  const tail = lines.length > 12 ? ['…', ...lines.slice(-12)] : lines;
  const pre = el('pre', 'tool-card-pre');
  const prompt = el('span', 'tool-card-prompt', '$');
  pre.append(prompt, document.createTextNode(` ${command}${tail.length ? `\n${tail.join('\n')}` : ''}`));
  return add(card, pre);
}

function browserCard(record, d, name, p) {
  const family = t('chat.toolCard.family.browser', null, 'Browser');
  const url = str(d.url) || str(p.url);
  if (name === 'browser_search') {
    const card = shell(record, 'browser', [family, t('chat.toolCard.search', null, 'Search')]);
    return add(card, el('b', 'tool-card-title', `“${oneLine(d.query || p.query, 80)}”`), meta(hostOf(url)));
  }
  const card = shell(record, 'browser', [family, hostOf(url)]);
  const title = name === 'browser_read' ? oneLine(str(d.text).split('\n').find((l) => l.trim()) || url, 120) : url;
  card.append(row({
    iconName: 'apps/browser',
    title,
    meta: name === 'browser_read' ? hostOf(url) : '',
    onClick: url ? () => openUrl(url) : null,
  }));
  if (name === 'browser_read') add(card, text(oneLine(str(d.text).split('\n').slice(1).join(' '), 260)));
  return card;
}

function webCard(record, d, p) {
  const results = list(d.results);
  const query = str(p.query);
  const card = shell(record, 'web', [t('chat.toolCard.family.web', null, 'Web'), query ? `“${oneLine(query, 40)}”` : '']);
  if (!results.length) return add(card, meta(t('chat.toolCard.noResults', null, 'No results')));
  rows(card, results, 3, (r) => row({ iconName: 'ui/search', title: str(r.title), meta: oneLine(r.snippet, 120) }));
  return card;
}

function videoMeta(v) {
  return [str(v.channel), str(v.duration)].filter(Boolean).join(' · ');
}

function safeImage(url) {
  const s = str(url);
  return /^(https:\/\/|\/)/.test(s) ? s : null;
}

function youtubeCard(record, d, name, p) {
  const family = t('chat.toolCard.family.youtube', null, 'YouTube');
  if (name === 'youtube_play') {
    const card = shell(record, 'youtube', [family, t('chat.toolCard.nowPlaying', null, 'Now playing')]);
    const thumb = safeImage(d.thumbnail);
    if (thumb) {
      const frame = el('div', 'tool-card-media');
      const img = el('img');
      img.src = thumb;
      img.alt = '';
      img.loading = 'lazy';
      img.decoding = 'async';
      frame.append(img, icon('ui/play', { size: 18, className: 'tool-card-media-play' }));
      card.append(frame);
    }
    return add(card, el('b', 'tool-card-title', str(d.title)), el('span', 'tool-card-sub', videoMeta(d)));
  }
  const results = list(d.results);
  const count = num(d.count) ?? results.length;
  const card = shell(record, 'youtube', [family, name === 'youtube_suggest'
    ? t('chat.toolCard.suggested', null, 'Suggested')
    : tp('chat.toolCard.videos', count, null, `${count} videos`)]);
  if (!results.length && p.query) card.append(meta(`“${oneLine(p.query, 60)}”`));
  rows(card, results, 4, (v) => row({ thumb: safeImage(v.thumbnail), iconName: 'apps/youtube', title: str(v.title), meta: videoMeta(v) }));
  return card;
}

function imageCard(record, d, name, p) {
  const family = t('chat.toolCard.family.image', null, 'Image');
  const w = num(d.width);
  const h = num(d.height);
  if (name === 'image_list') {
    const images = list(d.images);
    const count = num(d.count) ?? images.length;
    const card = shell(record, 'image', [family, tp('chat.toolCard.images', count, null, `${count} images`)]);
    rows(card, images, 4, (i) => row({
      iconName: 'ui/image',
      title: str(i.title),
      meta: num(i.width) && num(i.height) ? `${i.width} × ${i.height}` : '',
    }));
    return card;
  }
  const card = shell(record, 'image', [family, w && h ? `${w} × ${h}` : '']);
  add(card, el('b', 'tool-card-title', str(d.title) || str(p.image_id)));
  if (name === 'image_delete') card.classList.add('is-removed');
  return add(card, cells([
    [t('chat.toolCard.look', null, 'Look'), d.look],
    [t('chat.toolCard.edits', null, 'Edits'), d.operations_applied],
  ]));
}

const PROFILE_LABEL = {
  car: () => t('chat.toolCard.profile.car', null, 'Driving'),
  driving: () => t('chat.toolCard.profile.car', null, 'Driving'),
  foot: () => t('chat.toolCard.profile.foot', null, 'Walking'),
  walking: () => t('chat.toolCard.profile.foot', null, 'Walking'),
  bike: () => t('chat.toolCard.profile.bike', null, 'Cycling'),
  cycling: () => t('chat.toolCard.profile.bike', null, 'Cycling'),
};

/** Minutes as the big number: "48 min", or "1:12 h" from an hour up. */
function durationBig(minutes) {
  const m = Math.max(0, Math.round(minutes));
  if (m < 60) return big(String(m), t('chat.toolCard.unit.min', null, 'min'));
  return big(`${Math.floor(m / 60)}:${String(m % 60).padStart(2, '0')}`, t('chat.toolCard.unit.h', null, 'h'));
}

function routeCard(record, d, p) {
  const route = d.route || d.navigator || d;
  const seconds = num(route.total_duration_seconds);
  const minutes = seconds != null ? seconds / 60 : num(route.duration_min);
  const meters = num(route.total_distance_meters);
  const km = meters != null ? meters / 1000 : num(route.distance_km);
  const profile = PROFILE_LABEL[str(p.profile).toLowerCase()]?.() || '';
  const card = shell(record, 'route', [t('chat.toolCard.family.route', null, 'Route'), profile]);
  if (minutes != null) card.append(durationBig(minutes));
  const leg = el('div', 'tool-card-leg');
  leg.append(el('span', 'tool-card-dot'), el('span', 'tool-card-line'), el('span', 'tool-card-dot tool-card-dot--end'));
  card.append(leg);
  const from = str(p.from) || str(p.origin);
  const to = str(p.to) || str(p.destination) || str(route.destination?.name);
  if (from || to) {
    const names = el('div', 'tool-card-leg-names');
    names.append(el('span', null, from), el('span', null, to));
    card.append(names);
  }
  const steps = list(route.steps);
  const parts = [km != null ? `${fmtNumber(km, 1)} km` : '', steps.length ? oneLine(steps[0].instruction, 60) : ''];
  return add(card, meta(parts.filter(Boolean).join(' · ')));
}

function placesCard(record, d, name, p) {
  const family = t('chat.toolCard.family.map', null, 'Map');
  if (name === 'map_reverse') {
    const place = d.place || {};
    const card = shell(record, 'places', [family, t('chat.toolCard.address', null, 'Address')]);
    return add(card, row({ iconName: 'ui/pin', title: str(place.name) || oneLine(place.display_name, 80), meta: str(place.category) }));
  }
  const places = list(d.places);
  const query = str(p.q) || str(p.query) || str(p.category);
  const card = shell(record, 'places', [family, query ? `“${oneLine(query, 40)}”` : tp('chat.toolCard.places', places.length, null, `${places.length} places`)]);
  if (!places.length) return add(card, meta(t('chat.toolCard.noResults', null, 'No results')));
  rows(card, places, 4, (pl) => {
    const display = str(pl.display_name);
    const title = str(pl.name) || display.split(',')[0];
    return row({
      iconName: 'ui/pin',
      title,
      meta: [str(pl.place_type || pl.category), oneLine(display.split(',').slice(1).join(','), 60)].filter(Boolean).join(' · '),
    });
  });
  return card;
}

function tripCard(record, d) {
  const dest = d.destination || {};
  const card = shell(record, 'trip', [t('chat.toolCard.family.trip', null, 'Trip'), oneLine(str(dest.name).split(',')[0], 40)]);
  const route = d.route || {};
  if (num(route.duration_min) != null) card.append(durationBig(num(route.duration_min)));
  return add(card, cells([
    [t('chat.toolCard.distance', null, 'Distance'), num(route.distance_km) != null ? `${fmtNumber(num(route.distance_km), 1)} km` : ''],
    [t('chat.toolCard.guides', null, 'Guides'), d.guides_created],
  ]));
}

/** Word, Impress, PDF, Calc, Studio: one document card. */
function documentCard(record, d, name, p) {
  const prefix = name.slice(0, name.indexOf('_') + 1);
  const family = {
    doc_: t('chat.toolCard.family.document', null, 'Document'),
    slide_: t('chat.toolCard.family.slides', null, 'Slides'),
    pdf_: t('chat.toolCard.family.pdf', null, 'PDF'),
    calc_: t('chat.toolCard.family.sheet', null, 'Sheet'),
    studio_: t('chat.toolCard.family.studio', null, 'Studio'),
  }[prefix] || t('chat.toolCard.family.document', null, 'Document');
  const verb = name.slice(prefix.length);
  const status = {
    create: t('chat.toolCard.created', null, 'Created'),
    write: t('chat.toolCard.written', null, 'Written'),
    append: t('chat.toolCard.written', null, 'Written'),
    edit: t('chat.toolCard.edited', null, 'Edited'),
    update: t('chat.toolCard.edited', null, 'Edited'),
    delete: t('chat.toolCard.deleted', null, 'Deleted'),
    read: t('chat.toolCard.read', null, 'Read'),
  }[verb] || '';

  const collection = list(d.documents || d.presentations || d.spreadsheets || d.tracks || d.presets || d.arrangements);
  if (collection.length || verb === 'list') {
    const count = num(d.count) ?? collection.length;
    const card = shell(record, 'document', [family, tp('chat.toolCard.items', count, null, `${count} items`)]);
    rows(card, collection, 4, (item) => row({
      iconName: toolIcon(name) || 'ui/doc',
      title: str(item.title) || str(item.name),
      meta: [item.page_count ? tp('chat.toolCard.pages', item.page_count, null, `${item.page_count} pages`) : '', shortWhen(item.updated_at)]
        .filter(Boolean).join(' · '),
    }));
    return card;
  }
  const card = shell(record, 'document', [family, status]);
  add(card, row({ iconName: toolIcon(name) || 'ui/doc', title: str(d.title) || str(d.name) || str(p.title) || family }));
  if (verb === 'delete') card.classList.add('is-removed');
  return add(card, cells([
    [t('chat.toolCard.pagesKey', null, 'Pages'), d.page_count],
    [t('chat.toolCard.slidesKey', null, 'Slides'), name.startsWith('slide_') ? d.count : ''],
    [t('chat.toolCard.rowsKey', null, 'Rows'), d.rows],
    [t('chat.toolCard.colsKey', null, 'Columns'), d.cols],
    [t('chat.toolCard.bpm', null, 'BPM'), d.bpm],
  ]));
}

/** A weather payload wherever one turns up (`{ current: { temperature,
 *  weather_code }, daily? }` or flat `temperature`/`weather_code`). */
function weatherOf(d) {
  const cur = d?.current && typeof d.current === 'object' ? d.current : d;
  const temp = num(cur?.temperature ?? cur?.temperature_2m ?? cur?.temp);
  const code = num(cur?.weather_code ?? cur?.weathercode);
  return temp != null && code != null ? { cur, temp, code } : null;
}

function weatherCard(record, d, w) {
  const place = str(d.place || d.location || d.city);
  const card = shell(record, 'weather', [t('chat.toolCard.family.weather', null, 'Weather'), place]);
  const b = big(String(Math.round(w.temp)), '°C');
  b.append(icon(`insights/${weatherIconStem(w.code)}`, { size: 34, className: 'tool-card-big-icon' }));
  card.append(b);
  return add(card, meta(weatherLabel(w.code)), cells([
    [t('chat.toolCard.high', null, 'High'), w.cur.temperature_max != null ? tempText(w.cur.temperature_max) : ''],
    [t('chat.toolCard.low', null, 'Low'), w.cur.temperature_min != null ? tempText(w.cur.temperature_min) : ''],
    [t('chat.toolCard.wind', null, 'Wind'), num(w.cur.wind_speed ?? w.cur.windspeed) != null ? `${Math.round(num(w.cur.wind_speed ?? w.cur.windspeed))} km/h` : ''],
  ]));
}

/** Readable name of a payload key: "page_count" → "Page count". */
function humanKey(key) {
  const s = String(key).replace(/[_-]+/g, ' ').trim();
  return s.charAt(0).toUpperCase() + s.slice(1);
}

const ID_KEY = /(^id$|_id$|uuid$)/;

/** Unknown tool: its name as the label and a short key/value summary of the
 *  scalar fields (ids and nested structures left out). */
function genericCard(record, d, p, familyLabel = null) {
  const name = toolName(record.action);
  const label = familyLabel || humanKey(name.split('_')[0] || name);
  const verb = familyLabel ? humanKey(name.split('_').slice(1).join(' ')) : humanKey(name.split('_').slice(1).join(' '));
  const card = shell(record, 'generic', [label, verb]);
  const pairs = [];
  for (const [k, v] of Object.entries(d || {})) {
    if (pairs.length >= 4) break;
    if (ID_KEY.test(k) || v == null || typeof v === 'object' || str(v) === '') continue;
    pairs.push([humanKey(k), typeof v === 'boolean'
      ? (v ? t('chat.toolCard.yes', null, 'Yes') : t('chat.toolCard.no', null, 'No'))
      : oneLine(v, 60)]);
  }
  if (!pairs.length) {
    for (const [k, v] of Object.entries(d || {})) {
      if (Array.isArray(v)) {
        pairs.push([humanKey(k), fmtNumber(v.length, 0)]);
        break;
      }
    }
  }
  if (pairs.length) {
    const dl = el('dl', 'tool-card-fields');
    for (const [k, v] of pairs) dl.append(el('dt', 'tool-card-key', k), el('dd', null, v));
    card.append(dl);
  }
  return card;
}

function errorCard(record) {
  const name = toolName(record.action);
  const family = toolCardFamily(name);
  const card = shell(record, family === 'generic' ? 'generic' : family, [
    humanKey(name.replace(/_/g, ' ')),
    t('chat.toolCard.failed', null, 'Failed'),
  ]);
  card.classList.add('tool-card--error');
  const message = oneLine(record.data?.error || record.data?.message || record.data?.reason, 240);
  return add(card, text(message || t('chat.toolCard.failedBody', null, 'The tool did not finish.')));
}

/* ── entry point ────────────────────────────────────────────── */

/** Normalise a record from the stream (object) or a stored row (JSON text). */
export function parseToolRecord(raw) {
  let record = raw;
  if (typeof raw === 'string') {
    try { record = JSON.parse(raw); } catch { return null; }
  }
  if (!record || typeof record !== 'object' || !str(record.action)) return null;
  return record;
}

/**
 * The card for one tool record, or null when the tool gets none (housekeeping,
 * generated media, a malformed record).
 */
export function renderToolCard(raw) {
  const record = parseToolRecord(raw);
  if (!record) return null;
  const name = toolName(record.action);
  if (record.result && record.result !== 'ok') return errorCard(record);
  if (isHousekeepingTool(name)) return null;
  const d = record.data && typeof record.data === 'object' ? record.data : {};
  const p = record.params && typeof record.params === 'object' ? record.params : {};

  const weather = weatherOf(d);
  if (weather) return weatherCard(record, d, weather);

  switch (toolCardFamily(name)) {
    case 'radio': return radioCard(record, d, name);
    case 'calendar': return calendarCard(record, d, name);
    case 'mail': return mailCard(record, d, name, p);
    case 'files': return filesCard(record, d, name);
    case 'calculator': return calculatorCard(record, d, name, p);
    case 'terminal': return terminalCard(record, d, p);
    case 'browser': return browserCard(record, d, name, p);
    case 'web': return webCard(record, d, p);
    case 'youtube': return youtubeCard(record, d, name, p);
    case 'image': return imageCard(record, d, name, p);
    case 'route': return routeCard(record, d, p);
    case 'places': return placesCard(record, d, name, p);
    case 'trip': return tripCard(record, d);
    case 'document': return documentCard(record, d, name, p);
    default: return genericCard(record, d, p);
  }
}
