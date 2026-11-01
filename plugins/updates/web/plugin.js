import {
  toolbar, element as el, button, chip, emptyState, field, icon, iconButton,
  input, modal, notify, row, spinner, largeTitle, hairlineList, hairlineRow,
  listReveal, rollNumber, progress as progressBar,
} from '../../ui/index.js';

/**
 * plugin.js — the Updates plugin's window.
 *
 * The window's top bar is a tab strip, like the Browser's: the shared chrome
 * folds `data-window-bar` into the title row, so the tabs sit where the window
 * title normally is. Tabs: Updates · Ollama · History.
 *
 * The Updates tab is deliberately flat, in the editorial window language: a
 * large title ("4 updates" / "Up to date") under a mono eyebrow ("DEBIAN ·
 * APT"), one ink install pill, and hairline rows with the package name on the
 * left and its mono "1.2 → 1.3 · MAIN" on the right. Progress is a bar with
 * the phase and percent when the package manager reports structured progress
 * (apt's status-fd, the Ollama installer's steps and download bar), else
 * the last log line; the only spinner is the action button's. A Details
 * toggle shows the mono log. While an install runs every package row shows its
 * own state (queued · unpacking · configuring · done), updated in place. Ollama and the
 * audit trail live in their own tabs. Styles: the "Updates plugin window"
 * section of web/css/tiles.css.
 *
 * The persistent top-bar chip lives in `hud.js`; this file is only the window.
 */

import { apiFetch } from '../../js/api.js';
import { t, tp } from '../../js/i18n.js';

export const UPDATES_PLUGIN = 'updates';

const JOB_POLL_MS = 700;
/** Lines kept in the window's log (the server keeps its own, larger copy). */
const MAX_LOG_LINES = 3000;
const TABS = [
  ['updates', 'Updates', 'plugin.updates.tab.updates'],
  ['ollama', 'Ollama'],
  ['history', 'History', 'plugin.updates.tab.history'],
];

let tileEl = null;
let contentEl = null;
let checkBtn = null;
const tabChips = {};

let activeTab = 'updates';
let status = null;
let history = null;
let busy = false;
let detailsOpen = false;
let lastLine = '';
let logLines = [];
/** The job's structured progress (`{ phase, percent, message, current, packages, failed }`). */
let jobProgress = null;
/** Package name → its row, so progress updates touch rows in place. */
const pkgRows = new Map();
let pkgListEl = null;
let barEl = null;
let barLabelEl = null;
let lastResult = null; // 'ok' | 'error' | null
let logFrom = 0;
let lastLineEl = null;
let detailsPreEl = null;
/** The window's large title (`largeTitle()`), kept across renders. */
let hero = null;
let heroCount = null;
/** Cascade the next package / history list that renders (a check finished). */
let revealPackages = true;
let revealHistory = true;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/* ── data ───────────────────────────────────────────────────── */

async function loadStatus(force = false) {
  try {
    const res = await apiFetch(`/api/updates/status${force ? '?force=true' : ''}`);
    if (res?.data) {
      status = res.data;
      revealPackages = true;
    }
  } catch (err) {
    if (tileEl) {
      notify({
        app: 'Updates',
        title: t('plugin.updates.notify.checkFailed', null, 'Could not check for updates'),
        body: err.message || t('plugin.updates.notify.serviceUnavailable', null, 'The update service is unavailable.'),
        urgency: 'normal',
      });
    }
  }
  if (tileEl) render();
}

async function loadHistory() {
  try {
    const res = await apiFetch('/api/updates/history?limit=30');
    return res?.data?.history || [];
  } catch {
    return [];
  }
}

/* ── window shell ───────────────────────────────────────────── */

export function mountUpdatesTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile updates-tile';
  tileEl.dataset.plugin = UPDATES_PLUGIN;

  tileEl.appendChild(buildBar());

  contentEl = document.createElement('div');
  contentEl.className = 'updates-content';
  tileEl.appendChild(contentEl);
  hero = largeTitle({ scroller: contentEl, eyebrow: ' ' });
  heroCount = null;
  revealPackages = true;
  revealHistory = true;

  render();
  void loadStatus(false);
  void reattach();
  return tileEl;
}

function buildBar() {
  const bar = toolbar([], { className: 'updates-bar' });
  bar.dataset.windowBar = '';
  bar.setAttribute('role', 'tablist');
  bar.setAttribute('aria-label', t('plugin.updates.toolbar.sections', null, 'Sections'));

  for (const [id, label, key] of TABS) {
    const c = chip({
      label: key ? t(key, null, label) : label,
      active: activeTab === id,
      onClick: () => {
        activeTab = id;
        render();
      },
    });
    tabChips[id] = c;
    bar.appendChild(c);
  }

  const spacer = document.createElement('div');
  spacer.className = 'updates-bar-spacer';
  bar.appendChild(spacer);

  checkBtn = iconButton({
    icon: 'ui/refresh',
    label: t('plugin.updates.toolbar.check', null, 'Check for updates'),
    onClick: () => void loadStatus(true),
  });
  bar.appendChild(checkBtn);
  return bar;
}

export function unmountUpdatesTile() {
  hero?.dispose();
  hero = null;
  tileEl?.remove();
  tileEl = null;
  contentEl = null;
  checkBtn = null;
  for (const id of Object.keys(tabChips)) delete tabChips[id];
  lastLineEl = null;
  detailsPreEl = null;
  barEl = null;
  barLabelEl = null;
  pkgListEl = null;
  pkgRows.clear();
}

export function getUpdatesTileElement() {
  return tileEl;
}

/* ── rendering ──────────────────────────────────────────────── */

function render() {
  if (!contentEl) return;
  if (checkBtn) checkBtn.disabled = busy;
  for (const [id, c] of Object.entries(tabChips)) c.setActive(id === activeTab);

  contentEl.textContent = '';
  if (hero) contentEl.appendChild(hero.el);
  if (activeTab === 'ollama') renderOllama();
  else if (activeTab === 'history') renderHistory();
  else renderUpdates();
}

function errorLine() {
  return el('p', 'updates-error', status.error);
}

/** The large title: `title` over a mono `eyebrow`; a leading count rolls up when it changes. */
function setHero(title, { eyebrow = '', meta = [] } = {}) {
  if (!hero) return;
  hero.setTitle(title);
  hero.setEyebrow(eyebrow || ' ');
  hero.setMeta(meta);
  const count = title.match(/^\d+/)?.[0] ?? null;
  if (count !== heroCount) {
    heroCount = count;
    if (count !== null) rollNumber(hero.el.querySelector('.ui-large-title-text'), { duration: 700 });
  }
}

/** "DEBIAN · APT": the distro and its package manager as a mono eyebrow. */
function distroEyebrow() {
  const d = status?.distro;
  return [d?.id, d?.manager].filter(Boolean).join(' · ');
}

function heroTitle() {
  if (!status) return t('plugin.updates.hero.checking', null, 'Checking…');
  if (status.error) return t('plugin.updates.status.unavailable', null, 'Updates unavailable');
  if (!status.count) return t('plugin.updates.empty.upToDate', null, 'Up to date');
  return tp('plugin.updates.hero.count', status.count, null, '{count} updates');
}

/* ── Updates tab ────────────────────────────────────────────── */

function renderUpdates() {
  setHero(heroTitle(), { eyebrow: distroEyebrow() });
  if (!status) {
    contentEl.appendChild(spinner({ size: 18 }));
    return;
  }

  if (status.error) contentEl.appendChild(errorLine());
  if (status && !status.error && status.count > 0) contentEl.appendChild(installAction());
  if (busy || lastResult) contentEl.appendChild(progressLine());

  const packages = status?.packages || [];
  if (!packages.length) {
    // The large title already says "Up to date" / "Updates unavailable".
    if (!status.error) {
      const quiet = el('div', 'updates-actions');
      quiet.append(
        el('p', 'updates-status', t('plugin.updates.empty.noUpdates', null, 'No system updates are waiting.')),
        refreshButton({ spin: true }),
      );
      contentEl.appendChild(quiet);
    }
    return;
  }
  const rows = hairlineList('updates-list');
  pkgListEl = rows;
  pkgRows.clear();
  for (const p of packages) addPackageRow(p.name, versionLine(p));
  // apt full-upgrade can pull in packages that were not in the list (a new
  // kernel, a new dependency): show them too while the install runs.
  for (const p of showingProgress() ? jobProgress.packages : []) {
    if (!pkgRows.has(p.name)) addPackageRow(p.name, '');
  }
  applyPackageStates();
  contentEl.appendChild(rows);
  if (revealPackages) {
    revealPackages = false;
    void listReveal(rows.children);
  }
}

/** `spin`: this is the only control on screen, so it carries the busy spinner. */
function refreshButton({ spin = false } = {}) {
  return button({
    label: t('plugin.updates.action.refresh', null, 'Refresh package lists'),
    icon: 'ui/refresh',
    variant: 'quiet',
    size: 'sm',
    loading: spin && busy,
    disabled: busy || status?.distro?.supported === false,
    onClick: () => void startRefresh(),
  });
}

/** Whether package rows show per-package install states. */
function showingProgress() {
  return !!jobProgress?.packages && (busy || lastResult === 'error');
}

function addPackageRow(name, meta) {
  const lead = el('span', 'updates-pkg-state');
  const rowEl = hairlineRow({ lead, title: name, meta });
  rowEl.classList.add('updates-pkg');
  pkgRows.set(name, { row: rowEl, lead, state: undefined });
  pkgListEl?.appendChild(rowEl);
  return pkgRows.get(name);
}

const STATE_LABELS = {
  queued: ['plugin.updates.pkg.queued', 'Waiting'],
  unpacking: ['plugin.updates.pkg.unpacking', 'Unpacking'],
  configuring: ['plugin.updates.pkg.configuring', 'Configuring'],
  done: ['plugin.updates.pkg.done', 'Installed'],
  removing: ['plugin.updates.pkg.removing', 'Removing'],
  removed: ['plugin.updates.pkg.removed', 'Removed'],
  error: ['plugin.updates.pkg.error', 'Failed'],
};

/** Paint every package row's state from `jobProgress`, touching only rows that changed. */
function applyPackageStates() {
  const active = showingProgress();
  const states = new Map((active ? jobProgress.packages : []).map((p) => [p.name, p.state]));
  const failed = new Set(active ? jobProgress.failed || [] : []);
  if (active) {
    for (const name of states.keys()) {
      if (!pkgRows.has(name)) addPackageRow(name, '');
    }
  }
  for (const [name, entry] of pkgRows) {
    let state = null;
    if (active) {
      state = failed.has(name) ? 'error' : (states.get(name) || (busy ? 'queued' : null));
      // A package still mid-step when the job ended did not finish.
      if (!busy && (state === 'unpacking' || state === 'configuring')) state = 'error';
    }
    if (entry.state === state) continue;
    entry.state = state;
    const { row, lead } = entry;
    row.classList.remove('is-queued', 'is-working', 'is-done', 'is-error', 'is-removed');
    lead.textContent = '';
    lead.removeAttribute('title');
    if (!state) continue;
    if (state === 'queued') {
      row.classList.add('is-queued');
      lead.appendChild(el('span', 'updates-pkg-dot'));
    } else if (state === 'unpacking' || state === 'configuring' || state === 'removing') {
      row.classList.add('is-working');
      lead.appendChild(spinner({ size: 14 }));
    } else if (state === 'done') {
      row.classList.add('is-done');
      lead.appendChild(icon('ui/check', { size: 16 }));
    } else if (state === 'removed') {
      row.classList.add('is-removed');
      lead.appendChild(icon('ui/minus', { size: 16 }));
    } else {
      row.classList.add('is-error');
      lead.appendChild(icon('ui/close', { size: 16 }));
    }
    const [key, fallback] = STATE_LABELS[state];
    lead.title = t(key, null, fallback);
    lead.setAttribute('aria-label', lead.title);
  }
  // Keep the package being worked on in view.
  const current = busy && jobProgress?.current ? pkgRows.get(jobProgress.current) : null;
  if (current && current.row.isConnected && !current.row.dataset.seen) {
    current.row.dataset.seen = '1';
    current.row.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }
}

function versionLine(p) {
  const versions = p.current && p.candidate
    ? `${p.current} → ${p.candidate}`
    : (p.candidate || p.current || '');
  return [versions, p.repo].filter(Boolean).join(' · ');
}

function installAction() {
  const wrap = el('div', 'updates-actions');
  wrap.appendChild(button({
    label: busy
      ? t('plugin.updates.action.installing', null, 'Installing…')
      : tp('plugin.updates.action.install', status.count, null, 'Install {count} updates'),
    variant: 'primary',
    arrow: !busy,
    loading: busy,
    disabled: busy,
    onClick: () => void startInstallAll(),
  }));
  wrap.appendChild(refreshButton());
  return wrap;
}

function progressLine() {
  const wrap = el('div', 'updates-progress');
  if (lastResult === 'error' && !busy) wrap.classList.add('is-error');

  barEl = null;
  barLabelEl = null;
  if (busy) {
    // Structured progress (apt): phase · percent · counts over a bar. The bar
    // stays hidden until the first progress event arrives.
    const meter = el('div', 'updates-meter');
    barLabelEl = el('div', 'updates-meter-label');
    barEl = progressBar({ value: 0 });
    meter.append(barLabelEl, barEl);
    wrap.appendChild(meter);
    paintMeter();
  }

  // No spinner here: the action button already spins while the job runs.
  const line = el('div', 'updates-progress-line');
  lastLineEl = el('span', 'updates-progress-text', busy
    ? (lastLine || t('plugin.updates.status.working', null, 'Working…'))
    : (lastResult === 'ok'
      ? t('plugin.updates.status.finished', null, 'Update finished')
      : t('plugin.updates.status.failed', null, 'Update failed')));
  line.appendChild(lastLineEl);
  wrap.appendChild(line);

  wrap.appendChild(button({
    label: detailsOpen ? t('plugin.updates.action.hideDetails', null, 'Hide details') : t('plugin.updates.action.details', null, 'Details'),
    variant: 'quiet',
    size: 'sm',
    onClick: () => {
      detailsOpen = !detailsOpen;
      render();
    },
  }));

  if (detailsOpen) {
    detailsPreEl = el('pre', 'updates-log');
    detailsPreEl.textContent = logLines.length ? logLines.join('\n') : (lastLine || '');
    wrap.appendChild(detailsPreEl);
    requestAnimationFrame(() => {
      if (detailsPreEl) detailsPreEl.scrollTop = detailsPreEl.scrollHeight;
    });
  } else {
    detailsPreEl = null;
  }
  return wrap;
}

/** The bar's fill and its "Installing · 42% · 12 of 40 packages" label. */
function paintMeter() {
  if (!barEl || !barLabelEl) return;
  const p = jobProgress;
  const meter = barEl.parentElement;
  const step = p?.phase === 'step';
  const has = p?.phase && (step || typeof p.percent === 'number');
  meter?.classList.toggle('is-idle', !has);
  // A step without a percent (stopping Ollama, extracting, verifying): the bar
  // keeps moving so the window never looks frozen; the text line says what.
  meter?.classList.toggle('is-indeterminate', !!step);
  if (!has) {
    barLabelEl.textContent = '';
    barEl.set(0);
    return;
  }
  if (step) {
    barLabelEl.textContent = t('plugin.updates.status.working', null, 'Working…');
    return;
  }
  const pct = Math.round(p.percent);
  const phase = p.phase === 'download'
    ? t('plugin.updates.progress.download', null, 'Downloading')
    : t('plugin.updates.progress.install', null, 'Installing');
  const parts = [phase, `${pct}%`];
  const pkgs = p.packages || [];
  if (p.phase === 'install' && pkgs.length) {
    const total = Math.max(pkgRows.size, pkgs.length);
    const done = pkgs.filter((x) => x.state === 'done' || x.state === 'removed').length;
    parts.push(t('plugin.updates.progress.packages', { done, total }, '{done} of {total} packages'));
  }
  barLabelEl.textContent = parts.join(' · ');
  barEl.set(p.percent);
}

/** Apply a polled progress snapshot: bar, current step, package rows. */
function applyProgress(p) {
  if (!p) return;
  jobProgress = p;
  paintMeter();
  if (p.message && lastLineEl && busy) {
    lastLine = p.current && !p.message.includes(p.current) ? `${p.current} · ${p.message}` : p.message;
    lastLineEl.textContent = lastLine;
  }
  if (activeTab === 'updates') applyPackageStates();
}

/**
 * Append job lines without rebuilding the tree. The open log gets new text
 * nodes (never a full rewrite per line, which made long upgrades stall the
 * window) and only follows the output while scrolled to the bottom.
 */
function appendLog(lines) {
  if (!lines.length) return;
  logLines.push(...lines);
  if (logLines.length > MAX_LOG_LINES) logLines.splice(0, logLines.length - MAX_LOG_LINES);
  for (let i = lines.length - 1; i >= 0; i--) {
    if (lines[i].trim()) { lastLine = lines[i]; break; }
  }
  if (lastLineEl && !(busy && jobProgress?.message)) lastLineEl.textContent = lastLine;
  if (detailsPreEl) {
    const pinned = detailsPreEl.scrollHeight - detailsPreEl.scrollTop - detailsPreEl.clientHeight < 24;
    const prefix = detailsPreEl.textContent ? '\n' : '';
    detailsPreEl.appendChild(document.createTextNode(prefix + lines.join('\n')));
    if (pinned) detailsPreEl.scrollTop = detailsPreEl.scrollHeight;
  }
}

/* ── Ollama tab ─────────────────────────────────────────────── */

function renderOllama() {
  if (!status) {
    contentEl.appendChild(spinner({ size: 18 }));
    return;
  }
  const o = status.ollama;
  const installed = !!o?.installed;
  setHero('Ollama', { eyebrow: o?.version ? `v${o.version}` : '' });

  const info = el('div', 'updates-ollama');
  const text = el('div');
  text.appendChild(el('p', 'updates-status', installed
    ? t('plugin.updates.ollama.installed', {
      version: o.version ? t('plugin.updates.ollama.version', { version: o.version }, ' · version {version}') : '',
      path: o.path ? ` · ${o.path}` : '',
    }, 'Installed{version}{path}')
    : t('plugin.updates.ollama.notInstalled', null, 'Not installed (or not found)')));
  if (installed && o.service) {
    text.appendChild(el('p', 'updates-status', t('plugin.updates.ollama.service', { name: o.service }, 'Service: {name}')));
  }
  info.appendChild(text);
  contentEl.appendChild(info);

  contentEl.appendChild(row([
    button({
      label: busy
        ? t('plugin.updates.status.working', null, 'Working…')
        : (installed
          ? t('plugin.updates.ollama.update', null, 'Update Ollama')
          : t('plugin.updates.ollama.install', null, 'Install Ollama')),
      variant: 'primary',
      arrow: !busy,
      loading: busy,
      disabled: busy,
      onClick: () => void startOllama(),
    }),
  ]));

  if (busy || lastResult) contentEl.appendChild(progressLine());
}

/* ── History tab ────────────────────────────────────────────── */

function renderHistory() {
  setHero(t('plugin.updates.tab.history', null, 'History'), { eyebrow: distroEyebrow() });
  const wrap = el('div', 'updates-history');
  contentEl.appendChild(wrap);
  wrap.appendChild(spinner({ size: 18 }));
  void (async () => {
    const rows = history || (await loadHistory());
    history = rows;
    wrap.textContent = '';
    if (!rows.length) {
      wrap.appendChild(emptyState({
        title: t('plugin.updates.empty.noActivity', null, 'No activity yet'),
        body: t('plugin.updates.empty.noActivityBody', null, 'Updates you install will be listed here.'),
      }));
      return;
    }
    const listEl = hairlineList('updates-list');
    for (const r of rows) {
      const item = hairlineRow({
        lead: icon(r.status === 'ok' ? 'ui/check' : 'ui/close', { size: 16 }),
        title: t('plugin.updates.history.entry', { action: r.action, manager: r.manager || t('plugin.updates.history.system', null, 'system') }, '{action} · {manager}'),
        sub: r.detail || '',
        meta: r.at || '',
      });
      item.classList.toggle('is-error', r.status !== 'ok');
      listEl.appendChild(item);
    }
    wrap.appendChild(listEl);
    if (revealHistory) {
      revealHistory = false;
      void listReveal(listEl.children);
    }
  })();
}

/* ── job progress ───────────────────────────────────────────── */

async function startJob(path, body, label) {
  if (busy) {
    notify({
      app: 'Updates',
      title: t('plugin.updates.notify.busy', null, 'An update is already running'),
      body: t('plugin.updates.notify.busyBody', null, 'Wait for the current update to finish.'),
      urgency: 'low',
    });
    return;
  }
  busy = true;
  detailsOpen = false;
  lastLine = '';
  logLines = [];
  jobProgress = null;
  lastResult = null;
  logFrom = 0;
  render();

  let jobId;
  try {
    const res = await apiFetch(path, { method: 'POST', body: JSON.stringify(body) });
    jobId = res?.data?.job;
  } catch (err) {
    busy = false;
    lastResult = 'error';
    lastLine = err.message || t('plugin.updates.status.couldNot', { label }, 'Could not start: {label}');
    render();
    return;
  }
  if (!jobId) {
    busy = false;
    lastResult = 'error';
    lastLine = t('plugin.updates.status.noJobId', null, 'The update service did not respond as expected');
    render();
    return;
  }
  await pollJob(jobId);
}

/** A reopened window (or a reloaded shell) picks up an install that is still running. */
async function reattach() {
  if (busy) return;
  let data;
  try {
    data = (await apiFetch('/api/updates/jobs'))?.data;
  } catch {
    return;
  }
  if (!data?.job || data.done || busy) return;
  busy = true;
  detailsOpen = false;
  lastLine = '';
  logLines = [];
  jobProgress = null;
  lastResult = null;
  logFrom = 0;
  render();
  await pollJob(data.job);
}

async function pollJob(id) {
  for (;;) {
    let data;
    try {
      const res = await apiFetch(`/api/updates/jobs?job=${encodeURIComponent(id)}&from=${logFrom}`);
      data = res?.data;
    } catch (err) {
      finishJob(false, err.message || t('plugin.updates.status.lostConnection', null, 'Lost the connection to the update service'));
      return;
    }
    if (!data) {
      finishJob(false, t('plugin.updates.status.jobDisappeared', null, 'The update stopped unexpectedly'));
      return;
    }
    appendLog(data.lines || []);
    applyProgress(data.progress);
    logFrom = data.total ?? (logFrom + (data.lines?.length || 0));
    if (data.done) {
      finishJob(data.success === true, data.error || null);
      return;
    }
    await sleep(JOB_POLL_MS);
  }
}

function finishJob(success, error) {
  busy = false;
  lastResult = success ? 'ok' : 'error';
  if (error) appendLog([`!! ${error}`]);
  // One notification, through the centralized system — never a toast as well.
  notify({
    app: 'Updates',
    title: success ? t('plugin.updates.status.finished', null, 'Update finished') : t('plugin.updates.status.failed', null, 'Update failed'),
    body: success
      ? t('plugin.updates.status.upToDateBody', null, 'Your system is up to date.')
      : (error || t('plugin.updates.status.incomplete', null, 'The update did not complete. Open Updates for details.')),
    icon: success ? 'ui/check' : 'ui/close',
    urgency: success ? 'low' : 'normal',
  });
  history = null;
  revealHistory = true;
  render();
  void loadStatus(true);
}

/* ── password prompt ────────────────────────────────────────── */

function askPassword({ title, message }) {
  return new Promise((resolve) => {
    const pw = input({ type: 'password', placeholder: t('plugin.updates.dialog.passwordPlaceholder', null, 'Your password'), autocomplete: 'current-password' });
    const submit = () => {
      api.close();
      resolve(pw.value);
    };
    pw.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submit();
    });
    const cancel = button({ label: t('plugin.updates.dialog.cancel', null, 'Cancel'), variant: 'quiet', onClick: () => { api.close(); resolve(null); } });
    const go = button({ label: t('plugin.updates.dialog.continue', null, 'Continue'), icon: 'ui/check', onClick: submit });

    const api = modal({
      title,
      body: [
        field({
          label: t('plugin.updates.dialog.passwordField', null, 'Administrator password'),
          control: pw,
          hint: message || t('plugin.updates.dialog.passwordHint', null, 'Used once to run the update as administrator. It is never stored.'),
        }),
        row([cancel, go]),
      ],
    });
    api.open();
    pw.focus();
  });
}

/* ── actions ────────────────────────────────────────────────── */

async function startInstallAll() {
  const n = status?.count ?? 0;
  const password = await askPassword({
    title: t('plugin.updates.dialog.installTitle', null, 'Install updates'),
    message: tp('plugin.updates.dialog.installRights', n, null, 'Installing {count} updates requires administrator rights.'),
  });
  if (password == null) return;
  await startJob('/api/updates/apply', { password, all: true }, t('plugin.updates.label.installing', null, 'Installing updates'));
}

async function startRefresh() {
  const password = await askPassword({
    title: t('plugin.updates.action.refresh', null, 'Refresh package lists'),
    message: t('plugin.updates.dialog.refreshRights', { cmd: '`apt update`' }, 'Refreshing the package lists ({cmd}) requires administrator rights.'),
  });
  if (password == null) return;
  await startJob('/api/updates/refresh', { password }, t('plugin.updates.label.refreshing', null, 'Refreshing package lists'));
}

async function startOllama() {
  const password = await askPassword({
    title: t('plugin.updates.ollama.update', null, 'Update Ollama'),
    message: t('plugin.updates.dialog.ollamaMessage', null,
      'Runs the official installer. Ollama is stopped during the update and left stopped (restarting it mid-session can hang the GPU); start it again or reboot to use the new version.'),
  });
  if (password == null) return;
  await startJob('/api/updates/ollama/update', { password }, t('plugin.updates.label.updatingOllama', null, 'Updating Ollama'));
}

export default {
  name: UPDATES_PLUGIN,
  icon: 'ui/download',
  mount: mountUpdatesTile,
  unmount: unmountUpdatesTile,
  getElement: getUpdatesTileElement,
};
