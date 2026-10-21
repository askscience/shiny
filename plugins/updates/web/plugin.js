import {
  toolbar, element as el, button, chip, emptyState, field, icon, iconButton,
  input, list, listItem, modal, notify, row, spinner,
} from '../../ui/index.js';

/**
 * plugin.js — the Updates plugin's window.
 *
 * The window's top bar is a tab strip, like the Browser's: the shared chrome
 * folds `data-window-bar` into the title row, so the tabs sit where the window
 * title normally is. Tabs: Updates · Ollama · History.
 *
 * The Updates tab is deliberately flat (GNOME-Software-ish): a status line, one
 * primary install action, and a plain list with one row per package. Progress
 * is a spinner + the last line, with a Details toggle. Ollama and the audit
 * trail live in their own tabs.
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

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/* ── data ───────────────────────────────────────────────────── */

async function loadStatus(force = false) {
  try {
    const res = await apiFetch(`/api/updates/status${force ? '?force=true' : ''}`);
    if (res?.data) status = res.data;
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
  contentEl.style.cssText =
    'flex:1;min-height:0;overflow:auto;display:flex;flex-direction:column;padding:18px 20px 24px;gap:18px;';
  tileEl.appendChild(contentEl);

  render();
  void loadStatus(false);
  return tileEl;
}

function buildBar() {
  const bar = toolbar([], { className: 'updates-bar' });
  bar.dataset.windowBar = '';
  bar.style.cssText = 'display:flex;align-items:center;gap:6px;width:100%;padding:0 4px;';
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
  spacer.style.cssText = 'flex:1;min-width:0;';
  bar.appendChild(spacer);

  checkBtn = iconButton({
    icon: 'ui/search',
    label: t('plugin.updates.toolbar.check', null, 'Check for updates'),
    onClick: () => void loadStatus(true),
  });
  bar.appendChild(checkBtn);
  return bar;
}

export function unmountUpdatesTile() {
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
  if (activeTab === 'ollama') renderOllama();
  else if (activeTab === 'history') renderHistory();
  else renderUpdates();
}

function statusLine() {
  if (!status) return t('plugin.updates.status.checking', null, 'Checking for updates…');
  const where = status.distro?.name || t('plugin.updates.status.thisSystem', null, 'this system');
  if (status.error) return t('plugin.updates.status.unavailableOnSystem', null, 'Updates are unavailable on this system');
  if (status.count === 0) return t('plugin.updates.status.upToDate', { where }, 'Everything is up to date · {where}');
  return tp('plugin.updates.status.available', status.count, { where }, '{count} updates available · {where}');
}

function errorLine() {
  const p = el('p', 'ui-subtitle', status.error);
  p.style.color = 'var(--warn)';
  return p;
}

/* ── Updates tab ────────────────────────────────────────────── */

function renderUpdates() {
  contentEl.appendChild(el('p', 'ui-subtitle', statusLine()));
  if (!status) {
    contentEl.appendChild(spinner({ size: 18 }));
    return;
  }

  if (status.error) contentEl.appendChild(errorLine());
  if (status && !status.error && status.count > 0) contentEl.appendChild(installAction());
  if (busy || lastResult) contentEl.appendChild(progressLine());

  const manager = status?.distro?.manager_label;
  if (manager) contentEl.appendChild(el('p', 'ui-subtitle', manager));

  const packages = status?.packages || [];
  if (!packages.length) {
    contentEl.appendChild(emptyState({
      icon: status?.error ? 'ui/close' : 'ui/check',
      title: status?.error
        ? t('plugin.updates.status.unavailable', null, 'Updates unavailable')
        : t('plugin.updates.empty.upToDate', null, 'Up to date'),
      body: status?.error ? undefined : t('plugin.updates.empty.noUpdates', null, 'No system updates are waiting.'),
      action: status?.error ? null : refreshButton(),
    }));
    return;
  }
  contentEl.appendChild(list(packages, (p) => listItem({
    leading: icon('ui/archive', { size: 26 }),
    title: p.name,
    subtitle: versionLine(p),
  })));
}

function refreshButton() {
  return button({
    label: t('plugin.updates.action.refresh', null, 'Refresh lists'),
    icon: 'ui/download',
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
  const wrap = el('div');
  wrap.style.cssText = 'display:flex;gap:8px;align-items:center;';
  wrap.appendChild(button({
    label: busy
      ? t('plugin.updates.action.installing', null, 'Installing…')
      : tp('plugin.updates.action.install', status.count, null, 'Install {count} Updates'),
    icon: 'ui/download',
    variant: 'primary',
    loading: busy,
    disabled: busy,
    onClick: () => void startInstallAll(),
  }));
  wrap.appendChild(refreshButton());
  return wrap;
}

function progressLine() {
  const wrap = el('div');
  wrap.style.cssText = 'display:flex;flex-direction:column;gap:8px;';

  const line = el('div');
  line.style.cssText = 'display:flex;align-items:center;gap:10px;';
  if (busy) line.appendChild(spinner({ size: 16 }));
  lastLineEl = el('span', 'ui-subtitle', busy
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
    detailsPreEl = el('pre');
    detailsPreEl.style.cssText =
      'margin:0;white-space:pre-wrap;word-break:break-word;font-size:12px;line-height:1.45;' +
      'max-height:220px;overflow:auto;color:var(--muted);';
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

  const info = el('div');
  info.style.cssText = 'display:flex;align-items:center;gap:14px;';
  info.appendChild(icon('ui/download', { size: 34 }));
  const text = el('div');
  text.appendChild(el('div', 'ui-title', 'Ollama'));
  text.appendChild(el('p', 'ui-subtitle', installed
    ? t('plugin.updates.ollama.installed', {
      version: o.version ? t('plugin.updates.ollama.version', { version: o.version }, ' · version {version}') : '',
      path: o.path ? ` · ${o.path}` : '',
    }, 'Installed{version}{path}')
    : t('plugin.updates.ollama.notInstalled', null, 'Not installed (or not found on PATH)')));
  if (installed && o.service) {
    text.appendChild(el('p', 'ui-subtitle', t('plugin.updates.ollama.service', { name: o.service }, 'Service: {name}')));
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
      icon: 'ui/download',
      variant: 'primary',
      loading: busy,
      disabled: busy,
      onClick: () => void startOllama(),
    }),
  ]));

  if (logFrom || lastResult) contentEl.appendChild(progressLine());
}

/* ── History tab ────────────────────────────────────────────── */

function renderHistory() {
  const wrap = el('div');
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
    wrap.appendChild(list(rows, (r) => listItem({
      leading: r.status === 'ok' ? 'ui/check' : 'ui/close',
      title: t('plugin.updates.history.entry', { action: r.action, manager: r.manager || t('plugin.updates.history.system', null, 'system') }, '{action} · {manager}'),
      subtitle: [r.detail, r.at].filter(Boolean).join(' · '),
    })));
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
