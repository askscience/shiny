/**
 * share.js — public share links for the Files window.
 *
 * Backed by the plugin's managed Gokapi server (`plugins/files/src/share`):
 * the server uploads through `gokapi-cli` and hands back a download URL. This
 * module owns two dialogs:
 *
 * - **Share link…** (`openShareDialog`) — the options form (expiry, download
 *   limit, password), then the resulting link with a Copy button. Uploads run
 *   as a server-side job that this dialog polls.
 * - **Shared links** (`openSharesDialog`) — the caller's live links with
 *   download counts, Copy and Revoke.
 *
 * Host setup (Gokapi bundled with the package, Tailscale Funnel or a custom
 * public URL, on/off) is in Settings → Remote → File sharing
 * (`web/js/settings/fileSharing.js`); both dialogs link there when it is off.
 */

import {
  modal, button, input, textarea, select, field, toggleRow, toast, confirmDialog, emptyState,
} from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { copyText } from '../../js/clipboard.js';
import { t } from '../../js/i18n.js';
import { openCoreWindowSection } from '../../js/tiles.js';

const tr = (key, fallback, vars = null) => t(`plugin.files.share.${key}`, vars, fallback);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const post = (url, body) => apiFetch(url, { method: 'POST', body: JSON.stringify(body || {}) });

async function getStatus() {
  const res = await apiFetch('/api/files/share/status');
  return res?.data || {};
}

function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  Object.assign(node, props);
  for (const c of [].concat(children)) if (c) node.append(c);
  return node;
}

/** The signed-in account's display name (cached by core's session check). */
function senderName() {
  try {
    const traveler = JSON.parse(localStorage.getItem('traveler') || 'null');
    return typeof traveler?.name === 'string' ? traveler.name.slice(0, 64) : '';
  } catch {
    return '';
  }
}

/** End-to-end links carry their key after `#` (src/share/e2e.rs). */
function isE2e(share) {
  return typeof share?.url === 'string' && share.url.includes('#');
}

function hint(text) {
  return el('p', { className: 'ui-hint', textContent: text });
}

function actions(...buttons) {
  return el('div', { className: 'ui-row ui-dialog-actions' }, buttons);
}

function setBusy(btn, busy) {
  btn.disabled = busy;
  btn.setAttribute('aria-busy', busy ? 'true' : 'false');
}

function copyLink(url) {
  copyText(url);
  toast(tr('copied', 'Link copied'));
}

/* ── not set up ─────────────────────────────────────────────── */

/** Host setup lives in Settings → Remote → File sharing. */
function openSharingSettings() {
  void openCoreWindowSection('settings', 'remote');
}

function renderNotEnabled(body, dialog) {
  body.textContent = '';
  body.append(
    hint(tr('notEnabled', 'Link sharing is off. Turn it on in Settings → Remote → File sharing, then try again.')),
    actions(
      button({ label: t('common.cancel'), icon: 'ui/close', variant: 'ghost', onClick: () => dialog.close() }),
      button({
        label: tr('openSettings', 'Open Settings'),
        icon: 'ui/settings',
        onClick: () => { dialog.close(); openSharingSettings(); },
      }),
    ),
  );
}

/* ── share one entry ────────────────────────────────────────── */

function renderResult(body, share, dialog) {
  body.textContent = '';
  const link = input({ value: share.url, label: tr('link', 'Link') });
  link.readOnly = true;
  link.addEventListener('focus', () => link.select());
  const meta = [];
  if (share.expires_at) meta.push(tr('expires', 'Expires {date}', { date: new Date(share.expires_at * 1000).toLocaleString() }));
  if (share.max_downloads) meta.push(tr('maxDownloads', 'Up to {n} downloads', { n: share.max_downloads }));
  if (share.has_password) meta.push(tr('passwordSet', 'Password protected'));
  if (isE2e(share)) meta.push(tr('e2eSet', 'End-to-end encrypted'));
  body.append(
    field({ label: tr('link', 'Link'), control: link }),
    meta.length ? hint(meta.join(' · ')) : null,
    actions(
      button({ label: tr('done', 'Done'), icon: 'ui/check', variant: 'ghost', onClick: () => dialog.close() }),
      button({ label: tr('copy', 'Copy link'), icon: 'ui/copy', onClick: () => copyLink(share.url) }),
    ),
  );
  link.focus({ preventScroll: true });
  link.select();
}

function renderForm(body, entry, dialog) {
  body.textContent = '';
  const expiry = select({
    options: [
      { value: '1', label: tr('expiry1', '1 day') },
      { value: '7', label: tr('expiry7', '7 days') },
      { value: '30', label: tr('expiry30', '30 days') },
      { value: '0', label: tr('expiryNever', 'Never') },
    ],
    value: '1',
  });
  const downloads = input({ type: 'number', min: 0, max: 100000, step: 1, value: '1' });
  const password = input({ type: 'password', autocomplete: 'new-password', placeholder: tr('optional', 'Optional') });
  // Shown on the recipient's download page: "<from> wants to share a file with you".
  const from = input({ value: senderName(), maxlength: 64, autocomplete: 'off' });
  const message = textarea({ rows: 2, placeholder: tr('optional', 'Optional') });
  message.maxLength = 500;
  const e2e = toggleRow({
    label: tr('e2e', 'End-to-end encrypted'),
    hint: tr('e2eHint', 'Slower download, no progress bar.'),
    checked: true,
  });
  const status = hint('');
  const create = button({ label: tr('create', 'Create link'), icon: 'ui/link' });
  const cancel = button({ label: t('common.cancel'), icon: 'ui/close', variant: 'ghost', onClick: () => dialog.close() });

  create.addEventListener('click', async () => {
    setBusy(create, true);
    status.textContent = entry.kind === 'dir'
      ? tr('zipping', 'Compressing and uploading…')
      : tr('uploading', 'Uploading…');
    try {
      const res = await post('/api/files/share', {
        path: entry.path,
        expiry_days: Number(expiry.select.value) || 0,
        max_downloads: Math.max(0, Math.floor(Number(downloads.value) || 0)),
        password: password.value || null,
        from: from.value.trim(),
        message: message.value.trim(),
        e2e: e2e.toggle.isChecked(),
      });
      const job = res?.data?.job;
      for (;;) {
        await sleep(1000);
        if (!dialog.isOpen()) return; // closed: the job keeps running server-side
        const j = (await apiFetch(`/api/files/share/job?id=${encodeURIComponent(job)}`))?.data || {};
        if (j.state === 'done') { renderResult(body, j.share, dialog); return; }
        if (j.state === 'failed') throw new Error(j.error);
      }
    } catch (e) {
      status.textContent = '';
      toast(e.message || tr('failed', 'Could not create the link'), { type: 'error' });
      setBusy(create, false);
    }
  });

  body.append(
    hint(tr('sharing', 'Anyone with the link can download “{name}”.', { name: entry.name })),
    field({ label: tr('expiry', 'Expires after'), control: expiry }),
    field({ label: tr('downloads', 'Download limit'), control: downloads, hint: tr('downloadsHint', '0 = unlimited') }),
    field({ label: tr('password', 'Password'), control: password }),
    field({ label: tr('from', 'From'), control: from }),
    field({ label: tr('message', 'Message'), control: message }),
    e2e,
    status,
    actions(cancel, create),
  );
}

/** Open the "Share link…" dialog for one Files entry ({ path, name, kind }). */
export async function openShareDialog(entry) {
  const body = el('div', { className: 'ui-stack' });
  body.style.gap = '14px';
  const dialog = modal({ title: tr('title', 'Share link'), body, removeOnClose: true });
  dialog.open();
  body.append(hint(tr('loading', 'Loading…')));
  try {
    const status = await getStatus();
    if (status.enabled) renderForm(body, entry, dialog);
    else renderNotEnabled(body, dialog);
  } catch (e) {
    toast(e.message || tr('failed', 'Could not create the link'), { type: 'error' });
    dialog.close();
  }
}

/* ── shared links list ──────────────────────────────────────── */

function shareRow(s, refresh) {
  const meta = [];
  meta.push(s.path);
  if (s.download_count != null) meta.push(tr('downloadCount', '{n} downloads', { n: s.download_count }));
  if (s.downloads_remaining != null) meta.push(tr('remaining', '{n} left', { n: s.downloads_remaining }));
  meta.push(s.expires_at
    ? tr('expires', 'Expires {date}', { date: new Date(s.expires_at * 1000).toLocaleDateString() })
    : tr('noExpiry', 'No expiry'));
  if (s.has_password) meta.push(tr('passwordSet', 'Password protected'));
  if (isE2e(s)) meta.push(tr('e2eSet', 'End-to-end encrypted'));

  const revoke = button({ label: tr('revoke', 'Revoke'), icon: 'ui/trash', variant: 'danger', size: 'sm' });
  revoke.addEventListener('click', async () => {
    if (!(await confirmDialog(tr('revokeConfirm', 'Revoke the link to “{name}”? It stops working immediately.', { name: s.name })))) return;
    setBusy(revoke, true);
    try {
      await post('/api/files/share/revoke', { id: s.id });
      toast(tr('revoked', 'Link revoked'));
      refresh();
    } catch (e) {
      toast(e.message || tr('revokeFailed', 'Could not revoke the link'), { type: 'error' });
      setBusy(revoke, false);
    }
  });
  const text = el('div', { className: 'ui-stack' }, [
    el('strong', { textContent: s.name }),
    hint(meta.join(' · ')),
  ]);
  text.style.gap = '2px';
  text.style.minWidth = '0';
  text.style.flex = '1';
  const row = el('div', { className: 'ui-row' }, [
    text,
    button({ label: tr('copy', 'Copy link'), icon: 'ui/copy', variant: 'ghost', size: 'sm', onClick: () => copyLink(s.url) }),
    revoke,
  ]);
  row.style.gap = '8px';
  row.style.alignItems = 'center';
  return row;
}

/** Open the "Shared links" dialog: the caller's links, Copy / Revoke. */
export async function openSharesDialog() {
  const body = el('div', { className: 'ui-stack' });
  body.style.gap = '12px';
  const dialog = modal({ title: tr('listTitle', 'Shared links'), body, wide: true, removeOnClose: true });
  dialog.open();

  const refresh = async () => {
    body.textContent = '';
    body.append(hint(tr('loading', 'Loading…')));
    try {
      const [status, list] = await Promise.all([getStatus(), apiFetch('/api/files/shares')]);
      const shares = list?.data?.shares || [];
      body.textContent = '';
      if (!status.enabled) {
        body.append(hint(tr('off', 'Link sharing is off. Turn it on in Settings → Remote → File sharing.')));
      } else {
        body.append(hint(tr('servedAt', 'Links are served from {url}', { url: status.public_url || '' })));
        if (!list?.data?.live) body.append(hint(tr('notRunning', 'The sharing server is not running; download counts are unavailable.')));
      }
      if (!shares.length) {
        body.append(emptyState({ icon: 'ui/link', title: tr('empty', 'No shared links') }));
      } else {
        for (const s of shares) body.append(shareRow(s, refresh));
      }
      body.append(actions(button({
        label: tr('openSettings', 'Open Settings'),
        icon: 'ui/settings',
        variant: 'ghost',
        onClick: () => { dialog.close(); openSharingSettings(); },
      })));
    } catch (e) {
      body.textContent = '';
      body.append(hint(e.message || tr('listFailed', 'Could not load shared links')));
    }
  };
  refresh();
}
