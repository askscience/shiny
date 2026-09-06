/**
 * mail.js — the Mail plugin's window (IMAP/SMTP mail client).
 *
 * A three-pane mail client backed by io-email routes (`/api/mail/*`):
 * folders, message list and a reader, plus a compose modal and an account
 * settings panel. While no mail provider works yet, an onboarding card shows
 * per-provider help with links; once an account's Test connection succeeds
 * the card disappears and the mailbox appears.
 *
 * AI wiring: `mail_*` tool outcomes arrive via `agent:actions` — the window
 * refreshes its account/status state.
 */

import {
  button, emptyState, field, icon, input, modal, select, textarea, toast,
} from '/ui/index.js';
import { setIcon } from '/ui/index.js';
import { apiFetch } from '/js/api.js';

export const MAIL_PLUGIN = 'mail';

const PROVIDER_HELP = {
  gmail: {
    text: 'Gmail no longer accepts your normal password from third-party apps. Turn on IMAP, enable 2-Step Verification, then create an App Password and paste it in the password field. (New Google Workspace setups may require OAuth2 instead — this first version uses app passwords.)',
    links: [
      { label: 'Gmail IMAP/SMTP settings', href: 'https://support.google.com/mail/answer/7126229' },
      { label: 'Create an app password', href: 'https://myaccount.google.com/apppasswords' },
    ],
  },
  outlook: {
    text: 'Personal Outlook.com accounts use an app password (turn on two-step verification first, then generate it in your Microsoft account security). Work/school Microsoft 365 accounts now require OAuth2 (Modern Auth), which this first version does not support yet — use a personal account for now.',
    links: [
      { label: 'Outlook POP/IMAP/SMTP settings', href: 'https://support.microsoft.com/en-us/office/pop-imap-and-smtp-settings-for-outlook-com-d088b986-291d-42b8-9564-9c414e2aa040' },
    ],
  },
  yahoo: {
    text: 'Yahoo requires an app password. Enable two-step verification (or Account Key), then generate an app password in Account security and use it as the password here.',
    links: [
      { label: 'Yahoo app passwords', href: 'https://help.yahoo.com/kb/SLN4075.html' },
    ],
  },
  icloud: {
    text: 'iCloud requires an app-specific password — your Apple ID password won’t work. Create one at appleid.apple.com (Sign-In & Security → App-Specific Passwords) and make sure iCloud Mail is enabled on your device.',
    links: [
      { label: 'iCloud app-specific passwords', href: 'https://support.apple.com/en-us/102525' },
    ],
  },
  zoho: {
    text: 'Zoho Mail: enable IMAP in Settings → Mail Accounts → IMAP Access, then sign in with your full email and your Zoho password — or an app password if two-factor authentication is on.',
    links: [
      { label: 'Zoho Mail app settings', href: 'https://www.zoho.com/mail/help/zoho-mail-apps.html' },
    ],
  },
  fastmail: {
    text: 'Fastmail works with an app password (Settings → Password & Security → App Passwords) rather than your login password.',
    links: [
      { label: 'Fastmail SMTP/IMAP settings', href: 'https://www.fastmail.com/help/technical/smtp.html' },
    ],
  },
  proton: {
    text: 'Proton Mail has no direct IMAP — install Proton Mail Bridge, then add it here as a custom account on 127.0.0.1 (IMAP port 1143, SMTP port 1025) with your Proton credentials.',
    links: [
      { label: 'Proton Mail Bridge', href: 'https://proton.me/support/mail-bridge' },
    ],
  },
  custom: {
    text: 'Use your provider’s IMAP and SMTP server addresses, ports and security. Most providers now require an app password or OAuth2 instead of your normal password.',
    links: [],
  },
};

let tileEl = null;
let barAccountBtn = null;
let barAccountLabel = null;
let statusDot = null;
let foldersEl = null;
let listEl = null;
let readerEl = null;
let bodyEl = null;

let accounts = [];
let presets = [];
let configured = false;
let currentAccount = null;
let folders = [];
let currentFolder = 'INBOX';
let messages = [];
let selectedMessageId = null;
let currentMessage = null;
let busy = false;

/* Account picker popup (body-level). */
let accountPopup = null;
let accountPopupOpen = false;

/* Compose + settings modals. */
let composeModal = null;
let settingsModal = null;

// Compose field elements (built once; prefilled on reply/forward).
let composeFrom = null;
let composeTo = null;
let composeCc = null;
let composeBcc = null;
let composeSubject = null;
let composeBody = null;

/* ── API ────────────────────────────────────────────────────── */

async function api(path, options = {}) {
  const res = await apiFetch(path, options);
  return res?.data ?? null;
}

function status() {
  return api('/api/mail/status');
}

function listFolders(accountId) {
  const q = accountId ? `?account=${encodeURIComponent(accountId)}` : '';
  return api(`/api/mail/folders${q}`);
}

function listMessages(accountId, folder, page = 0) {
  const q = new URLSearchParams({ folder });
  if (accountId) q.set('account', accountId);
  if (page) q.set('page', String(page));
  return api(`/api/mail/list?${q}`);
}

function fetchMessage(accountId, folder, id) {
  const q = new URLSearchParams({ folder, id });
  if (accountId) q.set('account', accountId);
  return api(`/api/mail/message?${q}`);
}

function markSeen(accountId, folder, ids, seen) {
  return api('/api/mail/flag', {
    method: 'POST',
    body: JSON.stringify({ account_id: accountId, folder, ids, seen }),
  });
}

function sendMail(payload) {
  return api('/api/mail/send', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

function deleteMessage(accountId, folder, id) {
  return api('/api/mail/delete', {
    method: 'POST',
    body: JSON.stringify({ account_id: accountId, folder, id }),
  });
}

function createAccount(payload) {
  return api('/api/mail/accounts', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

function testAccount(payload) {
  return api('/api/mail/accounts/test', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

function updateAccount(id, payload) {
  return api(`/api/mail/accounts/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify(payload),
  });
}

function deleteAccount(id) {
  return api(`/api/mail/accounts/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/* ── State helpers ──────────────────────────────────────────── */

function verifiedAccounts() {
  return accounts.filter((a) => a.verified);
}

function accountById(id) {
  return accounts.find((a) => a.id === id) || null;
}

function pickAccount() {
  if (currentAccount && accounts.some((a) => a.id === currentAccount.id)) return currentAccount;
  const verified = verifiedAccounts();
  currentAccount = verified[0] || accounts[0] || null;
  return currentAccount;
}

function setStatus(mode) {
  if (!statusDot) return;
  statusDot.classList.toggle('is-ok', mode === 'ok');
}

/* ── Refresh pipeline ───────────────────────────────────────── */

async function refreshStatus({ keepFolder = false } = {}) {
  try {
    const s = await status();
    accounts = s?.accounts || [];
    presets = s?.presets || [];
    configured = !!s?.configured;
  } catch (e) {
    toast(e.message || 'Could not load mail status', { type: 'error' });
    return;
  }
  setStatus(configured ? 'ok' : 'off');
  render();
  if (pickAccount() && (configured || accounts.length)) {
    if (!keepFolder) {
      await loadFolders();
    } else {
      await loadFoldersAndMessages();
    }
  }
}

async function loadFolders() {
  const account = pickAccount();
  if (!account) return;
  try {
    const r = await listFolders(account.id);
    folders = r?.folders || [];
    if (!folders.some((f) => f.name === currentFolder)) {
      currentFolder = folders.find((f) => /^inbox$/i.test(f.name))?.name || folders[0]?.name || 'INBOX';
    }
    renderFolders();
    await loadMessages();
  } catch (e) {
    toast(e.message || 'Could not load folders', { type: 'error' });
  }
}

async function loadFoldersAndMessages() {
  const account = pickAccount();
  if (!account) return;
  try {
    const r = await listFolders(account.id);
    folders = r?.folders || [];
    renderFolders();
  } catch (e) { /* folder refresh failure is non-fatal */ }
  await loadMessages();
}

async function loadMessages() {
  const account = pickAccount();
  if (!account || busy) return;
  busy = true;
  try {
    const r = await listMessages(account.id, currentFolder);
    messages = r?.messages || [];
    renderMessages();
  } catch (e) {
    toast(e.message || 'Could not load messages', { type: 'error' });
  } finally {
    busy = false;
  }
}

async function openMessage(id) {
  const account = pickAccount();
  if (!account) return;
  const msg = messages.find((m) => m.id === id);
  selectedMessageId = id;
  currentMessage = null;
  renderMessages();
  readerEl.textContent = '';
  readerEl.appendChild(emptyState({ title: 'Loading…' }));
  try {
    const full = await fetchMessage(account.id, currentFolder, id);
    currentMessage = full;
    renderMessage(full);
    if (msg && !msg.seen) {
      void markSeen(account.id, currentFolder, [id], true).catch(() => {});
      msg.seen = true;
    }
  } catch (e) {
    readerEl.textContent = '';
    readerEl.appendChild(emptyState({ title: 'Could not open message', body: e.message }));
  }
}

/* ── Rendering: top bar ─────────────────────────────────────── */

function h(tag, className, text) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text != null) el.textContent = text;
  return el;
}

function toolbarButton(iconName, label, onClick, danger = false) {
  const btn = button({ icon: iconName, variant: 'ghost', onClick });
  btn.classList.add('ui-btn--icon', 'mail-tool');
  if (danger) btn.classList.add('mail-tool--danger');
  btn.title = label;
  btn.setAttribute('aria-label', label);
  return btn;
}

function renderBar() {
  const account = pickAccount();
  barAccountLabel.textContent = account ? (account.label || account.email) : 'No account';
  barAccountBtn.setAttribute('aria-expanded', String(accountPopupOpen));
}

function toggleAccountMenu() {
  if (accountPopupOpen) closeAccountMenu();
  else openAccountMenu();
}

function ensureAccountMenu() {
  if (accountPopup) return;
  accountPopup = document.createElement('div');
  accountPopup.className = 'mail-account-menu hidden';
  accountPopup.setAttribute('role', 'menu');
  document.body.appendChild(accountPopup);
}

function closeAccountMenu() {
  if (!accountPopupOpen) return;
  accountPopupOpen = false;
  accountPopup?.classList.add('hidden');
  renderBar();
  document.removeEventListener('pointerdown', onAccountMenuOutside, true);
}

function onAccountMenuOutside(e) {
  if (accountPopup && !accountPopup.contains(e.target)
    && barAccountBtn && !barAccountBtn.contains(e.target)) {
    closeAccountMenu();
  }
}

function openAccountMenu() {
  ensureAccountMenu();
  accountPopup.innerHTML = '';
  if (!accounts.length) {
    const empty = h('div', 'mail-account-menu-empty', 'No accounts yet');
    accountPopup.appendChild(empty);
  } else {
    for (const a of accounts) {
      const item = h('button', 'mail-account-menu-item');
      item.type = 'button';
      item.setAttribute('role', 'menuitem');
      if (a.id === currentAccount?.id) item.classList.add('is-active');
      const icon = h('span', 'mail-account-menu-icon');
      item.appendChild(icon);
      void setIcon(icon, 'ui/mail', { size: 14 });
      const label = h('span', 'mail-account-menu-label', a.label || a.email);
      const badge = h('span', `mail-account-row-badge${a.verified ? ' is-ok' : ''}`, a.verified ? 'connected' : 'not verified');
      item.append(icon, label, badge);
      item.addEventListener('click', () => {
        closeAccountMenu();
        currentAccount = a;
        selectedMessageId = null;
        renderBar();
        void loadFolders();
      });
      accountPopup.appendChild(item);
    }
  }
  const foot = h('div', 'mail-account-menu-foot');
  const manage = button({ icon: 'ui/settings', label: 'Mail accounts', variant: 'ghost', size: 'sm', onClick: () => { closeAccountMenu(); openSettings(); } });
  manage.classList.add('ui-btn--icon');
  foot.appendChild(manage);
  accountPopup.appendChild(foot);

  accountPopup.classList.remove('hidden');
  const r = barAccountBtn.getBoundingClientRect();
  const left = Math.max(12, Math.min(r.left, window.innerWidth - 280 - 12));
  accountPopup.style.left = `${left}px`;
  accountPopup.style.top = `${r.bottom + 8}px`;
  accountPopupOpen = true;
  renderBar();
  document.addEventListener('pointerdown', onAccountMenuOutside, true);
}

/* ── Rendering: main body ───────────────────────────────────── */

function render() {
  bodyEl.innerHTML = '';
  if (!configured) {
    renderSetup(bodyEl);
    return;
  }

  const foldersPane = h('div', 'mail-folders');
  foldersEl = foldersPane;
  const listPane = h('div', 'mail-list');
  listEl = listPane;
  const readerPane = h('div', 'mail-reader');
  readerEl = readerPane;
  readerEl.appendChild(emptyState({ title: 'Select a message', body: 'Pick a message to read it here.' }));
  bodyEl.append(foldersPane, listPane, readerPane);
  renderFolders();
  renderMessages();
}

function renderSetup(root) {
  const wrap = h('div', 'mail-onboarding');
  const intro = h('div', 'mail-help');
  if (!accounts.length) {
    intro.appendChild(h('h3', null, 'Connect your mail account'));
    intro.appendChild(h('p', null, 'Choose your provider and add your account. Provider instructions appear at the bottom of the form and disappear once the account is verified.'));
  } else {
    intro.appendChild(h('h3', null, 'Verify your mail account'));
    intro.appendChild(h('p', null, 'Your account is saved but the connection test hasn’t succeeded yet. Press Test connection below, or read the provider instructions at the bottom.'));
  }
  wrap.appendChild(intro);
  wrap.appendChild(buildAccountForm({ onboarding: !accounts.length }));
  root.appendChild(wrap);
}

function defaultPresets() {
  return [
    { provider: 'gmail', label: 'Gmail', imap_host: 'imap.gmail.com', imap_port: 993, imap_security: 'ssl', smtp_host: 'smtp.gmail.com', smtp_port: 465, smtp_security: 'ssl' },
    { provider: 'outlook', label: 'Outlook', imap_host: 'outlook.office365.com', imap_port: 993, imap_security: 'ssl', smtp_host: 'smtp-mail.outlook.com', smtp_port: 587, smtp_security: 'starttls' },
    { provider: 'yahoo', label: 'Yahoo Mail', imap_host: 'imap.mail.yahoo.com', imap_port: 993, imap_security: 'ssl', smtp_host: 'smtp.mail.yahoo.com', smtp_port: 465, smtp_security: 'ssl' },
    { provider: 'icloud', label: 'iCloud Mail', imap_host: 'imap.mail.me.com', imap_port: 993, imap_security: 'ssl', smtp_host: 'smtp.mail.me.com', smtp_port: 587, smtp_security: 'starttls' },
    { provider: 'zoho', label: 'Zoho Mail', imap_host: 'imap.zoho.com', imap_port: 993, imap_security: 'ssl', smtp_host: 'smtp.zoho.com', smtp_port: 465, smtp_security: 'ssl' },
    { provider: 'fastmail', label: 'Fastmail', imap_host: 'imap.fastmail.com', imap_port: 993, imap_security: 'ssl', smtp_host: 'smtp.fastmail.com', smtp_port: 465, smtp_security: 'ssl' },
    { provider: 'proton', label: 'Proton (Bridge)', imap_host: '127.0.0.1', imap_port: 1143, imap_security: 'none', smtp_host: '127.0.0.1', smtp_port: 1025, smtp_security: 'none' },
  ];
}

/** The add-account form: provider + email + password, optional custom servers. */
function buildAccountForm({ onboarding }) {
  const form = h('div', 'mail-account-form');
  form.appendChild(h('h4', null, onboarding ? 'Add your first account' : 'Add another account'));

  const providerSel = select({
    options: (presets.length ? presets : defaultPresets()).map((p) => ({ value: p.provider, label: p.label })),
    value: 'gmail',
  });
  providerSel.select.addEventListener('change', () => {
    const p = presets.find((x) => x.provider === providerSel.select.value)
      || defaultPresets().find((x) => x.provider === providerSel.select.value);
    if (p) {
      emailInput.value = '';
      imapHost.value = p.imap_host;
      imapPort.value = String(p.imap_port);
      imapSec.select.value = p.imap_security;
      smtpHost.value = p.smtp_host;
      smtpPort.value = String(p.smtp_port);
      smtpSec.select.value = p.smtp_security;
    }
    updateHelp(providerSel.select.value);
  });
  form.appendChild(field({ label: 'Provider', control: providerSel }));

  const emailInput = input({ type: 'email', placeholder: 'you@example.com', autocomplete: 'username' });
  form.appendChild(field({ label: 'Email', control: emailInput }));

  const userInput = input({ type: 'text', placeholder: 'Usually your full email', autocomplete: 'username' });
  form.appendChild(field({ label: 'Username (optional)', control: userInput }));

  const passInput = input({ type: 'password', placeholder: 'Password or app password', autocomplete: 'current-password' });
  form.appendChild(field({ label: 'Password', control: passInput }));

  const imapHost = input({ placeholder: 'imap.example.com' });
  const imapPort = input({ type: 'number', value: '993', placeholder: '993' });
  const imapSec = select({ options: ['ssl', 'starttls', 'none'], value: 'ssl' });
  form.appendChild(field({ label: 'IMAP server', control: imapHost }));
  const imapRow = h('div', 'ui-row');
  imapRow.style.gap = '8px';
  imapRow.append(field({ label: 'Port', control: imapPort }), field({ label: 'Security', control: imapSec }));
  form.appendChild(imapRow);

  const smtpHost = input({ placeholder: 'smtp.example.com' });
  const smtpPort = input({ type: 'number', value: '465', placeholder: '465' });
  const smtpSec = select({ options: ['ssl', 'starttls', 'none'], value: 'ssl' });
  form.appendChild(field({ label: 'SMTP server', control: smtpHost }));
  const smtpRow = h('div', 'ui-row');
  smtpRow.style.gap = '8px';
  smtpRow.append(field({ label: 'Port', control: smtpPort }), field({ label: 'Security', control: smtpSec }));
  form.appendChild(smtpRow);

  const actions = h('div', 'mail-compose-actions');
  const statusLine = h('span', 'mail-account-row-email');
  statusLine.style.flex = '1';
  const testBtn = button({ label: 'Test connection', variant: 'ghost', size: 'sm' });
  testBtn.setAttribute('aria-label', 'Test connection');
  const addBtn = button({ label: 'Add account', variant: 'primary', size: 'sm' });
  addBtn.setAttribute('aria-label', 'Add account');
  actions.append(statusLine, testBtn, addBtn);
  form.appendChild(actions);

  // Provider instructions live at the bottom and change with the selection.
  const helpEl = h('div', 'mail-provider-help');
  form.appendChild(helpEl);
  function updateHelp(provider) {
    const p = presets.find((x) => x.provider === provider) || defaultPresets().find((x) => x.provider === provider);
    const helpText = PROVIDER_HELP[provider] || PROVIDER_HELP.custom;
    helpEl.textContent = '';
    const label = h('strong', null, (p?.label || provider) + ': ');
    helpEl.appendChild(label);
    helpEl.appendChild(document.createTextNode(helpText.text + ' '));
    for (const link of helpText.links || []) {
      const a = h('a', null, link.label);
      a.href = link.href;
      a.target = '_blank';
      a.rel = 'noopener noreferrer';
      helpEl.appendChild(a);
      helpEl.appendChild(document.createTextNode(' '));
    }
  }
  updateHelp(providerSel.select.value);

  const collect = () => ({
    email: emailInput.value.trim(),
    provider: providerSel.select.value,
    username: userInput.value.trim() || undefined,
    password: passInput.value,
    imap_host: imapHost.value.trim() || undefined,
    imap_port: Number(imapPort.value) || undefined,
    imap_security: imapSec.select.value,
    smtp_host: smtpHost.value.trim() || undefined,
    smtp_port: Number(smtpPort.value) || undefined,
    smtp_security: smtpSec.select.value,
    test: false,
  });

  testBtn.addEventListener('click', async () => {
    testBtn.disabled = true;
    statusLine.textContent = 'Testing…';
    try {
      const r = await testAccount(collect());
      if (r?.ok) {
        statusLine.textContent = '✓ Connected';
        toast('Connection works — provider configured', { type: 'info' });
        await refreshStatus();
      } else {
        statusLine.textContent = `✗ ${r?.error || 'Connection failed'}`;
        toast(r?.error || 'Connection failed', { type: 'error' });
      }
    } catch (e) {
      statusLine.textContent = `✗ ${e.message}`;
      toast(e.message, { type: 'error' });
    } finally {
      testBtn.disabled = false;
    }
  });

  addBtn.addEventListener('click', async () => {
    const payload = collect();
    payload.test = true;
    if (!payload.email || !payload.password) {
      toast('Email and password are required', { type: 'error' });
      return;
    }
    addBtn.disabled = true;
    statusLine.textContent = 'Adding…';
    try {
      const created = await createAccount(payload);
      if (created?.verified) {
        toast(`Account ${created.email} connected`, { type: 'info' });
        await refreshStatus({ keepFolder: true });
      } else {
        statusLine.textContent = `Added but not verified — ${created?.last_error || 'test failed'}`;
        toast('Account added, but the connection test failed — check the provider help', { type: 'error' });
        await refreshStatus({ keepFolder: true });
      }
    } catch (e) {
      statusLine.textContent = `✗ ${e.message}`;
      toast(e.message, { type: 'error' });
    } finally {
      addBtn.disabled = false;
    }
  });

  return form;
}

/* ── Rendering: folders / list / reader ─────────────────────── */

function renderFolders() {
  if (!foldersEl) return;
  foldersEl.textContent = '';
  const sorted = [...folders].sort((a, b) => {
    const rank = (n) => (/^inbox$/i.test(n) ? 0 : /^sent$/i.test(n) ? 1 : /^drafts$/i.test(n) ? 2 : /^trash$/i.test(n) ? 3 : 4);
    return rank(a.name) - rank(b.name) || a.name.localeCompare(b.name);
  });
  if (!sorted.length) {
    foldersEl.appendChild(h('div', 'mail-folder', '…'));
    return;
  }
  for (const f of sorted) {
    const item = h('button', 'mail-folder');
    item.type = 'button';
    if (f.name === currentFolder) item.classList.add('is-active');
    const ic = h('span', 'mail-folder-icon');
    item.appendChild(ic);
    void setIcon(ic, 'ui/folder', { size: 13 });
    item.appendChild(h('span', 'mail-folder-name', f.name));
    const count = f.unread ? h('span', 'mail-folder-count', String(f.unread)) : null;
    if (count) item.appendChild(count);
    item.addEventListener('click', () => {
      currentFolder = f.name;
      selectedMessageId = null;
      renderFolders();
      void loadMessages();
    });
    foldersEl.appendChild(item);
  }
}

function renderMessages() {
  if (!listEl) return;
  listEl.textContent = '';
  if (!messages.length) {
    listEl.appendChild(emptyState({ title: 'No messages', body: `Nothing in ${currentFolder}.` }));
    return;
  }
  for (const m of messages) {
    const item = h('button', 'mail-item');
    item.type = 'button';
    if (m.id === selectedMessageId) item.classList.add('is-active');
    if (!m.seen) item.classList.add('is-unread');
    const head = h('div', 'mail-item-head');
    head.appendChild(h('span', 'mail-item-dot'));
    head.appendChild(h('span', 'mail-item-from', m.from || 'Unknown'));
    head.appendChild(h('span', 'mail-item-time', fmtWhen(m.date)));
    const subject = h('div', 'mail-item-subject', m.subject || '(no subject)');
    const snippet = h('div', 'mail-item-snippet', m.snippet || '');
    item.append(head, subject, snippet);
    item.addEventListener('click', () => void openMessage(m.id));
    listEl.appendChild(item);
  }
}

function renderMessage(msg) {
  if (!readerEl) return;
  readerEl.textContent = '';
  const subject = h('h2', 'mail-reader-subject', msg.subject || '(no subject)');
  const from = (msg.from || []).join(', ');
  const to = (msg.to || []).join(', ');
  const meta = h('p', 'mail-reader-meta', `From: ${from || '—'}${to ? `\nTo: ${to}` : ''}${msg.date ? `\n${fmtWhen(msg.date)}` : ''}`);

  const actions = h('div', 'mail-reader-actions');
  const replyBtn = button({ icon: 'ui/reply', label: 'Reply', variant: 'ghost', size: 'sm', onClick: () => openReply(msg) });
  const fwdBtn = button({ icon: 'ui/forward', label: 'Forward', variant: 'ghost', size: 'sm', onClick: () => openForward(msg) });
  const delBtn = button({ icon: 'ui/trash', label: 'Delete', variant: 'ghost', size: 'sm', onClick: () => deleteCurrent(msg) });
  actions.append(replyBtn, fwdBtn, delBtn);

  readerEl.append(subject, meta, actions);

  const content = h('div', 'mail-reader-content');
  const hasHtml = !!msg.html && msg.html.trim();
  if (hasHtml) {
    const frame = document.createElement('iframe');
    frame.sandbox = '';
    frame.srcdoc = msg.html;
    content.appendChild(frame);
  } else {
    content.appendChild(h('div', 'mail-reader-body', msg.text || '(empty message)'));
  }
  readerEl.appendChild(content);

  if (msg.attachments && msg.attachments.length) {
    const atts = h('div', 'mail-attachments');
    for (const att of msg.attachments) {
      const chip = h('span', 'mail-attachment', `${att.filename || 'attachment'} · ${fmtSize(att.size)}`);
      atts.appendChild(chip);
    }
    readerEl.appendChild(atts);
  }
}

/* ── Compose ────────────────────────────────────────────────── */

function openCompose(prefill = {}) {
  if (!pickAccount()) {
    toast('Add and verify a mail account first', { type: 'error' });
    return;
  }
  if (!composeModal) {
    composeTo = input({ placeholder: 'recipient@example.com' });
    composeCc = input({ placeholder: 'cc@example.com' });
    composeBcc = input({ placeholder: 'bcc@example.com' });
    composeSubject = input({ placeholder: 'Subject' });
    composeBody = textarea({ placeholder: 'Write your message…', rows: 12 });
    composeBody.classList.add('mail-compose-body');
    composeFrom = h('p', 'mail-account-row-email', 'From: ');
    const actions = h('div', 'mail-compose-actions');
    const sendBtn = button({ icon: 'ui/send', label: 'Send', variant: 'primary', size: 'sm' });
    const closeBtn = button({ label: 'Discard', variant: 'ghost', size: 'sm' });
    actions.append(sendBtn, closeBtn);

    composeModal = modal({
      title: 'New message',
      wide: true,
      body: [composeFrom, field({ label: 'To', control: composeTo }), field({ label: 'Cc', control: composeCc }), field({ label: 'Bcc', control: composeBcc }), field({ label: 'Subject', control: composeSubject }), field({ label: 'Message', control: composeBody }), actions],
    });
    closeBtn.addEventListener('click', () => composeModal.close());

    sendBtn.addEventListener('click', async () => {
      const to = parseAddresses(composeTo.value);
      if (!to.length) {
        toast('At least one recipient required', { type: 'error' });
        return;
      }
      sendBtn.disabled = true;
      try {
        await sendMail({
          account_id: pickAccount().id,
          to,
          cc: parseAddresses(composeCc.value),
          bcc: parseAddresses(composeBcc.value),
          subject: composeSubject.value.trim(),
          body: composeBody.value,
        });
        toast('Message sent', { type: 'info' });
        composeModal.close();
      } catch (e) {
        toast(e.message || 'Send failed', { type: 'error' });
      } finally {
        sendBtn.disabled = false;
      }
    });
  }

  composeTo.value = prefill.to || '';
  composeCc.value = prefill.cc || '';
  composeBcc.value = prefill.bcc || '';
  composeSubject.value = prefill.subject || '';
  composeBody.value = prefill.body || '';
  composeFrom.textContent = `From: ${pickAccount().email}`;
  composeModal.open();
}

function openReply(msg) {
  const toEmail = (msg.from_addresses && msg.from_addresses[0] && msg.from_addresses[0].email) || '';
  openCompose({
    to: toEmail,
    subject: `Re: ${msg.subject || ''}`,
    body: quotedBody(msg),
  });
}

function openForward(msg) {
  openCompose({
    subject: `Fwd: ${msg.subject || ''}`,
    body: quotedBody(msg),
  });
}

async function deleteCurrent(msg) {
  const account = pickAccount();
  if (!account) return;
  if (!window.confirm(`Delete this message from ${currentFolder}?`)) return;
  try {
    await deleteMessage(account.id, currentFolder, msg.id);
    toast('Message deleted', { type: 'info' });
    currentMessage = null;
    selectedMessageId = null;
    readerEl.textContent = '';
    readerEl.appendChild(emptyState({ title: 'Select a message', body: 'Pick a message to read it here.' }));
    await loadMessages();
  } catch (e) {
    toast(e.message || 'Delete failed', { type: 'error' });
  }
}

function quotedBody(msg) {
  const from = (msg.from || []).join(', ');
  const date = msg.date ? fmtWhen(msg.date) : '';
  const text = (msg.text && msg.text.trim()) ? msg.text.trim() : '';
  const body = text || '(no body)';
  return `\n\n----------\nFrom: ${from}\nDate: ${date}\n\n${body}`;
}

/* ── Settings ───────────────────────────────────────────────── */

function openSettings() {
  if (!settingsModal) {
    const body = h('div', 'ui-stack');
    body.style.gap = '12px';
    const list = h('div', 'ui-stack');
    list.style.gap = '8px';
    const form = buildAccountForm({ onboarding: false });
    const actions = h('div', 'mail-compose-actions');
    const doneBtn = button({ label: 'Done', variant: 'primary', size: 'sm' });
    actions.appendChild(doneBtn);

    settingsModal = modal({
      title: 'Mail accounts',
      wide: true,
      body: [list, form, actions],
    });
    doneBtn.addEventListener('click', () => settingsModal.close());
    settingsModal.el.addEventListener('open', () => {
      list.textContent = '';
      if (!accounts.length) {
        list.appendChild(h('p', 'mail-account-row-email', 'No accounts yet.'));
      }
      for (const a of accounts) {
        const row = h('div', 'mail-account-row');
        const meta = h('div', 'ui-stack');
        meta.style.flex = '1';
        meta.style.gap = '2px';
        meta.appendChild(h('div', 'mail-account-row-label', a.label || a.email));
        meta.appendChild(h('div', 'mail-account-row-email', a.email));
        const badge = h('span', `mail-account-row-badge${a.verified ? ' is-ok' : ''}`, a.verified ? 'connected' : a.last_error ? 'failed' : 'not tested');
        const testBtn = button({ label: 'Test', variant: 'ghost', size: 'sm' });
        testBtn.title = 'Test connection';
        const delBtn = button({ icon: 'ui/trash', variant: 'ghost', size: 'sm' });
        delBtn.title = 'Remove account';
        delBtn.classList.add('ui-btn--icon', 'mail-tool--danger');
        row.append(meta, badge, testBtn, delBtn);
        testBtn.addEventListener('click', async () => {
          testBtn.disabled = true;
          try {
            const r = await testAccount({ id: a.id });
            badge.textContent = r?.ok ? 'connected' : (r?.error || 'failed');
            badge.classList.toggle('is-ok', !!r?.ok);
            if (r?.ok) toast(`${a.email} connected`, { type: 'info' });
            await refreshStatus({ keepFolder: true });
          } catch (e) {
            toast(e.message, { type: 'error' });
          } finally {
            testBtn.disabled = false;
          }
        });
        delBtn.addEventListener('click', async () => {
          if (!window.confirm(`Remove mail account ${a.email}?`)) return;
          try {
            await deleteAccount(a.id);
            if (currentAccount?.id === a.id) currentAccount = null;
            await refreshStatus({ keepFolder: true });
          } catch (e) {
            toast(e.message, { type: 'error' });
          }
        });
        list.appendChild(row);
      }
    });
  }
  settingsModal.open();
}

/* ── misc helpers ───────────────────────────────────────────── */

function parseAddresses(value) {
  return value
    .split(/[,;]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

function fmtWhen(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const today = new Date();
  const sameDay = d.toDateString() === today.toDateString();
  return sameDay
    ? d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

function fmtSize(n) {
  if (!n) return '';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

/** Create the Mail tile element (the plugin's window container). */
export function mountMailTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile mail-tile';
  tileEl.dataset.plugin = MAIL_PLUGIN;

  /* Top bar: account menu + title + status dot */
  const bar = h('div', 'mail-bar');
  barAccountBtn = document.createElement('button');
  barAccountBtn.type = 'button';
  barAccountBtn.className = 'mail-account-btn';
  barAccountBtn.setAttribute('aria-haspopup', 'menu');
  barAccountBtn.setAttribute('aria-expanded', 'false');
  barAccountBtn.title = 'Accounts';
  const icon = h('span', 'mail-account-btn-icon');
  barAccountBtn.appendChild(icon);
  void setIcon(icon, 'ui/mail', { size: 15 });
  barAccountLabel = h('span', 'mail-account-btn-label', 'No account');
  const chevron = h('span', 'mail-account-btn-chevron');
  barAccountBtn.appendChild(chevron);
  void setIcon(chevron, 'ui/chevron-down', { size: 12 });
  barAccountBtn.appendChild(barAccountLabel);
  barAccountBtn.addEventListener('click', toggleAccountMenu);

  const title = h('div', 'mail-title', 'Mail');
  statusDot = h('span', 'mail-status-dot');
  statusDot.setAttribute('aria-hidden', 'true');

  // Single top bar (Studio-style): account menu + title + actions + status dot.
  bar.append(
    barAccountBtn,
    title,
    toolbarButton('ui/plus', 'New message', openCompose),
    toolbarButton('ui/refresh', 'Refresh', () => void refreshStatus({ keepFolder: true })),
    toolbarButton('ui/settings', 'Mail accounts', openSettings),
    statusDot,
  );
  tileEl.appendChild(bar);

  /* Body */
  bodyEl = h('div', 'mail-body');
  tileEl.appendChild(bodyEl);

  void refreshStatus();
  return tileEl;
}

/** Deactivated: drop the window. */
export function unmountMailTile() {
  closeAccountMenu();
  composeModal?.close();
  settingsModal?.close();
  tileEl?.remove();
  tileEl = null;
  barAccountBtn = null;
  barAccountLabel = null;
  statusDot = null;
  foldersEl = null;
  listEl = null;
  readerEl = null;
  bodyEl = null;
}

/** The tile element (or null when the Mail window is not mounted). */
export function getMailTileElement() {
  return tileEl;
}

/* ── AI wiring ──────────────────────────────────────────────── */

function onAgentActions(e) {
  const actions = e.detail || [];
  const mailActions = actions.filter((a) => /^mail_/.test(a?.action || ''));
  if (!mailActions.length) return;

  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: MAIL_PLUGIN } }));

  const sent = mailActions.some((a) => a.action === 'mail_send' && a.result === 'ok');
  const listed = mailActions.some((a) => a.action === 'mail_list' && a.result === 'ok');
  if (sent) {
    toast('Message sent', { type: 'info' });
    void refreshStatus({ keepFolder: true });
    // Re-check shortly after: the delivered copy can take a moment to land.
    window.setTimeout(() => void refreshStatus({ keepFolder: true }), 2500);
  } else if (listed) {
    void refreshStatus({ keepFolder: true });
  }
}

let wired = false;
export function wireMailEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
}

export default {
  name: 'mail',
  icon: 'ui/mail',
  mount: mountMailTile,
  unmount: unmountMailTile,
  getElement: getMailTileElement,
  wireEvents: wireMailEvents,
};
