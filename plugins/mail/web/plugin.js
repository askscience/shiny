import {
  searchBar, menuSurface, menuItem, confirmDialog, menuButton, toolbar, element as h,
  toolButton, placePopover, button, emptyState, field, icon, input, modal, notify,
  select, textarea, toast, setTileGlow, glowGradient, setIcon,
} from '../../ui/index.js';

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

import { apiFetch } from '../../js/api.js';
import { t, tp } from '../../js/i18n.js';
import { externalLinkUrl, isKiosk, openUrl } from '../../js/linkRouter.js';

export const MAIL_PLUGIN = 'mail';

const PROVIDER_HELP = {
  gmail: {
    textKey: 'plugin.mail.provider.gmail.text',
    links: [
      { labelKey: 'plugin.mail.provider.gmail.imapLabel', href: 'https://support.google.com/mail/answer/7126229' },
      { labelKey: 'plugin.mail.provider.gmail.appPasswordLabel', href: 'https://myaccount.google.com/apppasswords' },
    ],
  },
  outlook: {
    textKey: 'plugin.mail.provider.outlook.text',
    links: [
      { labelKey: 'plugin.mail.provider.outlook.settingsLabel', href: 'https://support.microsoft.com/en-us/office/pop-imap-and-smtp-settings-for-outlook-com-d088b986-291d-42b8-9564-9c414e2aa040' },
    ],
  },
  yahoo: {
    textKey: 'plugin.mail.provider.yahoo.text',
    links: [
      { labelKey: 'plugin.mail.provider.yahoo.appPasswordsLabel', href: 'https://help.yahoo.com/kb/SLN4075.html' },
    ],
  },
  icloud: {
    textKey: 'plugin.mail.provider.icloud.text',
    links: [
      { labelKey: 'plugin.mail.provider.icloud.appPasswordsLabel', href: 'https://support.apple.com/en-us/102525' },
    ],
  },
  zoho: {
    textKey: 'plugin.mail.provider.zoho.text',
    links: [
      { labelKey: 'plugin.mail.provider.zoho.settingsLabel', href: 'https://www.zoho.com/mail/help/zoho-mail-apps.html' },
    ],
  },
  fastmail: {
    textKey: 'plugin.mail.provider.fastmail.text',
    links: [
      { labelKey: 'plugin.mail.provider.fastmail.settingsLabel', href: 'https://www.fastmail.com/help/technical/smtp.html' },
    ],
  },
  proton: {
    textKey: 'plugin.mail.provider.proton.text',
    links: [
      { labelKey: 'plugin.mail.provider.proton.bridgeLabel', href: 'https://proton.me/support/mail-bridge' },
    ],
  },
  custom: {
    textKey: 'plugin.mail.provider.custom.text',
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
/** The reader's HTML-message frame, or null when the message is plain text. */
let readerFrameEl = null;
let bodyEl = null;
let searchInputEl = null;
let searchTimer = null;

let accounts = [];
let presets = [];
let configured = false;
let currentAccount = null;
let folders = [];
let currentFolder = 'INBOX';
let messages = [];
let selectedMessageId = null;
let currentMessage = null;
// Bumped per openMessage so a slow fetch for message A can't overwrite the
// reader after the user clicked message B.
let openToken = 0;
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

function searchMessages(accountId, query, folder) {
  const q = new URLSearchParams({ query });
  if (accountId) q.set('account', accountId);
  if (folder) q.set('folder', folder);
  return api(`/api/mail/search?${q}`);
}

function syncMail(accountId, folder) {
  return api('/api/mail/sync', {
    method: 'POST',
    body: JSON.stringify({ account: accountId, folder }),
  });
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

function deleteAccount(id) {
  return api(`/api/mail/accounts/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/* ── State helpers ──────────────────────────────────────────── */

function verifiedAccounts() {
  return accounts.filter((a) => a.verified);
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
    toast(e.message || t('plugin.mail.toast.statusFailed', null, 'Could not load mail status'), { type: 'error' });
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

/** Pull new mail into the local cache, then reload the current folder. */
async function syncNow() {
  const account = pickAccount();
  if (!account) return;
  try {
    const r = await syncMail(account.id, currentFolder);
    toast(t('plugin.mail.toast.synced', { new: r?.new ?? 0, cached: r?.total ?? 0 }, 'Mail synced — {new} new, {cached} cached'), { type: 'info' });
  } catch (e) {
    toast(e.message || t('plugin.mail.toast.syncFailed', null, 'Mail sync failed'), { type: 'error' });
  }
  await refreshStatus({ keepFolder: true });
}

/** Debounced local search over the cached mail. */
function onSearch(query) {
  if (searchTimer) window.clearTimeout(searchTimer);
  searchTimer = window.setTimeout(async () => {
    const q = (query || '').trim();
    const account = pickAccount();
    if (!account) return;
    if (!q) {
      await loadMessages();
      return;
    }
    try {
      const r = await searchMessages(account.id, q, currentFolder);
      messages = r?.messages || [];
      renderMessages();
    } catch (e) {
      toast(e.message || t('plugin.mail.toast.searchFailed', null, 'Search failed'), { type: 'error' });
    }
  }, 300);
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
    toast(e.message || t('plugin.mail.toast.foldersFailed', null, 'Could not load folders'), { type: 'error' });
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
    toast(e.message || t('plugin.mail.toast.messagesFailed', null, 'Could not load messages'), { type: 'error' });
  } finally {
    busy = false;
  }
}

async function openMessage(id) {
  const account = pickAccount();
  if (!account) return;
  const token = ++openToken;
  const msg = messages.find((m) => m.id === id);
  selectedMessageId = id;
  currentMessage = null;
  renderMessages();
  readerEl.textContent = '';
  readerEl.appendChild(emptyState({ title: t('plugin.mail.empty.loading', null, 'Loading…') }));
  try {
    const full = await fetchMessage(account.id, currentFolder, id);
    if (token !== openToken) return; // a newer open won
    currentMessage = full;
    // Mail carries no images: glow with a colour pair derived from the sender.
    const from = currentMessage.from;
    setTileGlow(tileEl, senderGlow(Array.isArray(from) ? from[0] : from));
    renderMessage(full);
    if (msg && !msg.seen) {
      void markSeen(account.id, currentFolder, [id], true).catch(() => {});
      msg.seen = true;
    }
  } catch (e) {
    if (token !== openToken) return;
    readerEl.textContent = '';
    readerEl.appendChild(emptyState({ title: t('plugin.mail.empty.openFailed', null, 'Could not open message'), body: e.message }));
  }
}

/* ── Rendering: top bar ─────────────────────────────────────── */

function toolbarButton(iconName, label, onClick, danger = false) {
  const btn = toolButton(iconName, label, onClick, { className: 'mail-tool', danger });
  return btn;
}

function renderBar() {
  const account = pickAccount();
  barAccountLabel.textContent = account ? (account.label || account.email) : t('plugin.mail.bar.noAccount', null, 'No account');
  barAccountBtn.setAttribute('aria-expanded', String(accountPopupOpen));
}

function toggleAccountMenu() {
  if (accountPopupOpen) closeAccountMenu();
  else openAccountMenu();
}

function ensureAccountMenu() {
  if (accountPopup) return;
  accountPopup = menuSurface({ className: 'mail-account-menu', onEscape: closeAccountMenu });
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
    const empty = h('div', 'mail-account-menu-empty', t('plugin.mail.menu.noAccounts', null, 'No accounts yet'));
    accountPopup.appendChild(empty);
  } else {
    for (const a of accounts) {
      const item = menuItem({ className: 'mail-account-menu-item' });
      item.type = 'button';
      item.setAttribute('role', 'menuitem');
      if (a.id === currentAccount?.id) item.classList.add('is-active');
      const icon = h('span', 'mail-account-menu-icon');
      item.appendChild(icon);
      void setIcon(icon, 'ui/mail', { size: 14 });
      const label = h('span', 'mail-account-menu-label', a.label || a.email);
      const badgeText = a.verified
        ? t('plugin.mail.status.connected', null, 'connected')
        : t('plugin.mail.status.notVerified', null, 'not verified');
      const badge = h('span', `mail-account-row-badge${a.verified ? ' is-ok' : ''}`, badgeText);
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
  const manage = button({ icon: 'ui/settings', label: t('plugin.mail.toolbar.accounts', null, 'Mail accounts'), variant: 'ghost', size: 'sm', onClick: () => { closeAccountMenu(); openSettings(); } });
  manage.classList.add('ui-btn--icon');
  foot.appendChild(manage);
  accountPopup.appendChild(foot);

  accountPopup.classList.remove('hidden');
  placePopover(accountPopup, barAccountBtn);
  accountPopup.focusFirst();
  accountPopupOpen = true;
  renderBar();
  document.addEventListener('pointerdown', onAccountMenuOutside, true);
}

/* ── Rendering: main body ───────────────────────────────────── */

function render() {
  renderBar();
  bodyEl.innerHTML = '';
  if (!configured) {
    renderSetup(bodyEl);
    return;
  }

  const foldersPane = h('div', 'mail-folders');
  foldersEl = foldersPane;

  const listPane = h('div', 'mail-list');
  const search = searchBar({
    placeholder: t('plugin.mail.search.placeholder', null, 'Search cached mail…'),
    label: t('plugin.mail.search.label', null, 'Search mail'), onInput: onSearch,
  });
  searchInputEl = search.input;
  const searchWrap = h('div', 'mail-search');
  searchWrap.appendChild(search);
  listPane.appendChild(searchWrap);
  listEl = h('div', 'mail-list-items');
  listPane.appendChild(listEl);

  const readerPane = h('div', 'mail-reader');
  readerEl = readerPane;
  readerEl.appendChild(emptyState({ title: t('plugin.mail.empty.selectTitle', null, 'Select a message'), body: t('plugin.mail.empty.selectBody', null, 'Pick a message to read it here.') }));
  bodyEl.append(foldersPane, listPane, readerPane);
  renderFolders();
  renderMessages();
}

function renderSetup(root) {
  const wrap = h('div', 'mail-onboarding');
  const intro = h('div', 'mail-help');
  if (!accounts.length) {
    intro.appendChild(h('h3', null, t('plugin.mail.setup.connectTitle', null, 'Connect your mail account')));
    intro.appendChild(h('p', null, t('plugin.mail.setup.connectBody', null, 'Choose your provider and add your account. Provider instructions appear at the bottom of the form and disappear once the account is verified.')));
  } else {
    intro.appendChild(h('h3', null, t('plugin.mail.setup.verifyTitle', null, 'Verify your mail account')));
    intro.appendChild(h('p', null, t('plugin.mail.setup.verifyBody', null, 'Your account is saved but the connection test hasn’t succeeded yet. Press Test connection below, or read the provider instructions at the bottom.')));
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
  form.appendChild(h('h4', null, onboarding
    ? t('plugin.mail.form.addFirst', null, 'Add your first account')
    : t('plugin.mail.form.addAnother', null, 'Add another account')));

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
  form.appendChild(field({ label: t('plugin.mail.form.provider', null, 'Provider'), control: providerSel }));

  const emailInput = input({ type: 'email', placeholder: t('plugin.mail.form.emailPlaceholder', null, 'you@example.com'), autocomplete: 'username' });
  form.appendChild(field({ label: t('plugin.mail.form.email', null, 'Email'), control: emailInput }));

  const userInput = input({ type: 'text', placeholder: t('plugin.mail.form.usernamePlaceholder', null, 'Usually your full email'), autocomplete: 'username' });
  form.appendChild(field({ label: t('plugin.mail.form.username', null, 'Username (optional)'), control: userInput }));

  const passInput = input({ type: 'password', placeholder: t('plugin.mail.form.passwordPlaceholder', null, 'Password or app password'), autocomplete: 'current-password' });
  form.appendChild(field({ label: t('plugin.mail.form.password', null, 'Password'), control: passInput }));

  const imapHost = input({ placeholder: t('plugin.mail.form.imapHostPlaceholder', null, 'imap.example.com') });
  const imapPort = input({ type: 'number', value: '993', placeholder: '993' });
  const imapSec = select({ options: ['ssl', 'starttls', 'none'], value: 'ssl' });
  form.appendChild(field({ label: t('plugin.mail.form.imapServer', null, 'IMAP server'), control: imapHost }));
  const imapRow = h('div', 'ui-row');
  imapRow.style.gap = '8px';
  imapRow.append(field({ label: t('plugin.mail.form.port', null, 'Port'), control: imapPort }), field({ label: t('plugin.mail.form.security', null, 'Security'), control: imapSec }));
  form.appendChild(imapRow);

  const smtpHost = input({ placeholder: t('plugin.mail.form.smtpHostPlaceholder', null, 'smtp.example.com') });
  const smtpPort = input({ type: 'number', value: '465', placeholder: '465' });
  const smtpSec = select({ options: ['ssl', 'starttls', 'none'], value: 'ssl' });
  form.appendChild(field({ label: t('plugin.mail.form.smtpServer', null, 'SMTP server'), control: smtpHost }));
  const smtpRow = h('div', 'ui-row');
  smtpRow.style.gap = '8px';
  smtpRow.append(field({ label: t('plugin.mail.form.port', null, 'Port'), control: smtpPort }), field({ label: t('plugin.mail.form.security', null, 'Security'), control: smtpSec }));
  form.appendChild(smtpRow);

  const actions = h('div', 'mail-compose-actions');
  const statusLine = h('span', 'mail-account-row-email');
  statusLine.style.flex = '1';
  const testBtn = button({ label: t('plugin.mail.form.testConnection', null, 'Test connection'), variant: 'ghost', size: 'sm' });
  testBtn.setAttribute('aria-label', t('plugin.mail.form.testConnection', null, 'Test connection'));
  const addBtn = button({ label: t('plugin.mail.form.addAccount', null, 'Add account'), variant: 'primary', size: 'sm' });
  addBtn.setAttribute('aria-label', t('plugin.mail.form.addAccount', null, 'Add account'));
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
    helpEl.appendChild(document.createTextNode(t(helpText.textKey) + ' '));
    for (const link of helpText.links || []) {
      const a = h('a', null, t(link.labelKey));
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
    statusLine.textContent = t('plugin.mail.status.testing', null, 'Testing…');
    try {
      const r = await testAccount(collect());
      if (r?.ok) {
        statusLine.textContent = t('plugin.mail.status.connectedCheck', null, '✓ Connected');
        toast(t('plugin.mail.toast.connectionWorks', null, 'Connection works — provider configured'), { type: 'info' });
        await refreshStatus();
      } else {
        statusLine.textContent = `✗ ${r?.error || t('plugin.mail.status.connectionFailed', null, 'Connection failed')}`;
        toast(r?.error || t('plugin.mail.status.connectionFailed', null, 'Connection failed'), { type: 'error' });
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
      toast(t('plugin.mail.toast.credentialsRequired', null, 'Email and password are required'), { type: 'error' });
      return;
    }
    addBtn.disabled = true;
    statusLine.textContent = t('plugin.mail.status.adding', null, 'Adding…');
    try {
      const created = await createAccount(payload);
      if (created?.verified) {
        toast(t('plugin.mail.toast.accountConnected', { email: created.email }, 'Account {email} connected'), { type: 'info' });
        await refreshStatus({ keepFolder: true });
      } else {
        statusLine.textContent = t('plugin.mail.status.addedUnverified', { error: created?.last_error || t('plugin.mail.status.testFailed', null, 'test failed') }, 'Added but not verified — {error}');
        toast(t('plugin.mail.toast.accountAddedUnverified', null, 'Account added, but the connection test failed — check the provider help'), { type: 'error' });
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
    listEl.appendChild(emptyState({
      title: t('plugin.mail.empty.noMessages', null, 'No messages'),
      body: t('plugin.mail.empty.nothingIn', { folder: currentFolder }, 'Nothing in {folder}.'),
    }));
    return;
  }
  for (const m of messages) {
    const item = h('button', 'mail-item');
    item.type = 'button';
    if (m.id === selectedMessageId) item.classList.add('is-active');
    if (!m.seen) item.classList.add('is-unread');
    const head = h('div', 'mail-item-head');
    head.appendChild(h('span', 'mail-item-dot'));
    head.appendChild(h('span', 'mail-item-from', m.from || t('plugin.mail.list.unknownSender', null, 'Unknown')));
    head.appendChild(h('span', 'mail-item-time', fmtWhen(m.date)));
    const subject = h('div', 'mail-item-subject', m.subject || t('plugin.mail.list.noSubject', null, '(no subject)'));
    const snippet = h('div', 'mail-item-snippet', m.snippet || '');
    item.append(head, subject, snippet);
    item.addEventListener('click', () => void openMessage(m.id));
    listEl.appendChild(item);
  }
}

function renderMessage(msg) {
  if (!readerEl) return;
  readerEl.textContent = '';
  const subject = h('h2', 'mail-reader-subject', msg.subject || t('plugin.mail.list.noSubject', null, '(no subject)'));
  const from = (msg.from || []).join(', ');
  const to = (msg.to || []).join(', ');
  const fromLine = t('plugin.mail.reader.from', { from: from || '—' }, 'From: {from}');
  const toLine = to ? `\n${t('plugin.mail.reader.to', { to }, 'To: {to}')}` : '';
  const meta = h('p', 'mail-reader-meta', `${fromLine}${toLine}${msg.date ? `\n${fmtWhen(msg.date)}` : ''}`);

  const actions = h('div', 'mail-reader-actions');
  const replyBtn = button({ icon: 'ui/reply', label: t('plugin.mail.reader.reply', null, 'Reply'), variant: 'ghost', size: 'sm', onClick: () => openReply(msg) });
  const fwdBtn = button({ icon: 'ui/forward', label: t('plugin.mail.reader.forward', null, 'Forward'), variant: 'ghost', size: 'sm', onClick: () => openForward(msg) });
  const delBtn = button({ icon: 'ui/trash', label: t('plugin.mail.reader.delete', null, 'Delete'), variant: 'ghost', size: 'sm', onClick: () => deleteCurrent(msg) });
  actions.append(replyBtn, fwdBtn, delBtn);

  readerEl.append(subject, meta, actions);

  const content = h('div', 'mail-reader-content');
  const hasHtml = !!msg.html && msg.html.trim();
  readerFrameEl = null;
  if (hasHtml) {
    const frame = document.createElement('iframe');
    // In the kiosk the frame gets the click bridge (`mailFrameDocument`) so a
    // link opens in the browser plugin instead of loading in this pane; in a
    // normal browser the raw message renders exactly as before.
    const kiosk = isKiosk();
    frame.sandbox = kiosk ? 'allow-scripts' : '';
    frame.srcdoc = kiosk ? mailFrameDocument(msg.html) : msg.html;
    content.appendChild(frame);
    readerFrameEl = frame;
  } else {
    content.appendChild(h('div', 'mail-reader-body', msg.text || t('plugin.mail.reader.emptyBody', null, '(empty message)')));
  }
  readerEl.appendChild(content);

  if (msg.attachments && msg.attachments.length) {
    const atts = h('div', 'mail-attachments');
    for (const att of msg.attachments) {
      const chip = h('span', 'mail-attachment', `${att.filename || t('plugin.mail.reader.attachment', null, 'attachment')} · ${fmtSize(att.size)}`);
      atts.appendChild(chip);
    }
    readerEl.appendChild(atts);
  }
}

/* ── Reader frame links ─────────────────────────────────────── */

/**
 * The click bridge for the reader's HTML frame.
 *
 * The frame cannot reach the app (that is the point of the sandbox), so a link
 * in an HTML mail would otherwise load inside the pane, not in the browser. The
 * email's own script would be the easy bridge, but mail HTML is attacker
 * content: every script tag, inline handler and non-navigable href is stripped
 * first, so the frame still runs nothing but the bridge — and it never gains
 * `allow-same-origin`, `allow-popups`, `allow-forms` or `allow-top-navigation`.
 */
const LINK_BRIDGE_SRC = `document.addEventListener('click', (e) => {
  const a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
  if (!a) return;
  e.preventDefault();
  parent.postMessage({ type: 'mail:open-link', url: a.href }, '*');
}, true);`;

function mailFrameDocument(html) {
  const doc = new DOMParser().parseFromString(String(html ?? ''), 'text/html');
  for (const el of doc.querySelectorAll('script, iframe, object, embed')) el.remove();
  for (const el of doc.querySelectorAll('*')) {
    for (const attr of Array.from(el.attributes)) {
      if (attr.name.toLowerCase().startsWith('on')) el.removeAttribute(attr.name);
    }
  }
  for (const a of doc.querySelectorAll('a[href]')) {
    if (!/^(https?:|mailto:|#|\/)/i.test((a.getAttribute('href') || '').trim())) {
      a.removeAttribute('href');
    }
  }
  const bridge = doc.createElement('script');
  bridge.textContent = LINK_BRIDGE_SRC;
  doc.body.appendChild(bridge);
  return doc.documentElement.outerHTML;
}

/** A link was clicked in the reader's sandboxed frame (see above). */
function onReaderFrameMessage(event) {
  if (!readerFrameEl || event.source !== readerFrameEl.contentWindow) return;
  const data = event.data;
  if (!data || typeof data !== 'object' || data.type !== 'mail:open-link') return;
  const url = externalLinkUrl(String(data.url || ''));
  if (url) void openUrl(url);
}

/* ── Compose ────────────────────────────────────────────────── */

function openCompose(prefill = {}) {
  if (!pickAccount()) {
    toast(t('plugin.mail.toast.addAccountFirst', null, 'Add and verify a mail account first'), { type: 'error' });
    return;
  }
  if (!composeModal) {
    composeTo = input({ placeholder: t('plugin.mail.compose.toPlaceholder', null, 'recipient@example.com') });
    composeCc = input({ placeholder: t('plugin.mail.compose.ccPlaceholder', null, 'cc@example.com') });
    composeBcc = input({ placeholder: t('plugin.mail.compose.bccPlaceholder', null, 'bcc@example.com') });
    composeSubject = input({ placeholder: t('plugin.mail.compose.subject', null, 'Subject') });
    composeBody = textarea({ placeholder: t('plugin.mail.compose.bodyPlaceholder', null, 'Write your message…'), rows: 12 });
    composeBody.classList.add('mail-compose-body');
    composeFrom = h('p', 'mail-account-row-email', t('plugin.mail.compose.from', { email: '' }, 'From: {email}'));
    const actions = h('div', 'mail-compose-actions');
    const sendBtn = button({ icon: 'ui/send', label: t('plugin.mail.compose.send', null, 'Send'), variant: 'primary', size: 'sm' });
    const closeBtn = button({ label: t('plugin.mail.compose.discard', null, 'Discard'), variant: 'ghost', size: 'sm' });
    actions.append(sendBtn, closeBtn);

    composeModal = modal({
      title: t('plugin.mail.compose.title', null, 'New message'),
      wide: true,
      body: [
        composeFrom,
        field({ label: t('plugin.mail.compose.to', null, 'To'), control: composeTo }),
        field({ label: t('plugin.mail.compose.cc', null, 'Cc'), control: composeCc }),
        field({ label: t('plugin.mail.compose.bcc', null, 'Bcc'), control: composeBcc }),
        field({ label: t('plugin.mail.compose.subject', null, 'Subject'), control: composeSubject }),
        field({ label: t('plugin.mail.compose.message', null, 'Message'), control: composeBody }),
        actions,
      ],
    });
    closeBtn.addEventListener('click', () => composeModal.close());

    sendBtn.addEventListener('click', async () => {
      const to = parseAddresses(composeTo.value);
      if (!to.length) {
        toast(t('plugin.mail.toast.recipientRequired', null, 'At least one recipient required'), { type: 'error' });
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
        toast(t('plugin.mail.toast.sent', null, 'Message sent'), { type: 'info' });
        composeModal.close();
      } catch (e) {
        toast(e.message || t('plugin.mail.toast.sendFailed', null, 'Send failed'), { type: 'error' });
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
  composeFrom.textContent = t('plugin.mail.compose.from', { email: pickAccount().email }, 'From: {email}');
  composeModal.open();
}

function openReply(msg) {
  const toEmail = (msg.from_addresses && msg.from_addresses[0] && msg.from_addresses[0].email) || '';
  openCompose({
    to: toEmail,
    subject: t('plugin.mail.compose.replySubject', { subject: msg.subject || '' }, 'Re: {subject}'),
    body: quotedBody(msg),
  });
}

function openForward(msg) {
  openCompose({
    subject: t('plugin.mail.compose.forwardSubject', { subject: msg.subject || '' }, 'Fwd: {subject}'),
    body: quotedBody(msg),
  });
}

async function deleteCurrent(msg) {
  const account = pickAccount();
  if (!account) return;
  const folder = currentFolder;
  if (!await confirmDialog(t('plugin.mail.dialog.delete', { folder: currentFolder }, 'Delete this message from {folder}?'))) return;
  if (!tileEl || folder !== currentFolder || account !== pickAccount()) return;
  try {
    await deleteMessage(account.id, currentFolder, msg.id);
    toast(t('plugin.mail.toast.deleted', null, 'Message deleted'), { type: 'info' });
    currentMessage = null;
    selectedMessageId = null;
    setTileGlow(tileEl, null);
    readerEl.textContent = '';
    readerEl.appendChild(emptyState({ title: t('plugin.mail.empty.selectTitle', null, 'Select a message'), body: t('plugin.mail.empty.selectBody', null, 'Pick a message to read it here.') }));
    await loadMessages();
  } catch (e) {
    toast(e.message || t('plugin.mail.toast.deleteFailed', null, 'Delete failed'), { type: 'error' });
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
    const doneBtn = button({ label: t('plugin.mail.settings.done', null, 'Done'), variant: 'primary', size: 'sm' });
    actions.appendChild(doneBtn);

    settingsModal = modal({
      title: t('plugin.mail.settings.title', null, 'Mail accounts'),
      wide: true,
      body: [list, form, actions],
    });
    doneBtn.addEventListener('click', () => settingsModal.close());
    settingsModal.el.addEventListener('open', () => {
      list.textContent = '';
      if (!accounts.length) {
        list.appendChild(h('p', 'mail-account-row-email', t('plugin.mail.settings.noAccounts', null, 'No accounts yet.')));
      }
      for (const a of accounts) {
        const row = h('div', 'mail-account-row');
        const meta = h('div', 'ui-stack');
        meta.style.flex = '1';
        meta.style.gap = '2px';
        meta.appendChild(h('div', 'mail-account-row-label', a.label || a.email));
        meta.appendChild(h('div', 'mail-account-row-email', a.email));
        const badgeText = a.verified
          ? t('plugin.mail.status.connected', null, 'connected')
          : a.last_error
            ? t('plugin.mail.status.failed', null, 'failed')
            : t('plugin.mail.status.notTested', null, 'not tested');
        const badge = h('span', `mail-account-row-badge${a.verified ? ' is-ok' : ''}`, badgeText);
        const testBtn = button({ label: t('plugin.mail.settings.test', null, 'Test'), variant: 'ghost', size: 'sm' });
        testBtn.title = t('plugin.mail.form.testConnection', null, 'Test connection');
        const delBtn = button({ icon: 'ui/trash', variant: 'ghost', size: 'sm' });
        delBtn.title = t('plugin.mail.settings.removeAccount', null, 'Remove account');
        delBtn.classList.add('ui-btn--icon', 'mail-tool--danger');
        row.append(meta, badge, testBtn, delBtn);
        testBtn.addEventListener('click', async () => {
          testBtn.disabled = true;
          try {
            const r = await testAccount({ id: a.id });
            badge.textContent = r?.ok
              ? t('plugin.mail.status.connected', null, 'connected')
              : (r?.error || t('plugin.mail.status.failed', null, 'failed'));
            badge.classList.toggle('is-ok', !!r?.ok);
            if (r?.ok) toast(t('plugin.mail.toast.emailConnected', { email: a.email }, '{email} connected'), { type: 'info' });
            await refreshStatus({ keepFolder: true });
          } catch (e) {
            toast(e.message, { type: 'error' });
          } finally {
            testBtn.disabled = false;
          }
        });
        delBtn.addEventListener('click', async () => {
          if (!await confirmDialog(t('plugin.mail.dialog.removeAccount', { email: a.email }, 'Remove mail account {email}?'))) return;
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

/** Stable ambient colour pair for a sender address (mail has no images):
 *  the same person always glows the same way, like Gmail letter avatars. */
function senderGlow(addr) {
  const s = String(addr || '').toLowerCase();
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  const a = `hsl(${Math.abs(h) % 360} 70% 55%)`;
  const b = `hsl(${(Math.abs(h) * 7) % 360} 65% 32%)`;
  return glowGradient(a, b, { x: 15, y: 0, x2: 80, y2: 100 });
}

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

/* ── New-mail notifications ────────────────────────────────────
   While the Mail window is open, poll the folders' unread counts and raise a
   notification when new mail lands. */

const MAIL_POLL_MS = 60 * 1000;

let mailPollTimer = null;
let lastUnreadTotal = null;

function totalUnread(folderList) {
  return (folderList || []).reduce((sum, f) => sum + (Number(f.unread) || 0), 0);
}

async function checkNewMail() {
  if (document.hidden) return;
  const account = pickAccount();
  if (!account || !configured) return;
  try {
    const r = await listFolders(account.id);
    const unread = totalUnread(r?.folders);
    if (lastUnreadTotal != null && unread > lastUnreadTotal) {
      const delta = unread - lastUnreadTotal;
      notify({
        app: 'Mail',
        title: t('plugin.mail.notify.title', null, 'New mail'),
        body: tp('plugin.mail.notify.newMail', delta, { where: account.email || currentFolder }),
        icon: 'ui/mail',
        plugin: MAIL_PLUGIN,
        urgency: 'normal',
      });
    }
    lastUnreadTotal = unread;
  } catch (_) { /* transient — next poll retries */ }
}

function startMailPolling() {
  stopMailPolling();
  void checkNewMail();
  mailPollTimer = window.setInterval(() => void checkNewMail(), MAIL_POLL_MS);
}

function stopMailPolling() {
  if (mailPollTimer) window.clearInterval(mailPollTimer);
  mailPollTimer = null;
  lastUnreadTotal = null;
}

/* ── Tile lifecycle ─────────────────────────────────────────── */

/** Create the Mail tile element (the plugin's window container). */
export function mountMailTile() {
  if (tileEl) return tileEl;

  tileEl = document.createElement('section');
  tileEl.className = 'tile mail-tile';
  tileEl.dataset.plugin = MAIL_PLUGIN;

  /* Top bar: account menu + title + status dot */
  const bar = toolbar([], { className: 'mail-bar' });
  bar.dataset.windowBar = '';
  barAccountBtn = menuButton({ icon: 'ui/mail', label: t('plugin.mail.bar.accounts', null, 'Accounts'),
    text: t('plugin.mail.bar.noAccount', null, 'No account'), onClick: toggleAccountMenu,
    className: 'mail-account-btn' });
  barAccountLabel = barAccountBtn.caption;

  const title = h('div', 'tile-bar-spacer');
  statusDot = h('span', 'mail-status-dot');
  statusDot.setAttribute('aria-hidden', 'true');

  // Single top bar (Studio-style): account menu + title + actions + status dot.
  bar.append(
    barAccountBtn,
    title,
    toolbarButton('ui/plus', t('plugin.mail.toolbar.newMessage', null, 'New message'), openCompose),
    toolbarButton('ui/refresh', t('plugin.mail.toolbar.sync', null, 'Sync mail'), () => void syncNow()),
    toolbarButton('ui/settings', t('plugin.mail.toolbar.accounts', null, 'Mail accounts'), openSettings),
    statusDot,
  );
  tileEl.appendChild(bar);

  /* Body */
  bodyEl = h('div', 'mail-body');
  tileEl.appendChild(bodyEl);

  void refreshStatus().finally(() => checkNewMail());
  startMailPolling();
  return tileEl;
}

/** Deactivated: drop the window. */
export function unmountMailTile() {
  stopMailPolling();
  closeAccountMenu();
  composeModal?.close();
  settingsModal?.close();
  window.removeEventListener('message', onReaderFrameMessage);
  tileEl?.remove();
  tileEl = null;
  barAccountBtn = null;
  barAccountLabel = null;
  statusDot = null;
  foldersEl = null;
  listEl = null;
  readerEl = null;
  readerFrameEl = null;
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

  const read = mailActions.find((a) => a.action === 'mail_read' && a.result === 'ok' && a.data?.id);
  const sent = mailActions.some((a) => a.action === 'mail_send' && a.result === 'ok');
  const listed = mailActions.some((a) => a.action === 'mail_list' && a.result === 'ok');
  const searched = mailActions.some((a) => a.action === 'mail_search' && a.result === 'ok');

  if (read) {
    // Open the exact message the AI read/selected in the reader pane.
    void (async () => {
      const { id, folder, account: acctEmail } = read.data || {};
      if (acctEmail && accounts.some((a) => a.email === acctEmail)) {
        currentAccount = accounts.find((a) => a.email === acctEmail);
      }
      if (folder) currentFolder = folder;
      try { await loadMessages(); } catch (_) { /* list load is non-fatal */ }
      await openMessage(id);
    })();
  } else if (sent) {
    toast(t('plugin.mail.toast.sent', null, 'Message sent'), { type: 'info' });
    void refreshStatus({ keepFolder: true });
    // Re-check shortly after: the delivered copy can take a moment to land.
    window.setTimeout(() => void refreshStatus({ keepFolder: true }), 2500);
  } else if (listed || searched) {
    void refreshStatus({ keepFolder: true });
  }
}

let wired = false;
export function wireMailEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('agent:actions', onAgentActions);
  // Links clicked inside the reader's sandboxed frame (the bridge above).
  window.addEventListener('message', onReaderFrameMessage);
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators + window management. */
export function mailContextMenu() {
  const account = pickAccount();
  const message = currentMessage;
  const unread = messages.filter((m) => !m.seen);
  return [
    { type: 'item', label: t('plugin.mail.toolbar.newMessage', null, 'New message'), icon: 'ui/plus', disabled: !account, onClick: () => openCompose() },
    { type: 'item', label: t('plugin.mail.toolbar.sync', null, 'Sync mail'), icon: 'ui/refresh', disabled: !account, onClick: () => void syncNow() },
    {
      type: 'item',
      label: t('plugin.mail.ctx.markAllRead', null, 'Mark all read'),
      icon: 'ui/check',
      disabled: !account || !unread.length,
      onClick: () => {
        const ids = unread.map((m) => m.id);
        void markSeen(account.id, currentFolder, ids, true).then(() => {
          unread.forEach((m) => { m.seen = true; });
          renderMessages();
        }).catch(() => { /* transient — the next refresh reconciles */ });
      },
    },
    { type: 'separator' },
    { type: 'item', label: t('plugin.mail.reader.reply', null, 'Reply'), icon: 'ui/reply', disabled: !message, onClick: () => openReply(message) },
    { type: 'item', label: t('plugin.mail.ctx.deleteMessage', null, 'Delete message'), icon: 'ui/trash', danger: true, disabled: !message, onClick: () => void deleteCurrent(message) },
  ];
}

export default {
  name: 'mail',
  icon: 'ui/mail',
  mount: mountMailTile,
  unmount: unmountMailTile,
  getElement: getMailTileElement,
  wireEvents: wireMailEvents,
  contextMenu: mailContextMenu,
};
