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

import {
  button, chip, emptyState, field, icon, iconButton, input, list, listItem,
  modal, notify, row, spinner,
} from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';

export const UPDATES_PLUGIN = 'updates';

const JOB_POLL_MS = 700;
const TABS = [
  ['updates', 'Updates'],
  ['ollama', 'Ollama'],
  ['history', 'History'],
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
        title: 'Could not check for updates',
        body: err.message || 'The update service is unavailable.',
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
  const bar = document.createElement('div');
  bar.className = 'updates-bar';
  bar.dataset.windowBar = '';
  bar.style.cssText = 'display:flex;align-items:center;gap:6px;width:100%;padding:0 4px;';
  bar.setAttribute('role', 'tablist');
  bar.setAttribute('aria-label', 'Sections');

  for (const [id, label] of TABS) {
    const c = chip({
      label,
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
    label: 'Check for updates',
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

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

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
  if (!status) return 'Checking for updates…';
  const where = status.distro?.name || 'this system';
  if (status.error) return 'Updates are unavailable on this system';
  if (status.count === 0) return `Everything is up to date · ${where}`;
  return `${status.count} update${status.count === 1 ? '' : 's'} available · ${where}`;
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
      title: status?.error ? 'Updates unavailable' : 'Up to date',
      body: status?.error ? undefined : 'No system updates are waiting.',
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
    label: 'Refresh lists',
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
    label: busy ? 'Installing…' : `Install ${status.count} Update${status.count === 1 ? '' : 's'}`,
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
    ? (lastLine || 'Working…')
    : (lastResult === 'ok' ? 'Update finished' : 'Update failed'));
  line.appendChild(lastLineEl);
  wrap.appendChild(line);

  wrap.appendChild(button({
    label: detailsOpen ? 'Hide details' : 'Details',
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
    ? `Installed${o.version ? ` · version ${o.version}` : ''}${o.path ? ` · ${o.path}` : ''}`
    : 'Not installed (or not found on PATH)'));
  if (installed && o.service) text.appendChild(el('p', 'ui-subtitle', `Service: ${o.service}`));
  info.appendChild(text);
  contentEl.appendChild(info);

  contentEl.appendChild(row([
    button({
      label: busy ? 'Working…' : (installed ? 'Update Ollama' : 'Install Ollama'),
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
        title: 'No activity yet',
        body: 'Updates you install will be listed here.',
      }));
      return;
    }
    wrap.appendChild(list(rows, (r) => listItem({
      leading: r.status === 'ok' ? 'ui/check' : 'ui/close',
      title: `${r.action} · ${r.manager || 'system'}`,
      subtitle: [r.detail, r.at].filter(Boolean).join(' · '),
    })));
  })();
}

/* ── job progress ───────────────────────────────────────────── */

async function startJob(path, body, label) {
  if (busy) {
    notify({
      app: 'Updates',
      title: 'An update is already running',
      body: 'Wait for the current update to finish.',
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
    lastLine = err.message || `Could not ${label.toLowerCase()}`;
    render();
    return;
  }
  if (!jobId) {
    busy = false;
    lastResult = 'error';
    lastLine = 'The server did not return a job id';
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
      finishJob(false, err.message || 'Lost the job connection');
      return;
    }
    if (!data) {
      finishJob(false, 'The job disappeared');
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
    title: success ? 'Update finished' : 'Update failed',
    body: success
      ? 'Your system is up to date.'
      : (error || 'The update did not complete. Open Updates for details.'),
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
    const pw = input({ type: 'password', placeholder: 'sudo password', autocomplete: 'current-password' });
    const submit = () => {
      api.close();
      resolve(pw.value);
    };
    pw.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submit();
    });
    const cancel = button({ label: 'Cancel', variant: 'quiet', onClick: () => { api.close(); resolve(null); } });
    const go = button({ label: 'Continue', icon: 'ui/check', onClick: submit });

    const api = modal({
      title,
      body: [
        field({
          label: 'Administrator password',
          control: pw,
          hint: message || 'Used once to run the command with sudo. It is never stored.',
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
    title: 'Install updates',
    message: `Installing ${n} update${n === 1 ? '' : 's'} requires administrator rights.`,
  });
  if (password == null) return;
  await startJob('/api/updates/apply', { password, all: true }, 'Installing updates');
}

async function startRefresh() {
  const password = await askPassword({
    title: 'Refresh lists',
    message: 'Refreshing package metadata (e.g. `apt update`) requires administrator rights.',
  });
  if (password == null) return;
  await startJob('/api/updates/refresh', { password }, 'Refreshing lists');
}

async function startOllama() {
  const password = await askPassword({
    title: 'Update Ollama',
    message: 'Runs the official installer. Ollama is stopped during the update and left stopped (restarting it mid-session can hang the GPU); start it again or reboot to use the new version.',
  });
  if (password == null) return;
  await startJob('/api/updates/ollama/update', { password }, 'Updating Ollama');
}

export default {
  name: UPDATES_PLUGIN,
  icon: 'ui/download',
  mount: mountUpdatesTile,
  unmount: unmountUpdatesTile,
  getElement: getUpdatesTileElement,
};
