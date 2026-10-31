import {
  searchBar, menuSurface, menuItem, confirmDialog, menuButton, element as h,
  toolButton, placePopover, button, emptyState, field, icon, iconButton, input,
  listReveal, modal, notify, select, textarea, toast,
} from '../../ui/index.js';

/**
 * mail.js — the Mail plugin's window (IMAP/SMTP mail client).
 *
 * A mail surface backed by io-email routes (`/api/mail/*`): a folder rail and
 * the message list, plus a reader that opens beside the list when a message is
 * selected, under a window chrome with the account menu and live status. The
 * list opens on a large folder title that hands over to the window bar as it
 * scrolls (a scroll-linked `--sc` 0…1 on the window element); rows cascade in
 * through the shared `listReveal()` motion. A compose modal and an account
 * manager handle sending and configuration; onboarding appears while no
 * provider works yet.
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
let chromeStatusEl = null;
/** The folder name in the window bar — fades in as the large title scrolls away. */
let chromeTitleEl = null;
let barAccountBtn = null;
let barAccountLabel = null;
let bodyEl = null;
let footerEl = null;
let railHeadEl = null;
let foldersEl = null;
let shellEl = null;
let scrollEl = null;
let heroEl = null;
let listEl = null;
let readerEl = null;
/** The reader's HTML-message frame, or null when the message is plain text. */
let readerFrameEl = null;

let accounts = [];
let presets = [];
let configured = false;
let currentAccount = null;
let folders = [];
let currentFolder = 'INBOX';
let messages = [];
let messageTotal = 0;
let selectedMessageId = null;
let currentMessage = null;
let activeQuery = '';
// Bumped per openMessage so a slow fetch for message A can't overwrite the
// reader after the user clicked message B.
let openToken = 0;
let busy = false;
let searchTimer = null;
// Set when fresh rows arrive (a folder loaded, a search answered): the next
// renderMessages() plays the shared list cascade; selection re-renders don't.
let revealRows = false;
let scrollFrame = 0;

/* Account picker popup (body-level). */
let accountPopup = null;
let accountPopupOpen = false;

/* Compose + settings modals. */
let composeModal = null;
let settingsModal = null;
let settingsListEl = null;

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

function currentFolderInfo() {
  return folders.find((folder) => folder.name === currentFolder) || null;
}

function folderIconName(name) {
  if (/^inbox$/i.test(name)) return 'ui/mail';
  if (/^(sent|sent mail|sent items)$/i.test(name)) return 'ui/send';
  if (/^(drafts?|bozze)$/i.test(name)) return 'ui/file';
  if (/^(trash|deleted|bin|cestino)$/i.test(name)) return 'ui/trash';
  if (/^archive$/i.test(name)) return 'ui/archive';
  if (/^(junk|spam|posta indesiderata)$/i.test(name)) return 'ui/warning';
  return 'ui/folder';
}

function folderLabel(name) {
  if (/^inbox$/i.test(name)) return t('plugin.mail.folder.inbox', null, 'Inbox');
  if (/^(sent|sent mail|sent items)$/i.test(name)) return t('plugin.mail.folder.sent', null, 'Sent');
  if (/^(drafts?|bozze)$/i.test(name)) return t('plugin.mail.folder.drafts', null, 'Drafts');
  if (/^(trash|deleted|bin|cestino)$/i.test(name)) return t('plugin.mail.folder.trash', null, 'Trash');
  if (/^archive$/i.test(name)) return t('plugin.mail.folder.archive', null, 'Archive');
  if (/^(junk|posta indesiderata)$/i.test(name)) return t('plugin.mail.folder.junk', null, 'Junk');
  if (/^spam$/i.test(name)) return t('plugin.mail.folder.spam', null, 'Spam');
  return name;
}

function messagesLabel(count) {
  return count
    ? tp('plugin.mail.folder.messages', count, { count })
    : t('plugin.mail.folder.noMessages', null, 'No messages');
}

function unreadLabel(count) {
  return tp('plugin.mail.chrome.unread', count, { count });
}

function initialOf(value) {
  const text = String(value || '').trim();
  const match = text.match(/[\p{L}\p{N}]/u);
  return match ? match[0].toUpperCase() : '?';
}

function statusPill(text, tone = 'neutral', { dot = false, label = false } = {}) {
  const node = h('span', `mail-status-pill mail-status-pill--${tone}${label ? ' mail-status-pill--label' : ''}`);
  if (dot) {
    const dotEl = h('span', 'mail-status-dot');
    if (tone === 'ok') dotEl.classList.add('is-ok');
    if (tone === 'error') dotEl.classList.add('is-error');
    node.appendChild(dotEl);
  }
  node.appendChild(h('span', null, text));
  return node;
}

function detailRow(label, value) {
  const row = h('div', 'mail-detail-row');
  row.append(h('span', 'mail-detail-key', label), h('span', 'mail-detail-val', value));
  return row;
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
    activeQuery = q;
    const account = pickAccount();
    if (!account) return;
    if (!q) {
      await loadMessages();
      return;
    }
    try {
      const r = await searchMessages(account.id, q, currentFolder);
      messages = r?.messages || [];
      messageTotal = messages.length;
      revealRows = true;
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
      selectedMessageId = null;
      messages = [];
      resetReader();
    }
    messageTotal = 0;
    activeQuery = '';
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
  activeQuery = '';
  try {
    const r = await listMessages(account.id, currentFolder);
    messages = r?.messages || [];
    messageTotal = Number(r?.total) || messages.length;
    revealRows = true;
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
  if (!readerEl) return;
  readerEl.textContent = '';
  const loading = h('div', 'mail-reader-empty');
  loading.appendChild(emptyState({ icon: 'ui/mail', title: t('plugin.mail.empty.loading', null, 'Loading…') }));
  readerEl.appendChild(loading);
  try {
    const full = await fetchMessage(account.id, currentFolder, id);
    if (token !== openToken) return; // a newer open won
    currentMessage = full;
    renderMessage(full);
    if (msg && !msg.seen) {
      void markSeen(account.id, currentFolder, [id], true).catch(() => {});
      msg.seen = true;
      const folder = currentFolderInfo();
      if (folder) folder.unread = Math.max(0, (Number(folder.unread) || 0) - 1);
      renderFolders();
      renderMessages();
    }
  } catch (e) {
    if (token !== openToken) return;
    currentMessage = null;
    readerEl.textContent = '';
    const failed = h('div', 'mail-reader-empty');
    failed.appendChild(emptyState({ icon: 'ui/warning', title: t('plugin.mail.empty.openFailed', null, 'Could not open message'), body: e.message }));
    readerEl.appendChild(failed);
  }
}

/* ── Rendering: window chrome ───────────────────────────────── */

function renderChrome() {
  if (!tileEl) return;
  const account = pickAccount();
  if (barAccountLabel) {
    barAccountLabel.textContent = account
      ? (account.label || account.email)
      : t('plugin.mail.bar.noAccount', null, 'No account');
  }
  if (barAccountBtn) barAccountBtn.setAttribute('aria-expanded', String(accountPopupOpen));
  if (chromeTitleEl) chromeTitleEl.textContent = configured ? folderLabel(currentFolder) : '';
  if (!chromeStatusEl) return;

  chromeStatusEl.textContent = '';
  const tone = !account ? 'neutral' : account.verified ? 'ok' : 'error';
  const text = !account
    ? t('plugin.mail.bar.noAccount', null, 'No account')
    : account.verified
      ? t('plugin.mail.status.connected', null, 'connected')
      : t('plugin.mail.status.notVerified', null, 'not verified');
  chromeStatusEl.appendChild(statusPill(text, tone, { dot: !!account, label: true }));

  const unread = Number(currentFolderInfo()?.unread) || 0;
  if (configured && unread > 0) {
    chromeStatusEl.appendChild(statusPill(unreadLabel(unread), 'accent'));
  }
}

function renderRailHead() {
  if (!railHeadEl) return;
  const account = pickAccount();
  railHeadEl.textContent = '';
  railHeadEl.append(
    h('span', 'mail-eyebrow', t('plugin.mail.rail.eyebrow', null, 'Mailbox')),
    h('h2', 'mail-rail-title', account
      ? (account.label || account.email)
      : t('plugin.mail.rail.title', null, 'Mail')),
    h('p', 'mail-rail-sub', account
      ? account.email
      : t('plugin.mail.rail.sub', null, 'No account connected')),
  );
}

/** The large folder title at the top of the list, with its mono count. */
function renderHero() {
  if (!heroEl) return;
  heroEl.textContent = '';
  const counts = [];
  const unread = Number(currentFolderInfo()?.unread) || 0;
  if (unread > 0) counts.push(tp('plugin.mail.hero.new', unread, { count: unread }));
  counts.push(messagesLabel(messageTotal || messages.length));
  heroEl.append(
    h('h2', 'mail-hero-title', folderLabel(currentFolder)),
    h('span', 'mail-hero-count', counts.join(' · ')),
  );
}

/** Scroll-linked handover: `--sc` runs 0…1 over the first 56px of the list,
 *  fading the large title out and the window bar's title + hairline in. */
function syncScroll() {
  scrollFrame = 0;
  if (!tileEl) return;
  const y = configured && scrollEl ? scrollEl.scrollTop : 56;
  tileEl.style.setProperty('--sc', String(Math.min(1, Math.max(0, y / 56)).toFixed(3)));
}

function onListScroll() {
  if (!scrollFrame) scrollFrame = window.requestAnimationFrame(syncScroll);
}

/** The reader only takes room while a message is selected. */
function syncReaderOpen() {
  shellEl?.classList.toggle('has-reader', !!selectedMessageId);
}

function closeReader() {
  selectedMessageId = null;
  openToken += 1;
  resetReader();
  renderMessages();
}

function renderFooter() {
  if (!footerEl) return;
  footerEl.textContent = '';
  const count = messageTotal || messages.length;
  const account = pickAccount();
  footerEl.append(
    h('span', 'mail-footer-step', t(
      'plugin.mail.footer.step',
      { folder: folderLabel(currentFolder), messages: messagesLabel(count) },
      '{folder} · {messages}',
    )),
    h('span', 'mail-footer-note', t('plugin.mail.footer.note', null,
      'Cached locally — sync to check for new mail.')),
  );
  const status = h('span', `mail-footer-status${account?.verified ? ' is-ok' : ''}`);
  status.append(
    h('span', 'mail-footer-dot'),
    h('span', null, !account
      ? t('plugin.mail.bar.noAccount', null, 'No account')
      : account.verified
        ? t('plugin.mail.status.connected', null, 'connected')
        : t('plugin.mail.status.notVerified', null, 'not verified')),
  );
  footerEl.appendChild(status);
}

/* ── Rendering: account menu ────────────────────────────────── */

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
  renderChrome();
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
    accountPopup.appendChild(h('div', 'mail-account-menu-empty', t('plugin.mail.menu.noAccounts', null, 'No accounts yet')));
  } else {
    for (const a of accounts) {
      const item = menuItem({ className: 'mail-account-menu-item' });
      item.type = 'button';
      item.setAttribute('role', 'menuitem');
      if (a.id === currentAccount?.id) item.classList.add('is-active');
      const avatar = h('span', 'mail-account-menu-avatar', initialOf(a.label || a.email));
      const copy = h('span', 'mail-account-menu-copy');
      copy.append(
        h('span', 'mail-account-menu-label', a.label || a.email),
        h('span', 'mail-account-menu-email', a.email),
      );
      const badgeText = a.verified
        ? t('plugin.mail.status.connected', null, 'connected')
        : t('plugin.mail.status.notVerified', null, 'not verified');
      const statusEl = h('span', `mail-account-row-badge${a.verified ? ' is-ok' : ''}`, badgeText);
      item.append(avatar, copy, statusEl);
      item.addEventListener('click', () => {
        closeAccountMenu();
        currentAccount = a;
        selectedMessageId = null;
        currentMessage = null;
        messages = [];
        messageTotal = 0;
        activeQuery = '';
        resetReader();
        renderChrome();
        renderMessages();
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
  renderChrome();
  document.addEventListener('pointerdown', onAccountMenuOutside, true);
}

/* ── Rendering: main surface ────────────────────────────────── */

function render() {
  if (!tileEl || !bodyEl) return;
  renderChrome();
  bodyEl.textContent = '';
  footerEl = h('footer', 'mail-footer');
  shellEl = null;
  scrollEl = null;
  heroEl = null;
  listEl = null;

  if (!configured) {
    renderSetup(bodyEl);
    bodyEl.appendChild(footerEl);
    renderFooter();
    syncScroll();
    return;
  }

  const shell = h('div', 'mail-shell');
  shellEl = shell;

  const rail = h('nav', 'mail-rail');
  railHeadEl = h('div', 'mail-rail-head');
  foldersEl = h('div', 'mail-rail-list');
  const railFoot = h('div', 'mail-rail-foot');
  const manage = button({
    icon: 'ui/settings',
    label: t('plugin.mail.toolbar.accounts', null, 'Mail accounts'),
    variant: 'ghost',
    size: 'sm',
    onClick: openSettings,
  });
  manage.classList.add('mail-rail-manage');
  railFoot.appendChild(withArrow(manage));
  rail.append(railHeadEl, foldersEl, railFoot);

  const stage = h('main', 'mail-stage');
  scrollEl = h('div', 'mail-scroll');
  scrollEl.addEventListener('scroll', onListScroll, { passive: true });
  heroEl = h('header', 'mail-hero');
  const search = searchBar({
    placeholder: t('plugin.mail.search.placeholder', null, 'Search cached mail…'),
    label: t('plugin.mail.search.label', null, 'Search mail'), onInput: onSearch,
  });
  const searchWrap = h('div', 'mail-search');
  searchWrap.appendChild(search);
  listEl = h('div', 'mail-list');
  listEl.setAttribute('role', 'list');
  scrollEl.append(heroEl, searchWrap, listEl);
  stage.appendChild(scrollEl);

  readerEl = h('aside', 'mail-reader');
  shell.append(rail, stage, readerEl);
  bodyEl.append(shell, footerEl);

  if (currentMessage) renderMessage(currentMessage);
  else renderReaderEmpty();

  renderRailHead();
  renderFolders();
  renderMessages();
  renderFooter();
  syncScroll();
}

/** Editorial pill: a trailing arrow that nudges forward on hover. */
function withArrow(btn) {
  btn.classList.add('mail-pill');
  btn.appendChild(h('span', 'mail-pill-arrow', '\u2192'));
  btn.lastChild.setAttribute('aria-hidden', 'true');
  return btn;
}

function renderSetup(root) {
  const wrap = h('div', 'mail-onboarding');
  const inner = h('div', 'mail-onboarding-inner');
  const head = h('header', 'mail-step-head');
  const mark = h('span', 'mail-step-mark');
  mark.appendChild(icon('ui/mail', { size: 22 }));
  const copy = h('div', 'mail-step-copy');
  copy.append(
    h('span', 'mail-step-kicker', t('plugin.mail.setup.kicker', null, 'Setup')),
    h('h2', 'mail-step-title', accounts.length
      ? t('plugin.mail.setup.verifyTitle', null, 'Verify your mail account')
      : t('plugin.mail.setup.connectTitle', null, 'Connect your mail account')),
    h('p', 'mail-step-sub', accounts.length
      ? t('plugin.mail.setup.verifyBody', null, 'Your account is saved but the connection test hasn’t succeeded yet. Press Test connection below, or read the provider instructions at the bottom.')
      : t('plugin.mail.setup.connectBody', null, 'Choose your provider and add your account. Provider instructions appear at the bottom of the form and disappear once the account is verified.')),
  );
  head.append(mark, copy);
  inner.append(head, buildAccountForm({ onboarding: !accounts.length }));
  wrap.appendChild(inner);
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

/** The add-account form: provider, credentials and optional custom servers. */
function buildAccountForm({ onboarding }) {
  const form = h('div', 'mail-form-card');
  form.appendChild(h('h4', 'mail-form-title', onboarding
    ? t('plugin.mail.form.addFirst', null, 'Add your first account')
    : t('plugin.mail.form.addAnother', null, 'Add another account')));

  const providerSel = select({
    options: (presets.length ? presets : defaultPresets()).map((p) => ({ value: p.provider, label: p.label })),
    value: 'gmail',
  });
  const emailInput = input({ type: 'email', placeholder: t('plugin.mail.form.emailPlaceholder', null, 'you@example.com'), autocomplete: 'username' });
  const userInput = input({ type: 'text', placeholder: t('plugin.mail.form.usernamePlaceholder', null, 'Usually your full email'), autocomplete: 'username' });
  const passInput = input({ type: 'password', placeholder: t('plugin.mail.form.passwordPlaceholder', null, 'Password or app password'), autocomplete: 'current-password' });
  const imapHost = input({ placeholder: t('plugin.mail.form.imapHostPlaceholder', null, 'imap.example.com') });
  const imapPort = input({ type: 'number', value: '993', placeholder: '993' });
  const imapSec = select({ options: ['ssl', 'starttls', 'none'], value: 'ssl' });
  const smtpHost = input({ placeholder: t('plugin.mail.form.smtpHostPlaceholder', null, 'smtp.example.com') });
  const smtpPort = input({ type: 'number', value: '465', placeholder: '465' });
  const smtpSec = select({ options: ['ssl', 'starttls', 'none'], value: 'ssl' });

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

  const accountSection = h('section', 'mail-form-section');
  accountSection.appendChild(h('h5', 'mail-form-section-title', t('plugin.mail.form.section.account', null, 'Account')));
  const accountGrid = h('div', 'mail-form-grid');
  accountGrid.append(
    field({ label: t('plugin.mail.form.provider', null, 'Provider'), control: providerSel }),
    field({ label: t('plugin.mail.form.email', null, 'Email'), control: emailInput }),
    field({ label: t('plugin.mail.form.username', null, 'Username (optional)'), control: userInput }),
    field({ label: t('plugin.mail.form.password', null, 'Password'), control: passInput }),
  );
  accountSection.appendChild(accountGrid);
  form.appendChild(accountSection);

  const imapSection = h('section', 'mail-form-section');
  imapSection.appendChild(h('h5', 'mail-form-section-title', t('plugin.mail.form.section.imap', null, 'Incoming mail (IMAP)')));
  const imapGrid = h('div', 'mail-form-grid mail-form-grid--server');
  imapGrid.append(
    field({ label: t('plugin.mail.form.imapServer', null, 'IMAP server'), control: imapHost }),
    field({ label: t('plugin.mail.form.port', null, 'Port'), control: imapPort }),
    field({ label: t('plugin.mail.form.security', null, 'Security'), control: imapSec }),
  );
  imapSection.appendChild(imapGrid);
  form.appendChild(imapSection);

  const smtpSection = h('section', 'mail-form-section');
  smtpSection.appendChild(h('h5', 'mail-form-section-title', t('plugin.mail.form.section.smtp', null, 'Outgoing mail (SMTP)')));
  const smtpGrid = h('div', 'mail-form-grid mail-form-grid--server');
  smtpGrid.append(
    field({ label: t('plugin.mail.form.smtpServer', null, 'SMTP server'), control: smtpHost }),
    field({ label: t('plugin.mail.form.port', null, 'Port'), control: smtpPort }),
    field({ label: t('plugin.mail.form.security', null, 'Security'), control: smtpSec }),
  );
  smtpSection.appendChild(smtpGrid);
  form.appendChild(smtpSection);

  // Provider instructions live at the bottom and change with the selection.
  const helpEl = h('div', 'mail-provider-help');
  const helpIcon = h('span', 'mail-provider-help-icon');
  helpIcon.appendChild(icon('ui/info', { size: 16 }));
  const helpCopy = h('div', 'mail-provider-help-copy');
  helpEl.append(helpIcon, helpCopy);
  form.appendChild(helpEl);

  function updateHelp(provider) {
    const p = presets.find((x) => x.provider === provider) || defaultPresets().find((x) => x.provider === provider);
    const helpText = PROVIDER_HELP[provider] || PROVIDER_HELP.custom;
    helpCopy.textContent = '';
    const label = h('strong', null, (p?.label || provider) + ': ');
    helpCopy.appendChild(label);
    helpCopy.appendChild(document.createTextNode(t(helpText.textKey) + ' '));
    for (const link of helpText.links || []) {
      const a = h('a', null, t(link.labelKey));
      a.href = link.href;
      a.target = '_blank';
      a.rel = 'noopener noreferrer';
      helpCopy.appendChild(a);
      helpCopy.appendChild(document.createTextNode(' '));
    }
  }
  updateHelp(providerSel.select.value);

  const actions = h('div', 'mail-form-actions');
  const statusLine = h('span', 'mail-form-status');
  const testBtn = button({ icon: 'ui/refresh', label: t('plugin.mail.form.testConnection', null, 'Test connection'), variant: 'ghost', size: 'sm' });
  testBtn.setAttribute('aria-label', t('plugin.mail.form.testConnection', null, 'Test connection'));
  const addBtn = withArrow(button({ label: t('plugin.mail.form.addAccount', null, 'Add account'), variant: 'primary', size: 'sm' }));
  addBtn.setAttribute('aria-label', t('plugin.mail.form.addAccount', null, 'Add account'));
  actions.append(statusLine, testBtn, addBtn);
  form.appendChild(actions);

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

/* ── Rendering: folders / messages / reader ─────────────────── */

function renderFolders() {
  if (!foldersEl) return;
  foldersEl.textContent = '';
  const sorted = [...folders].sort((a, b) => {
    const rank = (n) => (/^inbox$/i.test(n) ? 0 : /^sent$/i.test(n) ? 1 : /^drafts$/i.test(n) ? 2 : /^trash$/i.test(n) ? 3 : 4);
    return rank(a.name) - rank(b.name) || a.name.localeCompare(b.name);
  });
  if (!sorted.length) {
    foldersEl.appendChild(h('div', 'mail-rail-empty', t('plugin.mail.folder.none', null, 'No folders')));
  }
  for (const f of sorted) {
    const item = h('button', 'mail-folder');
    item.type = 'button';
    const active = f.name === currentFolder;
    item.classList.toggle('is-active', active);
    if (active) item.setAttribute('aria-current', 'page');
    item.title = `${folderLabel(f.name)} · ${messagesLabel(Number(f.total) || 0)}`;
    const glyph = h('span', 'mail-folder-icon');
    glyph.appendChild(icon(folderIconName(f.name), { size: 15 }));
    item.append(glyph, h('span', 'mail-folder-label', folderLabel(f.name)));
    const unread = Number(f.unread) || 0;
    item.appendChild(h('span', 'mail-folder-count', unread > 0 ? String(unread) : ''));
    item.addEventListener('click', () => {
      if (currentFolder === f.name) return;
      currentFolder = f.name;
      selectedMessageId = null;
      currentMessage = null;
      messages = [];
      messageTotal = 0;
      activeQuery = '';
      resetReader();
      if (scrollEl) scrollEl.scrollTop = 0;
      syncScroll();
      renderFolders();
      renderMessages();
      void loadMessages();
    });
    foldersEl.appendChild(item);
  }
  renderRailHead();
  renderHero();
  renderChrome();
  renderFooter();
}

function renderMessages() {
  if (!listEl) return;
  if (selectedMessageId && !messages.some((m) => m.id === selectedMessageId)) {
    selectedMessageId = null;
    currentMessage = null;
    resetReader();
  }
  listEl.textContent = '';
  if (!messages.length) {
    const body = activeQuery
      ? t('plugin.mail.empty.noSearchResults', { query: activeQuery }, 'No messages match “{query}”.')
      : t('plugin.mail.empty.nothingIn', { folder: folderLabel(currentFolder) }, 'Nothing in {folder}.');
    listEl.appendChild(emptyState({
      icon: 'ui/mail',
      title: t('plugin.mail.empty.noMessages', null, 'No messages'),
      body,
    }));
  } else {
    const rows = [];
    for (const m of messages) {
      const item = h('button', 'mail-row');
      item.type = 'button';
      item.dataset.id = m.id;
      item.setAttribute('role', 'listitem');
      if (m.id === selectedMessageId) {
        item.classList.add('is-active');
        item.setAttribute('aria-current', 'true');
      }
      if (!m.seen) item.classList.add('is-unread');
      const subject = h('span', 'mail-row-subject');
      subject.append(
        h('b', null, m.subject || t('plugin.mail.list.noSubject', null, '(no subject)')),
        h('span', 'mail-row-preview', m.snippet || ''),
      );
      item.append(
        h('span', 'mail-row-dot'),
        h('span', 'mail-row-from', m.from || m.from_addresses?.[0]?.email || t('plugin.mail.list.unknownSender', null, 'Unknown')),
        subject,
        h('span', 'mail-row-time', fmtWhen(m.date)),
      );
      item.addEventListener('click', () => void openMessage(m.id));
      listEl.appendChild(item);
      rows.push(item);
    }
    if (revealRows) void listReveal(rows);
  }
  revealRows = false;
  syncReaderOpen();
  renderHero();
  renderChrome();
  renderFooter();
}

function renderReaderEmpty() {
  if (!readerEl) return;
  readerEl.textContent = '';
  const wrap = h('div', 'mail-reader-empty');
  wrap.appendChild(emptyState({
    icon: 'ui/mail',
    title: t('plugin.mail.empty.selectTitle', null, 'Select a message'),
    body: t('plugin.mail.empty.selectBody', null, 'Pick a message to read it here.'),
  }));
  readerEl.appendChild(wrap);
}

function resetReader() {
  currentMessage = null;
  renderReaderEmpty();
  syncReaderOpen();
}

function renderMessage(msg) {
  if (!readerEl) return;
  readerEl.textContent = '';
  const from = (msg.from || []).join(', ');
  const to = (msg.to || []).join(', ');

  const head = h('header', 'mail-reader-head');
  head.append(
    h('span', 'mail-eyebrow', t('plugin.mail.reader.eyebrow', null, 'Message')),
    h('h2', 'mail-reader-subject', msg.subject || t('plugin.mail.list.noSubject', null, '(no subject)')),
    h('p', 'mail-reader-sub', [from || '—', msg.date ? fmtWhen(msg.date) : ''].filter(Boolean).join(' · ')),
  );

  const actions = h('div', 'mail-reader-actions');
  const replyBtn = button({ icon: 'ui/reply', label: t('plugin.mail.reader.reply', null, 'Reply'), variant: 'ghost', size: 'sm', onClick: () => openReply(msg) });
  const fwdBtn = button({ icon: 'ui/forward', label: t('plugin.mail.reader.forward', null, 'Forward'), variant: 'ghost', size: 'sm', onClick: () => openForward(msg) });
  const delBtn = button({ icon: 'ui/trash', label: t('plugin.mail.reader.delete', null, 'Delete'), variant: 'ghost', size: 'sm', onClick: () => deleteCurrent(msg) });
  const closeBtn = toolButton('ui/close', t('plugin.mail.reader.close', null, 'Close message'), closeReader, { className: 'mail-reader-close' });
  actions.append(withArrow(replyBtn), fwdBtn, delBtn, closeBtn);

  const details = h('div', 'mail-reader-details');
  details.append(
    detailRow(t('plugin.mail.reader.fromLabel', null, 'From'), from || '—'),
    detailRow(t('plugin.mail.reader.toLabel', null, 'To'), to || '—'),
  );
  if (msg.date) details.appendChild(detailRow(t('plugin.mail.reader.dateLabel', null, 'Date'), fmtDate(msg.date)));

  readerEl.append(head, actions, details);

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
    composeFrom = h('span', 'mail-compose-from', t('plugin.mail.compose.from', { email: '' }, 'From: {email}'));

    const addressGrid = h('div', 'mail-compose-grid');
    addressGrid.append(
      field({ label: t('plugin.mail.compose.to', null, 'To'), control: composeTo }),
      field({ label: t('plugin.mail.compose.cc', null, 'Cc'), control: composeCc }),
      field({ label: t('plugin.mail.compose.bcc', null, 'Bcc'), control: composeBcc }),
    );

    const actions = h('div', 'mail-compose-actions');
    const sendBtn = withArrow(button({ label: t('plugin.mail.compose.send', null, 'Send'), variant: 'primary', size: 'sm' }));
    const closeBtn = button({ label: t('plugin.mail.compose.discard', null, 'Discard'), variant: 'ghost', size: 'sm' });
    actions.append(sendBtn, closeBtn);

    composeModal = modal({
      title: t('plugin.mail.compose.title', null, 'New message'),
      wide: true,
      body: [
        composeFrom,
        addressGrid,
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
    resetReader();
    await loadMessages();
  } catch (e) {
    toast(e.message || t('plugin.mail.toast.deleteFailed', null, 'Delete failed'), { type: 'error' });
  }
}

function quotedBody(msg) {
  const from = (msg.from || []).join(', ');
  const date = msg.date ? fmtDate(msg.date) : '';
  const text = (msg.text && msg.text.trim()) ? msg.text.trim() : '';
  const body = text || '(no body)';
  return `\n\n----------\nFrom: ${from}\nDate: ${date}\n\n${body}`;
}

/* ── Settings ───────────────────────────────────────────────── */

function renderAccountList(list) {
  if (!list) return;
  list.textContent = '';
  if (!accounts.length) {
    list.appendChild(h('p', 'mail-accounts-empty', t('plugin.mail.settings.noAccounts', null, 'No accounts yet.')));
    return;
  }
  for (const a of accounts) {
    const row = h('div', 'mail-account-row');
    const iconWrap = h('span', 'mail-account-row-icon');
    iconWrap.appendChild(icon('ui/mail', { size: 16 }));
    const meta = h('div', 'mail-account-row-copy');
    meta.append(
      h('div', 'mail-account-row-label', a.label || a.email),
      h('div', 'mail-account-row-email', a.email),
    );
    const badgeText = a.verified
      ? t('plugin.mail.status.connected', null, 'connected')
      : a.last_error
        ? t('plugin.mail.status.failed', null, 'failed')
        : t('plugin.mail.status.notTested', null, 'not tested');
    const badgeEl = h('span', `mail-account-row-badge${a.verified ? ' is-ok' : ''}`, badgeText);
    const testBtn = button({ icon: 'ui/refresh', label: t('plugin.mail.settings.test', null, 'Test'), variant: 'ghost', size: 'sm' });
    testBtn.title = t('plugin.mail.form.testConnection', null, 'Test connection');
    const delBtn = iconButton({
      icon: 'ui/trash',
      variant: 'ghost',
      size: 'sm',
      label: t('plugin.mail.settings.removeAccount', null, 'Remove account'),
    });
    delBtn.classList.add('plugin-card-remove');
    row.append(iconWrap, meta, badgeEl, testBtn, delBtn);

    testBtn.addEventListener('click', async () => {
      testBtn.disabled = true;
      try {
        const r = await testAccount({ id: a.id });
        badgeEl.textContent = r?.ok
          ? t('plugin.mail.status.connected', null, 'connected')
          : (r?.error || t('plugin.mail.status.failed', null, 'failed'));
        badgeEl.classList.toggle('is-ok', !!r?.ok);
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
        renderAccountList(list);
      } catch (e) {
        toast(e.message, { type: 'error' });
      }
    });
    list.appendChild(row);
  }
}

function openSettings() {
  if (!settingsModal) {
    const body = h('div', 'mail-settings');
    const head = h('header', 'mail-settings-head');
    head.append(
      h('span', 'mail-eyebrow', t('plugin.mail.settings.kicker', null, 'Accounts')),
      h('p', 'mail-settings-sub', t('plugin.mail.settings.subtitle', null, 'Add, test or remove the mailboxes this window uses.')),
    );
    settingsListEl = h('div', 'mail-accounts');
    const add = h('section', 'mail-settings-add');
    add.appendChild(h('h4', 'mail-form-section-title', t('plugin.mail.form.addAnother', null, 'Add another account')));
    add.appendChild(buildAccountForm({ onboarding: false }));
    const actions = h('div', 'mail-settings-actions');
    const doneBtn = withArrow(button({ label: t('plugin.mail.settings.done', null, 'Done'), variant: 'primary', size: 'sm' }));
    actions.appendChild(doneBtn);

    body.append(head, settingsListEl, add, actions);
    settingsModal = modal({
      title: t('plugin.mail.settings.title', null, 'Mail accounts'),
      wide: true,
      body: [body],
    });
    doneBtn.addEventListener('click', () => settingsModal.close());
  }
  renderAccountList(settingsListEl);
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

function fmtDate(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? String(iso) : d.toLocaleString();
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
  tileEl.style.setProperty('--sc', '1');

  /* Window chrome: account menu, live status and window actions. */
  const chrome = h('div', 'mail-chrome');
  chrome.dataset.windowBar = '';

  barAccountBtn = menuButton({
    label: t('plugin.mail.bar.accounts', null, 'Accounts'),
    text: t('plugin.mail.bar.noAccount', null, 'No account'), onClick: toggleAccountMenu,
    className: 'mail-account-btn',
  });
  barAccountLabel = barAccountBtn.caption;
  chromeStatusEl = h('span', 'mail-chrome-status');
  chromeTitleEl = h('span', 'mail-chrome-title');
  chromeTitleEl.setAttribute('aria-hidden', 'true');

  chrome.append(
    barAccountBtn,
    chromeTitleEl,
    chromeStatusEl,
    toolButton('ui/plus', t('plugin.mail.toolbar.newMessage', null, 'New message'), openCompose),
    toolButton('ui/refresh', t('plugin.mail.toolbar.sync', null, 'Sync mail'), () => void syncNow()),
    toolButton('ui/settings', t('plugin.mail.toolbar.accounts', null, 'Mail accounts'), openSettings),
  );
  tileEl.appendChild(chrome);

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
  chromeStatusEl = null;
  chromeTitleEl = null;
  if (scrollFrame) window.cancelAnimationFrame(scrollFrame);
  scrollFrame = 0;
  barAccountBtn = null;
  barAccountLabel = null;
  bodyEl = null;
  footerEl = null;
  railHeadEl = null;
  foldersEl = null;
  shellEl = null;
  scrollEl = null;
  heroEl = null;
  listEl = null;
  readerEl = null;
  readerFrameEl = null;
  accounts = [];
  folders = [];
  messages = [];
  currentAccount = null;
  configured = false;
  selectedMessageId = null;
  currentMessage = null;
  messageTotal = 0;
  activeQuery = '';
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
