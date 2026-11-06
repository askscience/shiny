/**
 * import-panel.js — "Import from another browser", opened from the Browser's
 * main menu.
 *
 * Two sources (see `plugins/browser/src/import.rs`):
 *
 * - **This computer** — the Firefox- and Chromium-family profiles the server
 *   finds in the home (`GET /api/browser/import/sources`). One click brings a
 *   profile's history and bookmarks (`POST …/import/profile`, by the id the
 *   list gave — never a path).
 * - **A file** — what browsers export: a password CSV (Firefox, Chrome, Edge,
 *   Brave, Safari, Bitwarden, KeePassXC, 1Password) or a bookmarks HTML
 *   (`POST …/import/file` with the file's text). Passwords need the vault
 *   open: a locked vault opens the unlock prompt and the import resumes on
 *   its own once it is open (`vaultUnlocked`).
 */
import { apiFetch } from '../../js/api.js';
import { pickFiles } from '../../js/files.js';
import { t } from '../../js/i18n.js';
import { controlButton } from '../../ui/index.js';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function errorText(err) {
  return String(err?.message || err || '');
}

/**
 * @param {object} deps
 * @param {(className: string) => HTMLElement} deps.popover  the window's popover factory
 * @param {() => void} deps.onLayout  the panel changed size
 * @param {() => void} deps.onImported  bookmarks or history changed: reload them
 * @param {() => void} deps.openPasswords  show the passwords panel (its unlock prompt)
 * @param {typeof apiFetch} [deps.api]
 * @param {typeof pickFiles} [deps.pick]
 */
export function createImportPanel({
  popover, onLayout, onImported, openPasswords, api = apiFetch, pick = pickFiles,
}) {
  const panel = popover('browser-import-panel');
  panel.setAttribute('role', 'dialog');
  const title = t('plugin.browser.import.title', null, 'Import from another browser');
  panel.setAttribute('aria-label', title);

  const head = el('div', 'browser-popover-head');
  head.append(el('span', null, title));
  const body = el('div', 'browser-popover-list browser-import-body');
  const status = el('div', 'browser-pw-status browser-import-status');
  status.setAttribute('role', 'status');
  panel.append(head, body, status);

  /** A password file waiting for the vault to open. */
  let pendingText = null;
  let busy = false;

  function say(message, kind = '') {
    status.textContent = message || '';
    status.dataset.kind = kind;
    onLayout();
  }

  function row(titleText, subText, onClick) {
    const r = controlButton();
    r.type = 'button';
    r.className = 'browser-popover-row';
    const main = el('div', 'browser-popover-main');
    main.append(el('div', 'browser-popover-title', titleText));
    if (subText) main.append(el('div', 'browser-popover-sub', subText));
    r.appendChild(main);
    r.addEventListener('click', () => {
      if (!busy) void onClick(r);
    });
    return r;
  }

  async function run(button, work) {
    busy = true;
    for (const b of panel.querySelectorAll('button')) b.disabled = true;
    say(t('plugin.browser.import.working', null, 'Importing…'));
    try {
      await work();
    } finally {
      busy = false;
      for (const b of panel.querySelectorAll('button')) b.disabled = false;
      button?.focus?.();
    }
  }

  function sourceRow(source) {
    const what = source.history && source.bookmarks
      ? t('plugin.browser.import.historyAndBookmarks', null, 'History and bookmarks')
      : source.history
        ? t('plugin.browser.import.historyOnly', null, 'History')
        : t('plugin.browser.import.bookmarksOnly', null, 'Bookmarks');
    const name = source.profile ? `${source.browser} · ${source.profile}` : source.browser;
    return row(name, what, (button) => run(button, async () => {
      try {
        const res = await api('/api/browser/import/profile', {
          method: 'POST',
          body: JSON.stringify({ id: source.id }),
        });
        const added = res?.data?.added || {};
        say(t('plugin.browser.import.profileDone', {
          bookmarks: added.bookmarks || 0, history: added.history || 0,
        }, `Bookmarks imported: ${added.bookmarks || 0} · history pages: ${added.history || 0}.`));
        onImported();
      } catch (err) {
        say(t('plugin.browser.import.failed', null, 'The import did not work. Try again.'), 'error');
      }
    }));
  }

  async function importText(text) {
    try {
      const res = await api('/api/browser/import/file', {
        method: 'POST',
        body: JSON.stringify({ text }),
      });
      const data = res?.data || {};
      pendingText = null;
      if (data.kind === 'bookmarks') {
        say(t('plugin.browser.import.bookmarksDone', { n: data.added || 0 },
          `Bookmarks imported: ${data.added || 0}.`));
        onImported();
        return;
      }
      const c = data.counts || {};
      const added = c.added || 0;
      const updated = c.updated || 0;
      say([
        t('plugin.browser.import.passwordsDone', { added, updated },
          `Passwords imported: ${added} · updated: ${updated}.`),
        t('plugin.browser.import.deleteCsv', null,
          'Now delete the CSV file: it holds your passwords unencrypted.'),
      ].join(' '));
    } catch (err) {
      const message = errorText(err);
      if (message.includes('vault_locked')) {
        pendingText = text;
        say(t('plugin.browser.import.unlockFirst', null,
          'Unlock your saved passwords: the import continues by itself.'));
        openPasswords();
      } else if (message.includes('not_an_import_file')) {
        say(t('plugin.browser.import.unknownFile', null,
          'This is not a password CSV or a bookmarks HTML file.'), 'error');
      } else {
        say(t('plugin.browser.import.failed', null, 'The import did not work. Try again.'), 'error');
      }
    }
  }

  const fileRow = () => row(
    t('plugin.browser.import.fromFile', null, 'From a file…'),
    t('plugin.browser.import.fromFileHint', null,
      'Passwords: the CSV that Firefox or Chrome exports · Bookmarks: HTML'),
    (button) => run(button, async () => {
      const [file] = (await pick({ accept: '.csv,.html,.htm,text/csv,text/html' })) || [];
      if (!file) {
        say('');
        return;
      }
      await importText(await file.text());
    }),
  );

  async function render() {
    body.replaceChildren(el('div', 'browser-popover-empty', t('plugin.browser.import.looking', null, 'Looking…')));
    let sources = [];
    try {
      const res = await api('/api/browser/import/sources');
      sources = res?.data?.sources || [];
    } catch (_) {
      sources = [];
    }
    const nodes = [el('div', 'browser-popover-heading', t('plugin.browser.import.thisComputer', null, 'On this computer'))];
    if (sources.length) nodes.push(...sources.map(sourceRow));
    else {
      nodes.push(el('div', 'browser-popover-empty',
        t('plugin.browser.import.noneFound', null, 'No other browser found on this computer.')));
    }
    nodes.push(el('div', 'browser-popover-heading', t('plugin.browser.import.file', null, 'File')), fileRow());
    body.replaceChildren(...nodes);
    onLayout();
  }

  /** Show the panel: the list is looked up afresh each time. */
  async function open() {
    if (!busy) say('');
    await render();
  }

  /** The vault was just opened: finish a password import that waited for it. */
  function vaultUnlocked() {
    if (pendingText == null || busy) return;
    const text = pendingText;
    pendingText = null;
    void run(null, () => importText(text));
  }

  return { element: panel, open, vaultUnlocked, hasPending: () => pendingText != null };
}
