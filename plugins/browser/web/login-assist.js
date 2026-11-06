/**
 * login-assist.js — what the Browser's password manager does on its own, page
 * by page: offer to save a login the user just submitted, fill a saved login
 * in, and suggest a strong password on a sign-up form.
 *
 * The page is a native view the window cannot see into, so everything goes
 * through page scripts (`passwords.js`) run on the tab's load events:
 *
 * - **load finished** — hand over a login the previous page of the same origin
 *   kept (`takeScript`), watch the new page's form (`watchScript`), then fill
 *   the saved login when there is exactly one for the site and its fields are
 *   empty, or offer a strong password on a sign-up form.
 * - **load started / address change** — the page is being left (a form posted,
 *   a single-page app moved on): take what the watched form kept.
 *
 * A login is offered for the origin the window had loaded, never the one a
 * page claims, and only when it differs from the saved one. Nothing is saved
 * without the user's "Save"; with the vault locked, "Save" opens the unlock
 * prompt and then the editor, prefilled. Incognito tabs fill but never offer
 * to save. "Never for this site" is remembered on this device.
 */
import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';
import { controlButton, icon } from '../../ui/index.js';
import {
  fillScript, generatePassword, suggestScript, takeScript, watchScript,
} from './passwords.js';

const NEVER_KEY = 'browser.passwords.never';
/** A form a single-page app draws after the load gets one more try. */
const REFILL_DELAY_MS = 1500;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/**
 * A button with its icon and label: on a phone the icon stands alone and the
 * label is its name (`ui-btn--labelled`, ui/responsive.css).
 */
function button(className, label, onClick, iconName = '') {
  const b = controlButton();
  b.type = 'button';
  b.className = className;
  if (iconName) {
    b.classList.add('ui-btn--labelled');
    b.setAttribute('aria-label', label);
    b.append(icon(iconName, { size: 16 }), el('span', null, label));
  } else {
    b.textContent = label;
  }
  b.addEventListener('click', onClick);
  return b;
}

function hostOf(origin) {
  try {
    return new URL(origin).host;
  } catch (_) {
    return origin || '';
  }
}

/** The host without `www.`, as the vault matches it. */
function sameHost(origin) {
  return hostOf(origin).toLowerCase().replace(/^www\./, '');
}

function parse(json) {
  try {
    return json ? JSON.parse(json) : null;
  } catch (_) {
    return null;
  }
}

function neverList() {
  try {
    const list = JSON.parse(localStorage.getItem(NEVER_KEY) || '[]');
    return Array.isArray(list) ? list : [];
  } catch (_) {
    return [];
  }
}

function addNever(origin) {
  const list = neverList();
  if (list.includes(origin)) return;
  list.push(origin);
  try {
    localStorage.setItem(NEVER_KEY, JSON.stringify(list));
  } catch (_) {
    /* storage blocked: it is asked again next time */
  }
}

/**
 * @param {object} deps
 * @param {(className: string) => HTMLElement} deps.popover  the window's popover factory
 * @param {(tab: object, script: string) => Promise<string>} deps.runPageJs
 * @param {(tab: object) => string} deps.originOf  the http(s) origin the tab has loaded, or ''
 * @param {(tab: object) => boolean} deps.isActive
 * @param {(values: object|null) => void} deps.openPanel  the passwords panel (prefilled editor, or as is)
 * @param {(shown: boolean) => void} deps.onShow  the offer opened or closed (close the other panels, re-mask)
 * @param {(tab: object) => void} [deps.focusPage]  hand the keyboard back to the page
 * @param {typeof apiFetch} [deps.api]
 */
export function createLoginAssist({
  popover, runPageJs, originOf, isActive, openPanel, onShow, focusPage = () => {}, api = apiFetch,
}) {
  const element = popover('browser-pw-offer');
  element.setAttribute('role', 'dialog');
  element.setAttribute('aria-label', t('plugin.browser.passwords.title', null, 'Passwords'));

  /** The tab the visible offer belongs to. */
  let shownFor = null;
  /** Sites whose locked vault was already pointed out this session. */
  const unlockHinted = new Set();

  async function vaultState() {
    try {
      const res = await api('/api/browser/vault');
      return { exists: !!res?.data?.exists, unlocked: !!res?.data?.unlocked };
    } catch (_) {
      return { exists: false, unlocked: false };
    }
  }

  async function entriesFor(origin) {
    try {
      const res = await api(`/api/browser/passwords?url=${encodeURIComponent(origin)}`);
      return res?.data?.entries || [];
    } catch (_) {
      return null;
    }
  }

  async function reveal(id) {
    try {
      const res = await api('/api/browser/passwords/reveal', {
        method: 'POST',
        body: JSON.stringify({ id }),
      });
      return res?.data || null;
    } catch (_) {
      return null;
    }
  }

  /* ── The offer ─────────────────────────────────────────────── */

  function hide() {
    if (!element.classList.contains('hidden')) {
      element.classList.add('hidden');
      onShow(false);
    }
    shownFor = null;
  }

  /** Show `offer` over `tab` now, or when the user comes back to the tab. */
  function present(tab, offer) {
    tab.loginOffer = offer;
    if (!isActive(tab)) return;
    render(tab, offer);
  }

  function render(tab, offer) {
    const host = hostOf(offer.origin);
    const nodes = [];
    const buttons = el('div', 'browser-pw-buttons');
    const later = button('ui-btn ui-btn--quiet', t('plugin.browser.passwords.notNow', null, 'Not now'), () => {
      tab.loginOffer = null;
      hide();
      focusPage(tab);
    }, 'ui/close');
    let primary;
    if (offer.type === 'save' || offer.type === 'update') {
      nodes.push(el('p', 'browser-pw-intro', offer.type === 'update'
        ? t('plugin.browser.passwords.offerUpdate', { site: host }, `Update the saved password for ${host}?`)
        : t('plugin.browser.passwords.offerSave', { site: host }, `Save the password for ${host}?`)));
      if (offer.login.username) nodes.push(el('div', 'browser-popover-sub', offer.login.username));
      primary = button('ui-btn ui-btn--primary browser-pw-primary', offer.type === 'update'
        ? t('plugin.browser.passwords.update', null, 'Update')
        : t('plugin.browser.passwords.save', null, 'Save'), () => void save(tab, offer), 'ui/check');
      if (offer.type === 'save') {
        buttons.append(button('ui-btn ui-btn--quiet browser-pw-danger', t('plugin.browser.passwords.never', null, 'Never'), () => {
          addNever(offer.origin);
          tab.loginOffer = null;
          hide();
          focusPage(tab);
        }, 'ui/stop'));
      }
    } else if (offer.type === 'suggest') {
      nodes.push(el('p', 'browser-pw-intro', t('plugin.browser.passwords.offerSuggest', null,
        'Use a strong password for this account? It is saved when you sign up.')));
      primary = button('ui-btn ui-btn--primary browser-pw-primary',
        t('plugin.browser.passwords.useStrong', null, 'Use it'), () => void suggest(tab, offer), 'ui/check');
    } else if (offer.type === 'unlock') {
      nodes.push(el('p', 'browser-pw-intro', t('plugin.browser.passwords.offerUnlock', { site: host },
        `Unlock your saved passwords to sign in to ${host}.`)));
      primary = button('ui-btn ui-btn--primary browser-pw-primary',
        t('plugin.browser.passwords.unlock', null, 'Unlock'), () => {
          tab.loginOffer = null;
          hide();
          openPanel(null);
        }, 'ui/lock');
    } else {
      return;
    }
    buttons.append(later, primary);
    const status = el('div', 'browser-pw-status');
    status.setAttribute('role', 'status');
    element.replaceChildren(...nodes, buttons, status);
    shownFor = tab;
    if (element.classList.contains('hidden')) {
      element.classList.remove('hidden');
      onShow(true);
    }
  }

  async function save(tab, offer) {
    tab.loginOffer = null;
    const values = { site: offer.origin, username: offer.login.username || '', password: offer.login.password };
    const state = await vaultState();
    if (!state.unlocked) {
      hide();
      openPanel(values);
      return;
    }
    try {
      await api('/api/browser/passwords/save', {
        method: 'POST',
        body: JSON.stringify(offer.id ? { id: offer.id, ...values } : values),
      });
      hide();
      focusPage(tab);
    } catch (_) {
      // Locked in between, or the save failed: the editor shows why.
      hide();
      openPanel(values);
    }
  }

  async function suggest(tab, offer) {
    tab.loginOffer = null;
    const password = generatePassword();
    const result = await runPageJs(tab, suggestScript(offer.origin, password));
    hide();
    if (result === 'ok') focusPage(tab);
  }

  /* ── Page events ───────────────────────────────────────────── */

  /** A login the page kept: offer it when it is new or changed. */
  async function consider(tab, json, origin) {
    const login = parse(json);
    if (!login?.password || !origin || login.origin !== origin) return;
    if (tab.incognito || neverList().includes(origin)) return;
    const username = String(login.username || '').trim();
    const state = await vaultState();
    if (state.unlocked) {
      const entries = await entriesFor(origin);
      const same = (entries || []).find((e) => e.username === username && sameHost(e.origin) === sameHost(origin));
      if (same) {
        const secret = await reveal(same.id);
        if (secret && secret.password === login.password) return;
        present(tab, { type: 'update', origin, id: same.id, login: { username, password: login.password } });
        return;
      }
    }
    present(tab, { type: 'save', origin, login: { username, password: login.password } });
  }

  /** Fill the site's one saved login into empty fields. */
  async function autofill(tab, origin, retry = true) {
    const state = await vaultState();
    if (!state.unlocked) {
      // A pending "save?" offer stays: it is the more useful of the two.
      if (state.exists && !unlockHinted.has(origin) && !tab.loginOffer) {
        unlockHinted.add(origin);
        present(tab, { type: 'unlock', origin });
      }
      return;
    }
    // Unasked, only the site itself: the list also carries its parent
    // domain's logins (the panel offers those on a click).
    const host = sameHost(origin);
    const entries = (await entriesFor(origin) || []).filter((e) => sameHost(e.origin) === host);
    if (entries.length !== 1) return;
    const secret = await reveal(entries[0].id);
    if (!secret?.password) return;
    const result = await runPageJs(tab, fillScript(origin, secret.username || '', secret.password, { onlyEmpty: true }));
    if (result === 'nofield' && retry) {
      setTimeout(() => {
        if (originOf(tab) === origin) void autofill(tab, origin, false);
      }, REFILL_DELAY_MS);
    }
  }

  /** Watch the page's form; returns its kind. */
  async function watch(tab, origin) {
    const info = parse(await runPageJs(tab, watchScript()));
    tab.loginWatch = { origin, kind: info?.kind || 'none' };
    return tab.loginWatch.kind;
  }

  /** The tab finished loading a page. */
  async function pageLoaded(tab) {
    const origin = originOf(tab);
    if (!origin) {
      tab.loginWatch = null;
      return;
    }
    // A same-origin form post lands here: its login is in sessionStorage.
    await consider(tab, await runPageJs(tab, takeScript()), origin);
    const kind = await watch(tab, origin);
    if (kind === 'login') {
      await autofill(tab, origin);
    } else if (kind === 'signup' && !tab.incognito && !tab.loginOffer) {
      present(tab, { type: 'suggest', origin });
    }
  }

  /** The tab is leaving its page (a load started): take what the form kept. */
  async function pageLeaving(tab) {
    const origin = tab.loginWatch?.origin;
    if (shownFor === tab && tab.loginOffer?.type !== 'save' && tab.loginOffer?.type !== 'update') {
      tab.loginOffer = null;
      hide();
    }
    if (!origin) return;
    await consider(tab, await runPageJs(tab, takeScript()), origin);
  }

  /** The address changed without a load (a single-page app). */
  async function addressChanged(tab) {
    const origin = tab.loginWatch?.origin;
    if (!origin) return;
    await consider(tab, await runPageJs(tab, takeScript()), origin);
    if (originOf(tab) === origin) {
      const kind = await watch(tab, origin);
      if (kind === 'login') await autofill(tab, origin);
    }
  }

  /** The user switched tabs: show that tab's pending offer, if any. */
  function tabActivated(tab) {
    if (tab?.loginOffer) render(tab, tab.loginOffer);
    else if (shownFor && shownFor !== tab) hide();
  }

  function tabClosed(tab) {
    if (shownFor === tab) hide();
  }

  /** The vault was just unlocked: fill the active page. */
  function refill(tab) {
    const origin = tab && originOf(tab);
    if (origin && tab.loginWatch?.kind === 'login') void autofill(tab, origin, false);
  }

  return {
    element, pageLoaded, pageLeaving, addressChanged, tabActivated, tabClosed, refill, hide,
  };
}
