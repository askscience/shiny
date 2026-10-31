/**
 * passwords.js — the Browser window's password manager.
 *
 * A popover (one of the window's own) over the server's vault
 * (`plugins/browser/src/vault.rs`): saved logins sealed under the user's login
 * password. The window only ever holds what it is showing; the list carries
 * no passwords, and one is fetched (`/passwords/reveal`) at the moment it is
 * filled, copied or shown.
 *
 * Three views share the panel:
 *
 * - **locked** — the login-password prompt. The first unlock creates the
 *   vault; after a login-password change the server answers
 *   `password_changed` and the prompt asks once for the previous password, so
 *   the vault is re-sealed instead of lost. A forgotten previous password
 *   leaves "Reset", which deletes every saved login.
 * - **list** — logins for the page in front first, then all of them. A login
 *   for this page can be filled into it; any can be copied (kept out of the
 *   clipboard history) or edited.
 * - **edit** — add or change one login, prefilled from the page when the user
 *   asks to save what they typed into it.
 *
 * The page itself is reached only through the callbacks the window passes in
 * (`fillPage`, `readPage`), which run in the active tab's native view.
 */
import { apiFetch } from '../../js/api.js';
import { copyText } from '../../js/clipboard.js';
import { t } from '../../js/i18n.js';
import { confirmDialog, controlButton, icon } from '../../ui/index.js';

/** The answer `apiFetch` rejects with when the vault has locked itself. */
const LOCKED = 'vault_locked';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function button(className, label, onClick) {
  const b = controlButton();
  b.type = 'button';
  b.className = className;
  b.textContent = label;
  b.addEventListener('click', onClick);
  return b;
}

function iconAction(iconName, label, onClick) {
  const b = controlButton();
  b.type = 'button';
  b.className = 'browser-pw-icon';
  b.title = label;
  b.setAttribute('aria-label', label);
  b.appendChild(icon(iconName, { size: 15 }));
  b.addEventListener('click', (e) => {
    e.stopPropagation();
    onClick();
  });
  return b;
}

function field(label, input) {
  const wrap = el('label', 'browser-pw-field');
  wrap.append(el('span', 'browser-pw-label', label), input);
  return wrap;
}

function textInput(type, value = '', autocomplete = 'off') {
  const input = document.createElement('input');
  input.type = type;
  input.className = 'ui-input browser-pw-input';
  input.value = value;
  input.autocomplete = autocomplete;
  input.spellcheck = false;
  input.setAttribute('autocapitalize', 'off');
  return input;
}

function hostOf(origin) {
  try {
    return new URL(origin).host;
  } catch (_) {
    return origin || '';
  }
}

function isLockedError(err) {
  return String(err?.message || err || '').includes(LOCKED);
}

/**
 * Build the panel.
 *
 * @param {object} deps
 * @param {(className: string) => HTMLElement} deps.popover  the window's popover factory
 * @param {() => string} deps.pageUrl  the active tab's URL ('' on the home surface)
 * @param {(username: string, password: string) => Promise<string>} deps.fillPage
 *   fill the active page's login form; resolves to 'ok' | 'user' | 'nofield' | 'origin' | ''
 * @param {() => Promise<{username?: string, password?: string}|null>} deps.readPage
 *   what the user typed into the active page's login form
 * @param {() => void} deps.onLayout  the panel changed size or opened/closed
 * @param {() => void} deps.closePanels  close every popover of the window
 */
export function createPasswordsPanel({ popover, pageUrl, fillPage, readPage, onLayout, closePanels }) {
  const panel = popover('browser-passwords-panel');
  panel.setAttribute('role', 'dialog');
  panel.setAttribute('aria-label', t('plugin.browser.passwords.title', null, 'Passwords'));

  const head = el('div', 'browser-popover-head');
  const title = el('span', null, t('plugin.browser.passwords.title', null, 'Passwords'));
  const actions = el('div', 'browser-pw-actions');
  head.append(title, actions);
  const body = el('div', 'browser-popover-list browser-pw-body');
  const status = el('div', 'browser-pw-status');
  status.setAttribute('role', 'status');
  panel.append(head, body, status);

  let state = { exists: false, unlocked: false };
  let entries = [];
  let matching = new Set();
  let changed = false;

  function say(message, kind = '') {
    status.textContent = message || '';
    status.dataset.kind = kind;
  }

  function setActions(...nodes) {
    actions.replaceChildren(...nodes);
  }

  /* ── Locked ───────────────────────────────────────────────── */

  function renderLocked() {
    setActions();
    const intro = el('p', 'browser-pw-intro', state.exists
      ? t('plugin.browser.passwords.lockedIntro', null,
        'Your saved passwords are locked. Enter your login password to open them.')
      : t('plugin.browser.passwords.createIntro', null,
        'Save the passwords you use on websites, protected by your login password.'));
    const password = textInput('password', '', 'current-password');
    password.placeholder = t('plugin.browser.passwords.loginPassword', null, 'Login password');
    const previous = textInput('password', '', 'off');
    previous.placeholder = t('plugin.browser.passwords.previousPassword', null,
      'Previous login password');
    const previousField = field(
      t('plugin.browser.passwords.previousLabel', null,
        'Your login password changed. Enter the previous one once to keep your saved passwords.'),
      previous,
    );
    previousField.hidden = !changed;

    const form = el('form', 'browser-pw-form');
    const submit = button('ui-btn ui-btn--primary browser-pw-primary',
      state.exists
        ? t('plugin.browser.passwords.unlock', null, 'Unlock')
        : t('plugin.browser.passwords.create', null, 'Turn on'),
      () => {});
    submit.type = 'submit';
    form.append(field(t('plugin.browser.passwords.loginPassword', null, 'Login password'), password),
      previousField, submit);
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      void unlock(password.value, changed ? previous.value : '', submit);
    });

    const reset = button('browser-popover-action browser-pw-reset',
      t('plugin.browser.passwords.reset', null, 'Forgot the previous password? Reset'),
      () => void resetVault(password.value));
    reset.hidden = !changed;

    body.replaceChildren(intro, form, reset);
    setTimeout(() => password.focus(), 0);
  }

  async function unlock(password, previous, submit) {
    if (!password) {
      say(t('plugin.browser.passwords.needPassword', null, 'Enter your login password.'), 'error');
      return;
    }
    submit.disabled = true;
    say(t('plugin.browser.passwords.unlocking', null, 'Unlocking…'));
    try {
      const res = await apiFetch('/api/browser/vault/unlock', {
        method: 'POST',
        body: JSON.stringify(previous
          ? { password, previous_password: previous }
          : { password }),
      });
      const result = res?.data?.status;
      if (result === 'unlocked') {
        changed = false;
        state = { exists: true, unlocked: true };
        say('');
        await loadList();
        return;
      }
      if (result === 'password_changed') {
        changed = true;
        renderLocked();
        say('');
        return;
      }
      say(result === 'too_many_attempts'
        ? t('plugin.browser.passwords.tooMany', null, 'Too many attempts. Try again in a few minutes.')
        : t('plugin.browser.passwords.wrong', null, 'That is not the right password.'), 'error');
    } catch (err) {
      say(t('plugin.browser.passwords.failed', null, 'Something went wrong. Try again.'), 'error');
    } finally {
      submit.disabled = false;
    }
  }

  async function resetVault(password) {
    if (!password) {
      say(t('plugin.browser.passwords.needPassword', null, 'Enter your login password.'), 'error');
      return;
    }
    const sure = await confirmDialog(t('plugin.browser.passwords.resetConfirm', null,
      'Delete every saved password? This cannot be undone.'));
    if (!sure) return;
    try {
      const res = await apiFetch('/api/browser/vault/reset', {
        method: 'POST',
        body: JSON.stringify({ password }),
      });
      if (!res?.data?.reset) {
        say(t('plugin.browser.passwords.wrong', null, 'That is not the right password.'), 'error');
        return;
      }
      changed = false;
      state = { exists: false, unlocked: false };
      renderLocked();
      say(t('plugin.browser.passwords.resetDone', null, 'Saved passwords deleted.'));
    } catch (_) {
      say(t('plugin.browser.passwords.failed', null, 'Something went wrong. Try again.'), 'error');
    }
  }

  /* ── List ─────────────────────────────────────────────────── */

  async function loadList() {
    const url = pageUrl();
    try {
      const all = await apiFetch('/api/browser/passwords');
      entries = all?.data?.entries || [];
      matching = new Set();
      if (url) {
        const here = await apiFetch(`/api/browser/passwords?url=${encodeURIComponent(url)}`);
        for (const e of here?.data?.entries || []) matching.add(e.id);
      }
      renderList();
    } catch (err) {
      if (isLockedError(err)) {
        state.unlocked = false;
        renderLocked();
      } else {
        say(t('plugin.browser.passwords.failed', null, 'Something went wrong. Try again.'), 'error');
      }
    }
  }

  function renderList() {
    const fromPage = button('browser-popover-action',
      t('plugin.browser.passwords.saveFromPage', null, 'Save from page'),
      () => void saveFromPage());
    fromPage.disabled = !pageUrl();
    setActions(
      fromPage,
      button('browser-popover-action', t('plugin.browser.passwords.add', null, 'Add'),
        () => renderEdit(null, { site: pageUrl() })),
      button('browser-popover-action', t('plugin.browser.passwords.lock', null, 'Lock'),
        () => void lockVault()),
    );

    if (!entries.length) {
      body.replaceChildren(el('div', 'browser-popover-empty',
        t('plugin.browser.passwords.empty', null, 'No saved passwords yet')));
      onLayout();
      return;
    }
    const here = entries.filter((e) => matching.has(e.id));
    const rest = entries.filter((e) => !matching.has(e.id));
    const nodes = [];
    if (here.length) {
      nodes.push(el('div', 'browser-popover-heading', t('plugin.browser.passwords.thisSite', null, 'This site')));
      nodes.push(...here.map((e) => row(e, true)));
    }
    if (rest.length) {
      if (here.length) {
        nodes.push(el('div', 'browser-popover-heading', t('plugin.browser.passwords.all', null, 'All passwords')));
      }
      nodes.push(...rest.map((e) => row(e, false)));
    }
    body.replaceChildren(...nodes);
    onLayout();
  }

  function row(entry, canFill) {
    const r = el('div', 'browser-pw-row');
    const main = controlButton();
    main.type = 'button';
    main.className = 'browser-popover-row';
    const text = el('div', 'browser-popover-main');
    text.append(
      el('div', 'browser-popover-title', hostOf(entry.origin)),
      el('div', 'browser-popover-sub',
        entry.username || t('plugin.browser.passwords.noUsername', null, '(no username)')),
    );
    main.appendChild(text);
    main.title = canFill
      ? t('plugin.browser.passwords.fill', null, 'Fill in on this page')
      : t('plugin.browser.passwords.edit', null, 'Edit');
    main.addEventListener('click', () => (canFill ? void fill(entry) : void openEdit(entry)));

    const tools = el('div', 'browser-pw-tools');
    tools.append(
      iconAction('ui/copy', t('plugin.browser.passwords.copy', null, 'Copy password'),
        () => void copyPassword(entry)),
      iconAction('ui/eye', t('plugin.browser.passwords.edit', null, 'Edit'),
        () => void openEdit(entry)),
    );
    r.append(main, tools);
    return r;
  }

  async function reveal(entry) {
    try {
      const res = await apiFetch('/api/browser/passwords/reveal', {
        method: 'POST',
        body: JSON.stringify({ id: entry.id }),
      });
      return res?.data || null;
    } catch (err) {
      if (isLockedError(err)) {
        state.unlocked = false;
        renderLocked();
      } else {
        say(t('plugin.browser.passwords.failed', null, 'Something went wrong. Try again.'), 'error');
      }
      return null;
    }
  }

  async function fill(entry) {
    const secret = await reveal(entry);
    if (!secret) return;
    const result = await fillPage(secret.username || '', secret.password || '');
    if (result === 'ok' || result === 'user') {
      closePanels();
      return;
    }
    say(result === 'origin'
      ? t('plugin.browser.passwords.otherSite', null, 'The page changed to another site.')
      : t('plugin.browser.passwords.noForm', null, 'No login form found on this page.'), 'error');
  }

  async function copyPassword(entry) {
    const secret = await reveal(entry);
    if (!secret?.password) return;
    const ok = await copyText(secret.password, { sensitive: true });
    if (ok) say(t('plugin.browser.passwords.copied', null, 'Password copied.'));
  }

  async function lockVault() {
    try {
      await apiFetch('/api/browser/vault/lock', { method: 'POST', body: '{}' });
    } catch (_) {
      /* the server forgets the key on its own after a while */
    }
    state.unlocked = false;
    entries = [];
    renderLocked();
    say('');
  }

  async function saveFromPage() {
    const typed = await readPage();
    if (!typed?.password) {
      say(t('plugin.browser.passwords.nothingTyped', null,
        'Type your username and password into the page first.'), 'error');
      return;
    }
    renderEdit(null, { site: pageUrl(), username: typed.username || '', password: typed.password });
  }

  /* ── Edit ─────────────────────────────────────────────────── */

  async function openEdit(entry) {
    const secret = await reveal(entry);
    if (!secret) return;
    renderEdit(entry, {
      site: entry.origin,
      username: secret.username || '',
      password: secret.password || '',
      note: entry.note || '',
    });
  }

  function renderEdit(entry, values) {
    setActions();
    const site = textInput('url', values.site || '');
    const username = textInput('text', values.username || '', 'off');
    const password = textInput('password', values.password || '', 'new-password');
    const note = textInput('text', values.note || '');
    const show = iconAction('ui/eye', t('plugin.browser.passwords.show', null, 'Show password'), () => {
      password.type = password.type === 'password' ? 'text' : 'password';
    });
    const passwordRow = el('div', 'browser-pw-inline');
    passwordRow.append(password, show);

    const form = el('form', 'browser-pw-form');
    form.append(
      field(t('plugin.browser.passwords.site', null, 'Website'), site),
      field(t('plugin.browser.passwords.username', null, 'Username'), username),
      field(t('plugin.browser.passwords.password', null, 'Password'), passwordRow),
      field(t('plugin.browser.passwords.note', null, 'Note'), note),
    );
    const buttons = el('div', 'browser-pw-buttons');
    const save = button('ui-btn ui-btn--primary browser-pw-primary',
      t('plugin.browser.passwords.save', null, 'Save'), () => {});
    save.type = 'submit';
    buttons.append(
      button('ui-btn ui-btn--quiet', t('plugin.browser.passwords.cancel', null, 'Cancel'),
        () => { say(''); renderList(); }),
      save,
    );
    if (entry) {
      buttons.prepend(button('ui-btn ui-btn--quiet browser-pw-danger',
        t('plugin.browser.passwords.delete', null, 'Delete'), () => void remove(entry)));
    }
    form.appendChild(buttons);
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      void submitLogin(entry, {
        site: site.value,
        username: username.value,
        password: password.value,
        note: note.value,
      }, save);
    });
    body.replaceChildren(form);
    onLayout();
    setTimeout(() => (values.site ? username : site).focus(), 0);
  }

  async function submitLogin(entry, values, submit) {
    if (!values.site.trim() || !values.password) {
      say(t('plugin.browser.passwords.needSiteAndPassword', null,
        'A website and a password are needed.'), 'error');
      return;
    }
    submit.disabled = true;
    try {
      await apiFetch('/api/browser/passwords/save', {
        method: 'POST',
        body: JSON.stringify({ id: entry?.id, ...values }),
      });
      say(t('plugin.browser.passwords.saved', null, 'Password saved.'));
      await loadList();
    } catch (err) {
      if (isLockedError(err)) {
        state.unlocked = false;
        renderLocked();
      } else {
        say(t('plugin.browser.passwords.saveFailed', null, 'Could not save this password.'), 'error');
      }
    } finally {
      submit.disabled = false;
    }
  }

  async function remove(entry) {
    const sure = await confirmDialog(t('plugin.browser.passwords.deleteConfirm',
      { site: hostOf(entry.origin) }, `Delete the saved password for ${hostOf(entry.origin)}?`));
    if (!sure) return;
    try {
      await apiFetch('/api/browser/passwords/remove', {
        method: 'POST',
        body: JSON.stringify({ id: entry.id }),
      });
      say('');
      await loadList();
    } catch (err) {
      if (isLockedError(err)) {
        state.unlocked = false;
        renderLocked();
      }
    }
  }

  /* ── Public ───────────────────────────────────────────────── */

  /** Show the panel in whatever state the vault is in. */
  async function open() {
    say('');
    try {
      const res = await apiFetch('/api/browser/vault');
      state = { exists: !!res?.data?.exists, unlocked: !!res?.data?.unlocked };
    } catch (_) {
      state = { exists: false, unlocked: false };
    }
    if (state.unlocked) await loadList();
    else renderLocked();
    onLayout();
  }

  return { element: panel, open };
}

/**
 * The page script that fills a login form. Runs in the page (main frame) and
 * returns 'ok' (password filled), 'user' (only a username field: a two-step
 * sign-in), 'nofield', or 'origin' when the page is no longer on `origin` —
 * checked inside the page, so a navigation between the user's click and the
 * fill can never hand the password to another site.
 */
export function fillScript(origin, username, password) {
  return `(() => {
  if (location.origin !== ${JSON.stringify(origin)}) return 'origin';
  ${FIELD_HELPERS}
  const { user, pass } = loginFields();
  const set = (el, value) => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    el.focus();
    setter.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  };
  const username = ${JSON.stringify(username)};
  if (user && username) set(user, username);
  if (!pass) return user ? 'user' : 'nofield';
  set(pass, ${JSON.stringify(password)});
  return 'ok';
})()`;
}

/** The page script that reads what the user typed into a login form, as JSON. */
export function readScript() {
  return `(() => {
  ${FIELD_HELPERS}
  const { user, pass } = loginFields();
  return JSON.stringify({ origin: location.origin, username: user ? user.value : '', password: pass ? pass.value : '' });
})()`;
}

/** Shared by both page scripts: find the visible username and password fields. */
const FIELD_HELPERS = `
  const visible = (el) => !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
  const textual = 'input:not([type]),input[type=text],input[type=email],input[type=tel]';
  const loginFields = () => {
    const pass = [...document.querySelectorAll('input[type=password]')]
      .find((el) => visible(el) && !el.disabled && !el.readOnly) || null;
    const scope = (pass && pass.form) || document;
    const candidates = [...scope.querySelectorAll(textual)].filter((el) => visible(el) && !el.disabled);
    let user = null;
    if (pass) {
      for (const el of candidates) {
        if (el.compareDocumentPosition(pass) & Node.DOCUMENT_POSITION_FOLLOWING) user = el;
      }
    }
    if (!user) {
      user = candidates.find((el) => /user|mail|login|email|account/i
        .test((el.autocomplete || '') + ' ' + (el.name || '') + ' ' + (el.id || ''))) || null;
    }
    return { user, pass };
  };`;
