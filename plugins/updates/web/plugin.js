import {
  toolbar, element as el, button, chip, emptyState, field, icon, iconButton,
  input, modal, notify, row, spinner, largeTitle, hairlineList, hairlineRow,
  listReveal, rollNumber,
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
 * left and its mono "1.2 → 1.3 · MAIN" on the right. Progress is a spinner +
 * the last line, with a Details toggle that shows the mono log. Ollama and the
 * audit trail live in their own tabs. Styles: the "Updates plugin window"
 * section of web/css/tiles.css.
 *
 * The persistent top-bar chip lives in `hud.js`; this file is only the window.
 */

import { apiFetch } from '../../js/api.js';
import { t, tp } from '../../js/i18n.js';

export const UPDATES_PLUGIN = 'updates';

const JOB_POLL_MS = 700;
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
let fullLog = '';
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
        refreshButton(),
      );
      contentEl.appendChild(quiet);
    }
    return;
  }
  const rows = hairlineList('updates-list');
  for (const p of packages) rows.appendChild(hairlineRow({ title: p.name, meta: versionLine(p) }));
  contentEl.appendChild(rows);
  if (revealPackages) {
    revealPackages = false;
    void listReveal(rows.children);
  }
}

function refreshButton() {
  return button({
    label: t('plugin.updates.action.refresh', null, 'Refresh lists'),
    icon: 'ui/refresh',
    variant: 'quiet',
    size: 'sm',
    disabled: busy || status?.distro?.supported === false,
    onClick: () => void startRefresh(),
  });
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
      : tp('plugin.updates.action.install', status.count, null, 'Install {count} Updates'),
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

  const line = el('div', 'updates-progress-line');
  if (busy) line.appendChild(spinner({ size: 16 }));
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
    detailsPreEl.textContent = fullLog || lastLine || '';
    wrap.appendChild(detailsPreEl);
  } else {
    detailsPreEl = null;
  }
  return wrap;
}

/** Append a job line without rebuilding the tree. */
function appendLog(line) {
  fullLog += (fullLog ? '\n' : '') + line;
  if (line.trim()) lastLine = line;
  if (lastLineEl) lastLineEl.textContent = lastLine;
  if (detailsPreEl) {
    detailsPreEl.textContent = fullLog;
    detailsPreEl.scrollTop = detailsPreEl.scrollHeight;
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
    : t('plugin.updates.ollama.notInstalled', null, 'Not installed (or not found on PATH)')));
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

  if (logFrom || lastResult) contentEl.appendChild(progressLine());
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
  fullLog = '';
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
    lastLine = err.message || t('plugin.updates.status.couldNot', { label: label.toLowerCase() }, 'Could not {label}');
    render();
    return;
  }
  if (!jobId) {
    busy = false;
    lastResult = 'error';
    lastLine = t('plugin.updates.status.noJobId', null, 'The server did not return a job id');
    render();
    return;
  }
  await pollJob(jobId);
}

async function pollJob(id) {
  for (;;) {
    let data;
    try {
      const res = await apiFetch(`/api/updates/jobs?job=${encodeURIComponent(id)}&from=${logFrom}`);
      data = res?.data;
    } catch (err) {
      finishJob(false, err.message || t('plugin.updates.status.lostConnection', null, 'Lost the job connection'));
      return;
    }
    if (!data) {
      finishJob(false, t('plugin.updates.status.jobDisappeared', null, 'The job disappeared'));
      return;
    }
    for (const line of data.lines || []) appendLog(line);
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
  if (error) appendLog(`!! ${error}`);
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
    const pw = input({ type: 'password', placeholder: t('plugin.updates.dialog.passwordPlaceholder', null, 'sudo password'), autocomplete: 'current-password' });
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
          hint: message || t('plugin.updates.dialog.passwordHint', null, 'Used once to run the command with sudo. It is never stored.'),
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
    title: t('plugin.updates.action.refresh', null, 'Refresh lists'),
    message: t('plugin.updates.dialog.refreshRights', { cmd: '`apt update`' }, 'Refreshing package metadata (e.g. {cmd}) requires administrator rights.'),
  });
  if (password == null) return;
  await startJob('/api/updates/refresh', { password }, t('plugin.updates.label.refreshing', null, 'Refreshing lists'));
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
