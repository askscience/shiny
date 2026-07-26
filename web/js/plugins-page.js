// Plugins page controller. Standalone page — no modal, no admin role, no emojis.
//
// Every logged-in user can install/uninstall server-side (shared) and
// activate/deactivate per-user (own workspace).

import { apiFetch, getTraveler, getToken, validateSession } from './api.js';
import { renderAvatarEl } from './userProfiles.js';

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

const PLUGIN_ICON_SVG = `
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <rect x="3" y="3" width="8" height="8" rx="2"/>
    <rect x="13" y="3" width="8" height="8" rx="2"/>
    <rect x="3" y="13" width="8" height="8" rx="2"/>
    <rect x="13" y="13" width="8" height="8" rx="2" opacity="0.45"/>
  </svg>`;

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
  })[c]);
}

function niceName(plugin) {
  return plugin.name.charAt(0).toUpperCase() + plugin.name.slice(1);
}

function toast(message, type = 'info') {
  const container = document.getElementById('toast-container');
  if (!container) return;
  const el = document.createElement('div');
  el.className = `toast${type === 'error' ? ' error' : ''}`;
  el.textContent = message;
  container.appendChild(el);
  setTimeout(() => {
    el.style.opacity = '0';
    setTimeout(() => el.remove(), 300);
  }, 4000);
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
    gridEl.innerHTML = '';
    emptyEl.classList.remove('hidden');
    emptyEl.innerHTML = `<p class="plugins-empty-title">Couldn't load plugins</p><p>${escapeHtml(e.message || 'Unknown error')}</p>`;
  }
}

function renderPlugins(plugins) {
  gridEl.innerHTML = '';
  if (!plugins.length) {
    emptyEl.classList.remove('hidden');
    metaEl.textContent = '';
    return;
  }
  emptyEl.classList.add('hidden');
  metaEl.textContent = `${plugins.length} installed`;

  for (const p of plugins) {
    const card = document.createElement('article');
    card.className = `plugin-card ${p.enabled ? '' : 'inactive'}`;
    const toggleBtn = p.enabled
      ? `<button class="plugin-btn plugin-btn-outline" data-action="deactivate" data-name="${escapeHtml(p.name)}">Deactivate</button>`
      : `<button class="plugin-btn plugin-btn-primary" data-action="activate" data-name="${escapeHtml(p.name)}">Activate</button>`;
    card.innerHTML = `
      <div class="plugin-card-top">
        <div class="plugin-card-id">
          <span class="plugin-icon">${PLUGIN_ICON_SVG}</span>
          <div class="plugin-name-block">
            <h3 class="plugin-name">${escapeHtml(niceName(p))}</h3>
            <div class="plugin-meta">
              <span class="plugin-version">v${escapeHtml(p.version)}</span>
              <span>API ${p.api_level}</span>
            </div>
          </div>
        </div>
        <span class="plugin-status ${p.enabled ? 'active' : 'inactive'}">
          <span class="plugin-status-dot"></span>
          ${p.enabled ? 'Active' : 'Inactive'}
        </span>
      </div>
      <p class="plugin-description">${escapeHtml(p.description || p.summary || '')}</p>
      <div class="plugin-actions">
        ${toggleBtn}
        <button class="plugin-btn plugin-btn-danger" data-action="uninstall" data-name="${escapeHtml(p.name)}">Remove</button>
      </div>
    `;
    gridEl.appendChild(card);
  }

  gridEl.querySelectorAll('button[data-action]').forEach((btn) => {
    btn.addEventListener('click', onPluginButton);
  });
}

async function onPluginButton(e) {
  const t = e.currentTarget;
  const action = t.dataset.action;
  const name = t.dataset.name;
  if (!action || !name || busy) return;

  if (action === 'uninstall' && !confirm(`Remove plugin "${niceName({ name })}"? Its database tables will remain.`)) {
    return;
  }

  busy = true;
  const originalLabel = t.textContent;
  setButtonsDisabled(true);
  t.textContent = 'Working…';

  const endpoint = action === 'uninstall' ? '/api/plugins/uninstall'
    : action === 'activate' ? '/api/plugins/activate'
    : '/api/plugins/deactivate';

  try {
    await apiFetch(endpoint, {
      method: 'POST',
      body: JSON.stringify({ name }),
    });
    if (action === 'uninstall') {
      toast(`Removed ${name}`, 'info');
    } else if (action === 'activate') {
      toast(`Activated ${name}`, 'info');
    } else {
      toast(`Deactivated ${name}`, 'info');
    }
  } catch (err) {
    toast(err.message || 'Action failed', 'error');
  } finally {
    busy = false;
    setButtonsDisabled(false);
    t.textContent = originalLabel;
    await loadPlugins();
    await refreshActivity();
  }
}

function setButtonsDisabled(disabled) {
  gridEl.querySelectorAll('button[data-action]').forEach((b) => {
    b.disabled = disabled;
  });
}

async function refreshActivity() {
  try {
    const res = await fetch('/api/plugins/install.log', {
      headers: { Authorization: `Bearer ${getToken()}` },
    });
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
    activityEl.innerHTML = lines.map(line => {
      let cls = 'ok';
      if (/reject|failed|missing/i.test(line)) cls = 'error';
      else if (/deactivate|uninstall/i.test(line)) cls = 'warn';
      return `<div class="plugins-activity-line ${cls}">${escapeHtml(line)}</div>`;
    }).join('');
  } catch (_) {
    activityEl.innerHTML = '<div class="plugins-activity-empty">Activity unavailable.</div>';
  }
}

async function uploadArchive(file) {
  if (busy || !file) return;
  if (!/\.(zip|tar\.gz|tgz)$/i.test(file.name)) {
    toast('File must be .zip or .tar.gz', 'error');
    return;
  }
  busy = true;
  const originalTitle = dropzoneTitle.textContent;
  const originalHint = dropzoneHint.textContent;
  dropzoneTitle.innerHTML = '<span class="plugins-spinner"></span>Uploading';
  dropzoneHint.textContent = `${file.name} — installing…`;

  try {
    const form = new FormData();
    form.append('file', file);
    const res = await apiFetch('/api/plugins/install', {
      method: 'POST',
      body: form,
    });
    const installed = res?.data?.installed || 'plugin';
    toast(`Installed: ${installed}`, 'info');
  } catch (e) {
    toast(e.message || 'Install failed', 'error');
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
  if (!getToken()) {
    window.location.href = '/';
    return;
  }
  const ok = await validateSession();
  if (!ok) {
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