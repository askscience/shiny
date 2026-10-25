import { logoutSession } from '../api.js';
import { button, select, toast, toggleRow } from '../../ui/index.js';
import { getAvailableLocales, t } from '../i18n.js';
import { flushPreferencesNow, getLinkPlugin, getRemember, setLinkPlugin, setRemember, getUiLocale, setUiLocale } from '../preferences.js';
import { linkCatalog, linkPlugins, resolveLinkPlugin } from '../linkRouter.js';
import { el, field, heading } from './controls.js';

/** Build the system section with resources owned by its current mount. */
export function createSystemSection({ on, cleanups, resources }) {
  function buildSystem() {
    // Interface language. Applied live: the window re-renders itself through its
    // `locale:change` listener, so the new language is visible on the spot.
    const locale = select({
      options: [
        { value: '', label: t('settings.system.interfaceLanguage.auto', null, 'Automatic (system)') },
        ...getAvailableLocales().map((l) => ({ value: l.code, label: l.name || l.code })),
      ],
      value: getUiLocale(),
      onChange: (value) => setUiLocale(value),
    });
    const localeField = field(t('settings.system.interfaceLanguage', null, 'Interface language'), locale, {
      hint: t('settings.system.interfaceLanguage.hint', null, 'Applies to the interface — menus, window chrome and labels — and to the voice (speech recognition and replies) unless a different one is chosen under Voice.'),
    });

    const remember = toggleRow({
      label: t('settings.system.remember', null, 'Remember my workspace'),
      hint: t('settings.system.rememberHint', null, 'Restore your windows, layout and workspaces from last time.'),
      checked: getRemember(),
      onChange: (checked) => setRemember(checked),
    });

    // Which plugin opens links is an app-wide choice, not a browser-plugin
    // setting: list every installed plugin declaring the `open-urls` capability.
    const links = el('div', 'settings-links');
    void (async () => {
      const catalog = await linkCatalog();
      const names = linkPlugins(catalog).map((p) => p.name);
      if (!names.length) {
        links.appendChild(el('p', 'settings-hint', t('settings.system.noLinkPlugin', null, 'No plugin that can open links is installed.')));
        return;
      }
      const fallback = resolveLinkPlugin(catalog);
      const stored = getLinkPlugin();
      const label = (name) => name.charAt(0).toUpperCase() + name.slice(1);
      const picker = select({
        value: names.includes(stored) ? stored : fallback,
        options: names.map((name) => ({
          value: name,
          label: name === fallback
            ? t('settings.system.linkPluginDefault', { name: label(name) }, `${label(name)} (default)`)
            : label(name),
        })),
        onChange: (value) => {
          setLinkPlugin(value);
          toast(t('settings.system.linkHandlerSaved', null, 'Link handler saved'));
        },
      });
      links.appendChild(field(t('settings.system.links', null, 'Links'), picker, {
        hint: t('settings.system.linkHandlerHint', null, 'Links clicked anywhere — chat, mail, documents — open in this plugin.'),
      }));
    })();

    const logoutBtn = button({
      label: t('settings.system.logOut', null, 'Log out'),
      variant: 'danger',
      onClick: async () => {
        try {
          await flushPreferencesNow();
        } catch (_) { /* proceed */ }
        await logoutSession();
        // In the kiosk, hand the seat back to the machine's login screen: the
        // shell exits with its sign-out status and the session supervisor starts
        // the greeter. A plain browser has no bridge and reloads to the app's
        // own login screen.
        if (window.ipc?.postMessage) {
          window.ipc.postMessage('peakd:logout');
          return;
        }
        window.location.href = '/';
      },
    });
    const logoutHint = el('p', 'settings-hint', t('settings.system.logOutHint', null, 'Logging out returns you to the sign-in screen. Your plugins and chats stay as you set them.'));
    const actions = el('div', 'settings-inline-actions');
    actions.appendChild(logoutBtn);

    return [
      localeField,
      remember,
      el('p', 'settings-hint', t('settings.system.rememberBody', null, 'On: your windows, layout and workspaces come back exactly as you left them. Off: each sign-in starts with a clean desktop.')),
      links,
      heading(t('settings.system.session', null, 'Session')),
      actions,
      logoutHint,
    ];
  }

  /* ── Touch Bar ──────────────────────────────────────────────── */


  return buildSystem();
}
