// Plugins page controller — built on the Shiny UI library.
// Every logged-in user can install/uninstall server-side (shared) and
// activate/deactivate per-user (own workspace).

import { apiFetch, getTraveler, getToken, validateSession } from './api.js';
import { renderAvatarEl } from './userProfiles.js';
import {
  initThemeLoader, initAppearance, hydrateIcons,
  toast, button, icon, emptyState, spinner,
} from '../ui/index.js';

const dropzone = document.getElementById('plugins-dropzone');
const dropzoneTitle = document.getElementById('plugins-dropzone-title');
const dropzoneHint = document.getElementById('plugins-dropzone-hint');
const fileInput = document.getElementById('plugins-file-input');

const gridEl = document.getElementById('plugins-grid');
const emptyEl = document.getElementById('plugins-empty');
const activityEl = document.getElementById('plugins-activity');
const metaEl = document.getElementById('plugins-installed-meta');
const userAvatarEl = document.getElementById('plugins-user-avatar');
const userNameEl = document.getElementById('plugins-user-name');

let busy = false;

function niceName(plugin) {
  return plugin.name.charAt(0).toUpperCase() + plugin.name.slice(1);
}

/** Cross-page signal: the sphere re-evaluates active plugins when it regains focus. */
function notifyPluginsChanged() {
  localStorage.setItem('plugins.changed', String(Date.now()));
  window.dispatchEvent(new CustomEvent('plugins:changed'));
}

function renderUser() {
  const u = getTraveler();
  const name = u?.name || u?.username || 'Account';
  userNameEl.textContent = name;
  renderAvatarEl(userAvatarEl, { name, avatar: u?.avatar || null });
}

async function loadPlugins() {
  try {
    const res = await apiFetch('/api/plugins');
    const plugins = (res?.data) || [];
    renderPlugins(plugins);
  } catch (e) {
    gridEl.textContent = '';
    emptyEl.classList.remove('hidden');
    emptyEl.textContent = '';
    emptyEl.appendChild(emptyState({
      icon: 'ui/warning',
      title: "Couldn't load plugins",
      body: e.message || 'Unknown error',
    }));
  }
}

function pluginCard(p, idx) {
  const el = document.createElement('article');
  el.className = 'plugin-row';
  el.classList.toggle('is-inactive', !p.enabled);

  // Index number — editorial touch.
  const num = document.createElement('span');
  num.className = 'plugin-row-num';
  num.textContent = String(idx + 1).padStart(2, '0');

  const id = document.createElement('div');
  id.className = 'plugin-row-id';
  const iconWrap = document.createElement('span');
  iconWrap.className = 'plugin-row-icon';
  iconWrap.appendChild(icon('ui/puzzle', { size: 18 }));
  const nameBlock = document.createElement('div');
  nameBlock.className = 'plugin-row-name-block';
  const h3 = document.createElement('h3');
  h3.className = 'plugin-row-name';
  h3.textContent = niceName(p);
  const desc = document.createElement('p');
  desc.className = 'plugin-row-desc';
  desc.textContent = p.description || p.summary || '';
  nameBlock.append(h3, desc);
  id.append(iconWrap, nameBlock);

  const meta = document.createElement('div');
  meta.className = 'plugin-row-meta';
  const ver = document.createElement('span');
  ver.textContent = `v${p.version}`;
  const api = document.createElement('span');
  api.textContent = `API ${p.api_level}`;
  meta.append(ver, api);

  const status = document.createElement('div');
  status.className = 'plugin-row-status';
  const dot = document.createElement('span');
  dot.className = `plugin-dot ${p.enabled ? 'plugin-dot--on' : ''}`;
  status.append(dot, document.createTextNode(p.enabled ? 'Active' : 'Off'));

  const actions = document.createElement('div');
  actions.className = 'plugin-row-actions';
  const toggleBtn = button({
    label: p.enabled ? 'Deactivate' : 'Activate',
    variant: 'ghost', size: 'sm',
    onClick: () => onPluginAction(p.enabled ? 'deactivate' : 'activate', p.name),
  });
  toggleBtn.dataset.action = p.enabled ? 'deactivate' : 'activate';
  const removeBtn = button({
    label: 'Remove', variant: 'ghost', size: 'sm',
    onClick: () => onPluginAction('uninstall', p.name),
  });
  removeBtn.classList.add('plugin-remove');
  removeBtn.dataset.action = 'uninstall';
  actions.append(toggleBtn, removeBtn);

  el.append(num, id, meta, status, actions);
  return el;
}

function renderPlugins(plugins) {
  gridEl.textContent = '';
  if (!plugins.length) {
    emptyEl.classList.remove('hidden');
    emptyEl.textContent = '';
    emptyEl.appendChild(emptyState({
      icon: 'ui/puzzle',
      title: 'No plugins installed',
      body: 'Shiny is running in its bare form. Install a plugin above to add tools, routes, and skills.',
    }));
    metaEl.textContent = '';
    return;
  }
  emptyEl.classList.add('hidden');
  metaEl.textContent = `${plugins.length} installed`;
  plugins.forEach((p, i) => gridEl.appendChild(pluginCard(p, i)));
}

async function onPluginAction(action, name) {
  if (busy) return;
  if (action === 'uninstall' && !confirm(`Remove plugin "${name}"? Its database tables will remain.`)) {
    return;
  }

  busy = true;
  setButtonsDisabled(true);

  const endpoint = action === 'uninstall' ? '/api/plugins/uninstall'
    : action === 'activate' ? '/api/plugins/activate'
    : '/api/plugins/deactivate';

  try {
    await apiFetch(endpoint, {
      method: 'POST',
      body: JSON.stringify({ name }),
    });
    if (action === 'uninstall') toast(`Removed ${name}`);
    else if (action === 'activate') toast(`Activated ${name}`);
    else toast(`Deactivated ${name}`);
    notifyPluginsChanged();
  } catch (err) {
    toast(err.message || 'Action failed', { type: 'error' });
  } finally {
    busy = false;
    setButtonsDisabled(false);
    await loadPlugins();
    await refreshActivity();
  }
}

function setButtonsDisabled(disabled) {
  gridEl.querySelectorAll('button').forEach((b) => { b.disabled = disabled; });
}

async function refreshActivity() {
  try {
    // Only attach the Bearer header when we actually have a token; otherwise
    // rely on the session cookie (never send "Bearer null").
    const headers = {};
    const token = getToken();
    if (token) headers.Authorization = `Bearer ${token}`;
    const res = await fetch('/api/plugins/install.log', { headers });
    if (!res.ok) {
      activityEl.innerHTML = '<div class="plugins-activity-empty">Activity unavailable.</div>';
      return;
    }
    const text = (await res.text()).trim();
    if (!text) {
      activityEl.innerHTML = '<div class="plugins-activity-empty">No install events yet.</div>';
      return;
    }
    const lines = text.split('\n').slice(-60);
    activityEl.textContent = '';
    for (const line of lines) {
      const div = document.createElement('div');
      let cls = 'ok';
      if (/reject|failed|missing/i.test(line)) cls = 'error';
      else if (/deactivate|uninstall/i.test(line)) cls = 'warn';
      div.className = `plugins-activity-line ${cls}`;
      div.textContent = line;
      activityEl.appendChild(div);
    }
  } catch (_) {
    activityEl.innerHTML = '<div class="plugins-activity-empty">Activity unavailable.</div>';
  }
}

async function uploadArchive(file) {
  if (busy || !file) return;
  if (!/\.(zip|tar\.gz|tgz)$/i.test(file.name)) {
    toast('File must be .zip or .tar.gz', { type: 'error' });
    return;
  }
  busy = true;
  const originalTitle = dropzoneTitle.textContent;
  const originalHint = dropzoneHint.textContent;
  dropzoneTitle.textContent = '';
  dropzoneTitle.append(spinner({ size: 15 }), document.createTextNode('Uploading'));
  dropzoneHint.textContent = `${file.name} — installing…`;

  try {
    const form = new FormData();
    form.append('file', file);
    const res = await apiFetch('/api/plugins/install', {
      method: 'POST',
      body: form,
    });
    const installed = res?.data?.installed || 'plugin';
    toast(`Installed: ${installed}`);
    notifyPluginsChanged();
  } catch (e) {
    toast(e.message || 'Install failed', { type: 'error' });
  } finally {
    busy = false;
    dropzoneTitle.textContent = originalTitle;
    dropzoneHint.textContent = originalHint;
    fileInput.value = '';
    await loadPlugins();
    await refreshActivity();
  }
}

async function boot() {
  await initThemeLoader();
  initAppearance({ getScope: () => getTraveler()?.id });
  hydrateIcons();

  if (!(await validateSession())) {
    window.location.href = '/';
    return;
  }

  renderUser();
  await loadPlugins();
  await refreshActivity();

  fileInput?.addEventListener('change', () => {
    if (fileInput.files?.[0]) uploadArchive(fileInput.files[0]);
  });
  ['dragenter', 'dragover'].forEach((evt) => {
    dropzone.addEventListener(evt, (e) => {
      e.preventDefault();
      e.stopPropagation();
      dropzone.classList.add('dragging');
    });
  });
  ['dragleave', 'drop'].forEach((evt) => {
    dropzone.addEventListener(evt, (e) => {
      e.preventDefault();
      e.stopPropagation();
      dropzone.classList.remove('dragging');
    });
  });
  dropzone.addEventListener('drop', (e) => {
    const f = e.dataTransfer?.files?.[0];
    if (f) uploadArchive(f);
  });
}

boot();
