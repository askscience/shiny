/**
 * systemUpdates.js — the system and Ollama part of the Plugins window's
 * Updates section (the Updates plugin has no window of its own any more).
 *
 * The engine is the Updates plugin (`plugins/updates`, Rust): it lists the
 * pending packages with their download sizes, runs the upgrade with the
 * administrator password (asked here, used once, never stored) and reports a
 * structured progress — the phase, bytes downloaded, speed and time left,
 * every package with its state and percent, and the whole job's percent
 * (`Progress::view`). This module only shows it:
 *
 *  - an overview with a ring (the whole job's percent while one runs, the
 *    number of pending updates otherwise), the download and install meters,
 *    and one "Update all" that also updates the apps;
 *  - the System block: one hairline row per package — name, versions, size,
 *    state — whose own line fills as it downloads and installs;
 *  - the Ollama block;
 *  - the job log, opened from the overview.
 *
 * A running job survives the window: reopening it re-attaches to the job.
 * After a successful upgrade the plugins whose files changed are loaded again
 * and, when the server's own package was upgraded, the server swaps itself
 * for the new binary once the assistant is idle.
 *
 * Without the Updates plugin (removed or not answering) nothing here shows;
 * the Updates section is then the apps' alone.
 */

import {
  element as el, button, iconButton, modal, field, input, row, notify, eyebrow, icon, listReveal,
} from '../ui/index.js';
import { apiFetch } from './api.js';
import { t, tp, getLocale, loadPluginCatalogFromUrl } from './i18n.js';

const T = (key, params, fallback) => t(`plugin.updates.${key}`, params, fallback);
const JOB_POLL_MS = 600;
const MAX_LOG_LINES = 3000;

let status = null;          // /api/updates/status, or null (no plugin / not loaded)
let available = null;       // null: not asked yet; false: the plugin is not there
let job = null;             // { id, kind, progress, done, success, error }
let lastResult = null;      // 'ok' | 'error' | null — the last finished job
let logLines = [];
let logFrom = 0;
let logOpen = false;
let polling = false;
let onChange = () => {};
let catalogLoaded = false;
let revealRows = true;

/** The log's `pre`, kept across renders (lines are appended in place). */
const logPre = el('pre', 'pw-up-log');

/* ── formatting ─────────────────────────────────────────────── */

function bytes(n) {
  if (!Number.isFinite(n) || n <= 0) return '';
  const units = [['GB', 1024 ** 3], ['MB', 1024 ** 2], ['kB', 1024]];
  const nf = (v, d) => new Intl.NumberFormat(getLocale(), { maximumFractionDigits: d }).format(v);
  for (const [unit, size] of units) if (n >= size) return `${nf(n / size, n / size < 10 ? 1 : 0)} ${unit}`;
  return `${nf(n, 0)} B`;
}

function duration(secs) {
  if (!Number.isFinite(secs)) return '';
  if (secs < 60) return `${Math.max(1, Math.round(secs))} s`;
  const m = Math.floor(secs / 60);
  return m < 60 ? `${m} min ${Math.round(secs % 60)} s` : `${Math.floor(m / 60)} h ${m % 60} min`;
}

const pct = (v) => `${Math.round(Math.max(0, Math.min(100, Number(v) || 0)))}%`;

/** Fill a ring to `p` percent (its circle has pathLength 100). */
function setRing(ringBox, p) {
  const v = Math.max(0, Math.min(100, Number(p) || 0));
  const fill = ringBox?.querySelector('.pw-up-ring-fill');
  if (!fill) return;
  fill.style.strokeDasharray = `${v} 100`;
  // A round cap would leave a dot on an empty ring.
  fill.style.opacity = v > 0 ? '' : '0';
}

/* ── data ───────────────────────────────────────────────────── */

/** Load the plugin's status (force: ask the package manager again). */
export async function loadSystemUpdates(force = false) {
  try {
    if (!catalogLoaded) {
      catalogLoaded = true;
      void loadPluginCatalogFromUrl(new URL(`../plugins/updates/i18n/${getLocale()}.json`, import.meta.url).href)
        .then(() => onChange());
    }
    const res = await apiFetch(`/api/updates/status${force ? '?force=true' : ''}`, { authRedirect: false });
    status = res?.data || null;
    available = !!status;
    window.dispatchEvent(new CustomEvent('plugin:badge', {
      detail: { name: 'updates', count: status?.error ? 0 : Number(status?.count) || 0 },
    }));
  } catch (_) {
    available = false;
    status = null;
  }
  onChange();
  void reattach();
}

/** Re-render the section when the system state changes. */
export function onSystemUpdatesChange(fn) {
  onChange = typeof fn === 'function' ? fn : () => {};
}

export const systemUpdatesAvailable = () => available === true;
export const systemUpdateRunning = () => !!job && !job.done;

/** Pending system packages (0 without the plugin or on error). */
export function systemPendingCount() {
  return status && !status.error ? Number(status.count) || 0 : 0;
}

/** What changes the section's layout (not the running numbers, which are
 *  patched in place): the window re-renders when this does. */
export function systemSignature() {
  return JSON.stringify([
    available, status?.count, status?.checked_at, status?.error, status?.download_bytes,
    status?.ollama?.version, job?.id, job?.done, lastResult, logOpen, logLines.length > 0,
  ]);
}

/** The running job's whole percent, or null. */
export function systemJobPercent() {
  return systemUpdateRunning() ? Number(job.progress?.overall ?? job.progress?.percent ?? 0) : null;
}

/* ── jobs ───────────────────────────────────────────────────── */

function appendLog(lines) {
  if (!lines?.length) return;
  logLines.push(...lines);
  if (logLines.length > MAX_LOG_LINES) logLines = logLines.slice(-MAX_LOG_LINES);
  logPre.textContent = logLines.join('\n');
  logPre.scrollTop = logPre.scrollHeight;
}

function resetLog() {
  logLines = [];
  logFrom = 0;
  logPre.textContent = '';
}

async function startJob(path, body, label) {
  if (systemUpdateRunning()) {
    notify({
      app: 'Updates',
      title: T('notify.busy', null, 'An update is already running'),
      body: T('notify.busyBody', null, 'Wait for the current update to finish.'),
      urgency: 'low',
    });
    return false;
  }
  resetLog();
  lastResult = null;
  let id;
  try {
    id = (await apiFetch(path, { method: 'POST', body: JSON.stringify(body) }))?.data?.job;
  } catch (err) {
    lastResult = 'error';
    appendLog([`!! ${err.message || T('status.couldNot', { label }, 'Could not start: {label}')}`]);
    onChange();
    return false;
  }
  if (!id) {
    lastResult = 'error';
    appendLog([`!! ${T('status.noJobId', null, 'The update service did not respond as expected')}`]);
    onChange();
    return false;
  }
  job = { id, kind: path.includes('ollama') ? 'ollama' : 'system', progress: null, done: false };
  revealRows = true;
  onChange();
  await poll();
  return lastResult === 'ok';
}

/** A reopened window picks up a job still running. */
async function reattach() {
  if (systemUpdateRunning() || polling || available !== true) return;
  let data;
  try {
    data = (await apiFetch('/api/updates/jobs', { authRedirect: false }))?.data;
  } catch (_) {
    return;
  }
  if (!data?.job || data.done) return;
  resetLog();
  job = { id: data.job, kind: data.kind || 'system', progress: data.progress, done: false };
  onChange();
  await poll();
}

async function poll() {
  if (polling || !job) return;
  polling = true;
  try {
    for (;;) {
      let data;
      try {
        data = (await apiFetch(`/api/updates/jobs?job=${encodeURIComponent(job.id)}&from=${logFrom}`))?.data;
      } catch (err) {
        finish(false, err.message || T('status.lostConnection', null, 'Lost the connection to the update service'));
        return;
      }
      if (!data) {
        finish(false, T('status.jobDisappeared', null, 'The update stopped unexpectedly'));
        return;
      }
      appendLog(data.lines || []);
      logFrom = data.total ?? (logFrom + (data.lines?.length || 0));
      job.progress = data.progress || job.progress;
      if (data.done) {
        finish(data.success === true, data.error || null);
        return;
      }
      paintProgress();
      await new Promise((r) => setTimeout(r, JOB_POLL_MS));
    }
  } finally {
    polling = false;
  }
}

function finish(success, error) {
  if (job) job.done = true;
  lastResult = success ? 'ok' : 'error';
  if (error) appendLog([`!! ${error}`]);
  notify({
    app: 'Updates',
    title: success ? T('status.finished', null, 'Update finished') : T('status.failed', null, 'Update failed'),
    body: success
      ? T('status.upToDateBody', null, 'Your system is up to date.')
      : (error || T('status.incomplete', null, 'The update did not complete. Open Updates for details.')),
    icon: success ? 'ui/check' : 'ui/close',
    urgency: success ? 'low' : 'normal',
  });
  const upgraded = (job?.progress?.items || []).filter((i) => i.state === 'done').map((i) => i.name);
  onChange();
  void loadSystemUpdates(true);
  if (success && job?.kind === 'system') void followUp(upgraded);
}

/**
 * After a successful upgrade nothing has to restart for it to show: the
 * server loads the plugins whose files changed and — when its own package was
 * among the upgrades — swaps itself for the new binary once the assistant is
 * idle (the page reconnects on its own, `liveUpdate.js`). The kiosk shell
 * relaunches behind the next locked screen. Each step goes into the log.
 */
async function followUp(upgraded) {
  try {
    const report = (await apiFetch('/api/plugins/rescan', { method: 'POST', authRedirect: false }))?.data || {};
    const names = [...(report.updated || []), ...(report.added || [])];
    if (names.length) appendLog([T('live.plugins', { names: names.join(', ') }, `Loaded again without a restart: ${names.join(', ')}`)]);
    if (report.overridden?.length) {
      const list = report.overridden.join(', ');
      appendLog([T('live.overridden', { names: list }, `Not updated, your own copy is in use: ${list}`)]);
    }
    for (const [name, err] of report.failed || []) appendLog([`!! ${name}: ${err}`]);
  } catch (err) {
    appendLog([`!! ${err.message}`]);
  }
  let version = null;
  try {
    version = (await apiFetch('/api/system/version', { authRedirect: false }))?.data || null;
  } catch (_) { /* an older server: nothing to swap */ }
  if (!upgraded.some((n) => n === 'liteaura' || n === 'peakd') && !version?.binaryReplaced) return;
  try {
    await apiFetch('/api/system/reexec', { method: 'POST', authRedirect: false });
    appendLog([
      T('live.server', null, 'Server: switching to the new version as soon as the assistant is idle; the page reconnects on its own.'),
      T('live.shell', null, 'Shell: it relaunches with the new version the next time the screen is locked.'),
    ]);
  } catch (err) {
    appendLog([`!! ${err.message}`]);
  }
}

function askPassword({ title, message }) {
  return new Promise((resolve) => {
    const pw = input({ type: 'password', placeholder: T('dialog.passwordPlaceholder', null, 'Your password'), autocomplete: 'current-password' });
    let settled = false;
    const done = (value) => {
      if (settled) return;
      settled = true;
      api.close();
      resolve(value);
    };
    pw.addEventListener('keydown', (e) => { if (e.key === 'Enter') done(pw.value); });
    const cancel = button({ label: T('dialog.cancel', null, 'Cancel'), icon: 'ui/close', variant: 'quiet', onClick: () => done(null) });
    const go = button({ label: T('dialog.continue', null, 'Continue'), icon: 'ui/check', onClick: () => done(pw.value) });
    const api = modal({
      title,
      body: [
        field({
          label: T('dialog.passwordField', null, 'Administrator password'),
          control: pw,
          hint: message || T('dialog.passwordHint', null, 'Used once to run the update as administrator. It is never stored.'),
        }),
        row([cancel, go]),
      ],
    });
    api.open();
    pw.focus();
  });
}

/** Install every pending system update (asks for the password). */
export async function updateSystem() {
  const n = systemPendingCount();
  const password = await askPassword({
    title: T('dialog.installTitle', null, 'Install updates'),
    message: tp('plugin.updates.dialog.installRights', n, null, 'Installing {count} updates requires administrator rights.'),
  });
  if (password == null) return false;
  return startJob('/api/updates/apply', { password, all: true }, T('label.installing', null, 'Installing updates'));
}

async function refreshLists() {
  const password = await askPassword({
    title: T('action.refresh', null, 'Refresh package lists'),
    message: T('dialog.refreshRights', { cmd: '`apt update`' }, 'Refreshing the package lists ({cmd}) requires administrator rights.'),
  });
  if (password == null) return;
  await startJob('/api/updates/refresh', { password }, T('label.refreshing', null, 'Refreshing package lists'));
}

async function updateOllama() {
  const password = await askPassword({
    title: T('ollama.update', null, 'Update Ollama'),
    message: T('dialog.ollamaMessage', null,
      'Runs the official installer. Ollama is stopped during the update and left stopped (restarting it mid-session can hang the GPU); start it again or reboot to use the new version.'),
  });
  if (password == null) return;
  await startJob('/api/updates/ollama/update', { password }, T('label.updatingOllama', null, 'Updating Ollama'));
}

/* ── rendering ──────────────────────────────────────────────── */

const refs = { ring: null, ringValue: null, meters: null, now: null, rows: new Map(), list: null };

/** A ring for a percent (`null`: a still ring around a number). */
function ring() {
  const NS = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', '0 0 120 120');
  svg.setAttribute('class', 'pw-up-ring-svg');
  svg.setAttribute('aria-hidden', 'true');
  for (const cls of ['pw-up-ring-track', 'pw-up-ring-fill']) {
    const c = document.createElementNS(NS, 'circle');
    c.setAttribute('cx', '60');
    c.setAttribute('cy', '60');
    c.setAttribute('r', '52');
    c.setAttribute('pathLength', '100');
    c.setAttribute('class', cls);
    svg.appendChild(c);
  }
  return svg;
}

function meter(label, percent, details, cls = '') {
  const m = el('div', `pw-up-meter${cls ? ` ${cls}` : ''}`);
  m.style.setProperty('--p', String(Math.max(0, Math.min(100, Number(percent) || 0))));
  const head = el('div', 'pw-up-meter-head');
  head.append(el('span', 'pw-up-meter-label', label), el('span', 'pw-up-meter-value', pct(percent)));
  m.append(head, el('div', 'pw-up-meter-bar'));
  if (details) m.appendChild(el('div', 'pw-up-meter-detail', details));
  return m;
}

/** The download and install meters of the running job. */
function meters(p) {
  const box = el('div', 'pw-up-meters');
  if (!p) return box;
  if (p.download_total > 0) {
    const dl = p.download_total ? (100 * p.download_done) / p.download_total : 0;
    const parts = [`${bytes(p.download_done) || '0'} / ${bytes(p.download_total)}`];
    if (p.speed) parts.push(`${bytes(p.speed)}/s`);
    if (p.eta) parts.push(T('progress.left', { time: duration(p.eta) }, `${duration(p.eta)} left`));
    box.appendChild(meter(T('progress.download', null, 'Download'), dl, parts.join(' · '), 'is-download'));
  } else if (p.phase === 'download') {
    box.appendChild(meter(T('progress.download', null, 'Download'), p.percent, p.message || '', 'is-download'));
  }
  if (p.phase === 'install' || p.phase === 'step' || (p.items || []).some((i) => i.percent > 50)) {
    const items = p.items || [];
    const done = items.filter((i) => i.state === 'done' || i.state === 'removed').length;
    const detail = items.length
      ? T('progress.packages', { done, total: items.length }, `${done} of ${items.length} packages`)
      : (p.message || '');
    const installPct = p.phase === 'install' ? p.percent : 0;
    box.appendChild(meter(T('progress.install', null, 'Install'), installPct, detail, 'is-install'));
  }
  return box;
}

const STATE_LABEL = {
  queued: ['pkg.queued', 'Queued'],
  downloading: ['pkg.downloading', 'Downloading'],
  downloaded: ['pkg.downloaded', 'Downloaded'],
  unpacking: ['pkg.unpacking', 'Unpacking'],
  configuring: ['pkg.configuring', 'Configuring'],
  done: ['pkg.done', 'Installed'],
  removing: ['pkg.removing', 'Removing'],
  removed: ['pkg.removed', 'Removed'],
  failed: ['pkg.error', 'Failed'],
};

function stateText(item) {
  const [key, fallback] = STATE_LABEL[item.state] || ['pkg.queued', 'Queued'];
  const word = T(key, null, fallback);
  return item.state === 'downloading' ? `${word} ${pct(item.percent * 2)}` : word;
}

function pkgRow(name, info, item, number) {
  const r = el('div', 'pw-up-pkg');
  r.dataset.name = name;
  const num = el('span', 'pw-up-pkg-num', String(number).padStart(2, '0'));
  const text = el('div', 'pw-up-pkg-text');
  text.append(el('span', 'pw-up-pkg-name', name));
  const versions = info?.current && info?.candidate ? `${info.current} → ${info.candidate}` : (info?.candidate || '');
  if (versions) text.appendChild(el('span', 'pw-up-pkg-ver', versions));
  const size = el('span', 'pw-up-pkg-size', bytes(item?.size ?? status?.sizes?.[name]) || '');
  const state = el('span', 'pw-up-pkg-state', item ? stateText(item) : (info?.repo || ''));
  r.append(num, text, size, state, el('span', 'pw-up-pkg-bar'));
  paintRow(r, item);
  return r;
}

function paintRow(r, item) {
  const p = item ? Math.max(0, Math.min(100, item.percent)) : 0;
  r.style.setProperty('--p', String(p));
  for (const s of Object.keys(STATE_LABEL)) r.classList.toggle(`is-${s}`, item?.state === s);
  r.classList.toggle('is-live', !!item);
  const st = r.querySelector('.pw-up-pkg-state');
  if (st && item) st.textContent = stateText(item);
}

/** Bring `current` meters up to `next`'s numbers in place, so the bars
 *  glide instead of restarting from zero; a new meter replaces the set. */
function patchMeters(current, next) {
  const kinds = (box) => [...box.children].map((m) => m.className).join('|');
  if (kinds(current) !== kinds(next)) {
    current.replaceWith(next);
    refs.meters = next;
    return;
  }
  [...next.children].forEach((m, i) => {
    const old = current.children[i];
    old.style.setProperty('--p', m.style.getPropertyValue('--p'));
    for (const sel of ['.pw-up-meter-value', '.pw-up-meter-detail', '.pw-up-meter-label']) {
      const a = old.querySelector(sel);
      const b = m.querySelector(sel);
      if (a && b && a.textContent !== b.textContent) a.textContent = b.textContent;
    }
  });
}

/** Patch the running job's numbers in place (between full renders). */
function paintProgress() {
  const p = job?.progress;
  if (!p) return;
  const overall = Number(p.overall ?? p.percent ?? 0);
  if (refs.ring?.isConnected) {
    setRing(refs.ring, overall);
    refs.ringValue.textContent = pct(overall);
  }
  if (refs.meters?.isConnected) patchMeters(refs.meters, meters(p));
  if (refs.now?.isConnected) refs.now.textContent = p.current || p.message || '';
  for (const item of p.items || []) {
    const r = refs.rows.get(item.name);
    if (r?.isConnected) paintRow(r, item);
    else if (refs.list?.isConnected) {
      const added = pkgRow(item.name, null, item, refs.rows.size + 1);
      refs.rows.set(item.name, added);
      refs.list.appendChild(added);
    }
  }
  onChange('progress');
}

/**
 * The overview: the ring, the counts, the meters and the actions. `apps`
 * describes the apps' side: `{ count, busy, percent, updateAll }`.
 */
export function renderOverview(apps) {
  const sys = systemPendingCount();
  const running = systemUpdateRunning();
  const appsBusy = apps.busy;
  const total = sys + apps.count;
  const box = el('section', 'pw-up-overview');
  box.classList.toggle('is-running', running || appsBusy);
  box.classList.toggle('is-clear', !total && !running && !appsBusy);

  const ringBox = el('div', 'pw-up-ring');
  ringBox.appendChild(ring());
  const center = el('div', 'pw-up-ring-center');
  const value = el('span', 'pw-up-ring-value');
  const label = el('span', 'pw-up-ring-label');
  center.append(value, label);
  ringBox.appendChild(center);
  let percent = null;
  if (running) percent = systemJobPercent();
  else if (appsBusy) percent = apps.percent;
  if (percent != null) {
    setRing(ringBox, percent);
    value.textContent = pct(percent);
    label.textContent = T('overview.complete', null, 'done');
  } else {
    setRing(ringBox, total ? 0 : 100);
    if (total) {
      value.textContent = String(total);
      label.textContent = T('overview.pending', null, 'to update');
    } else {
      value.appendChild(icon('ui/check', { size: 34 }));
      label.textContent = T('overview.current', null, 'up to date');
    }
  }
  refs.ring = ringBox;
  refs.ringValue = value;

  const side = el('div', 'pw-up-side');
  const title = running
    ? (job.kind === 'ollama' ? T('label.updatingOllama', null, 'Updating Ollama') : T('label.installing', null, 'Installing updates'))
    : appsBusy
      ? T('overview.appsRunning', null, 'Updating apps')
      : total
        ? tp('plugin.updates.overview.ready', total, null, '{count} updates ready')
        : T('overview.upToDate', null, 'Everything is up to date');
  side.appendChild(el('h3', 'pw-up-title', title));

  const stats = el('div', 'pw-up-stats');
  const stat = (label, val, extra = '') => {
    const s = el('div', 'pw-up-stat');
    s.append(eyebrow(label, 'pw-up-stat-label'), el('span', 'pw-up-stat-value', val));
    if (extra) s.appendChild(el('span', 'pw-up-stat-extra', extra));
    return s;
  };
  if (available) {
    stats.appendChild(stat(T('section.system', null, 'System'), String(sys),
      status?.download_bytes ? T('size.toDownload', { size: bytes(status.download_bytes) }, `${bytes(status.download_bytes)} to download`) : ''));
  }
  stats.appendChild(stat(T('section.apps', null, 'Apps'), String(apps.count)));
  if (available && status?.ollama?.installed) stats.appendChild(stat('Ollama', status.ollama.version ? `v${status.ollama.version}` : '—'));
  side.appendChild(stats);

  if (running) {
    refs.meters = meters(job.progress);
    side.appendChild(refs.meters);
    refs.now = el('p', 'pw-up-now', job.progress?.current || job.progress?.message || '');
    side.appendChild(refs.now);
  } else {
    refs.meters = null;
    if (lastResult === 'error') side.appendChild(el('p', 'pw-up-now is-error', T('status.failed', null, 'Update failed')));
  }

  const actions = el('div', 'pw-up-actions');
  if (!running && total) {
    actions.appendChild(button({
      label: T('action.updateAll', null, 'Update all'),
      icon: 'ui/download',
      variant: 'primary',
      arrow: true,
      disabled: appsBusy && !sys,
      onClick: () => void (async () => {
        apps.updateAll();
        if (sys) await updateSystem();
      })(),
    }));
  }
  if (logLines.length) {
    actions.appendChild(button({
      label: logOpen ? T('action.hideDetails', null, 'Hide details') : T('action.details', null, 'Details'),
      icon: 'ui/list',
      variant: 'ghost',
      size: 'sm',
      onClick: () => { logOpen = !logOpen; onChange(); },
    }));
  }
  if (actions.childElementCount) side.appendChild(actions);

  box.append(ringBox, side);
  const wrap = el('div', 'pw-up-overview-wrap');
  wrap.appendChild(box);
  if (logOpen && logLines.length) wrap.appendChild(logPre);
  return wrap;
}

/** The System and Ollama blocks (nothing without the plugin). */
export function renderSystemBlocks() {
  const out = [];
  if (!available || !status) return out;
  const running = systemUpdateRunning();
  const p = job?.progress;
  const sysJob = running && job.kind === 'system';

  /* System */
  const block = el('section', 'pw-up-block');
  const head = el('header', 'pw-up-block-head');
  const where = [status.distro?.name, status.distro?.manager?.toUpperCase()].filter(Boolean).join(' · ');
  const titleBox = el('div', 'pw-up-block-title');
  titleBox.append(eyebrow(where || T('status.thisSystem', null, 'this system'), 'pw-up-block-kicker'),
    el('h3', 'pw-up-block-name', T('section.system', null, 'System')));
  head.appendChild(titleBox);
  const headActions = el('div', 'pw-up-block-actions');
  if (!running && status.count) {
    headActions.appendChild(button({
      label: tp('plugin.updates.action.install', status.count, null, 'Install {count} updates'),
      icon: 'ui/download', variant: 'ghost', size: 'sm', arrow: true,
      onClick: () => void updateSystem(),
    }));
  }
  headActions.appendChild(iconButton({
    label: T('action.refresh', null, 'Refresh package lists'),
    icon: 'ui/refresh', variant: 'quiet', size: 'sm', disabled: running,
    onClick: () => void refreshLists(),
  }));
  head.appendChild(headActions);
  block.appendChild(head);

  const items = new Map((sysJob || (job?.done && job.kind === 'system') ? p?.items || [] : []).map((i) => [i.name, i]));
  const names = [...status.packages.map((x) => x.name), ...[...items.keys()].filter((n) => !status.packages.some((x) => x.name === n))];
  refs.rows = new Map();
  if (status.error) {
    block.appendChild(el('p', 'pw-up-note is-error', status.error));
  } else if (!names.length) {
    block.appendChild(el('p', 'pw-up-note', T('empty.noUpdates', null, 'No system updates waiting.')));
  } else {
    const list = el('div', 'pw-up-pkgs');
    names.forEach((name, i) => {
      const r = pkgRow(name, status.packages.find((x) => x.name === name), items.get(name), i + 1);
      refs.rows.set(name, r);
      list.appendChild(r);
    });
    refs.list = list;
    block.appendChild(list);
    if (revealRows) {
      revealRows = false;
      void listReveal(list.children);
    }
  }
  out.push(block);

  /* Ollama */
  const o = status.ollama || {};
  const ob = el('section', 'pw-up-block pw-up-block--ollama');
  const ohead = el('header', 'pw-up-block-head');
  const otitle = el('div', 'pw-up-block-title');
  otitle.append(eyebrow(o.installed ? (o.version ? `v${o.version}` : T('ollama.installedShort', null, 'Installed')) : T('ollama.notInstalled', null, 'Not installed (or not found)'), 'pw-up-block-kicker'),
    el('h3', 'pw-up-block-name', 'Ollama'));
  ohead.appendChild(otitle);
  const oactions = el('div', 'pw-up-block-actions');
  oactions.appendChild(button({
    label: o.installed ? T('ollama.update', null, 'Update Ollama') : T('ollama.install', null, 'Install Ollama'),
    icon: 'ui/download', variant: 'ghost', size: 'sm', arrow: true, disabled: running,
    onClick: () => void updateOllama(),
  }));
  ohead.appendChild(oactions);
  ob.appendChild(ohead);
  if (o.installed && o.service) ob.appendChild(el('p', 'pw-up-note', T('ollama.service', { name: o.service }, 'Service: {name}')));
  if (running && job.kind === 'ollama') ob.appendChild(meters(p));
  out.push(ob);
  return out;
}

/** The system updates' history, for the Activity section. */
export async function systemHistory() {
  if (available === false) return [];
  try {
    return (await apiFetch('/api/updates/history?limit=30', { authRedirect: false }))?.data?.history || [];
  } catch (_) {
    return [];
  }
}
