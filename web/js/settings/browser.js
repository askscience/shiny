import { apiFetch } from '../api.js';
import { openCoreWindow } from '../tiles.js';
import { toast, button, select, toggleRow } from '../../ui/index.js';
import { t } from '../i18n.js';
import { getLinkPlugin, setLinkPlugin } from '../preferences.js';
import { linkCatalog, linkPlugins, resolveLinkPlugin } from '../linkRouter.js';
import { el, heading, settingsGroups } from './controls.js';

/** Build the browser section with resources owned by its current mount. */
export function createBrowserSection({ on, cleanups, resources }) {
  function buildBrowser() {
    const holder = el('div', 'settings-browser');
    holder.appendChild(el('p', 'settings-hint', t('settings.browser.loading', null, 'Loading browser settings…')));

    void (async () => {
      let settings = { search_engine: 'duckduckgo', adblock: true, downloads_dir: '' };
      // The link handler is a core preference, not a browser-plugin setting, and
      // its candidates come from the installed plugins — fetch both alongside.
      const [settingsRes, catalog] = await Promise.all([
        apiFetch('/api/browser/settings').catch(() => null),
        linkCatalog(),
      ]);
      settings = { ...settings, ...(settingsRes?.data || {}) };

      const engine = select({
        value: settings.search_engine,
        options: [
          { value: 'duckduckgo', label: t('settings.browser.engine.duckduckgo', null, 'DuckDuckGo (default)') },
          { value: 'brave', label: t('settings.browser.engine.brave', null, 'Brave Search') },
          { value: 'google', label: t('settings.browser.engine.google', null, 'Google') },
          { value: 'bing', label: t('settings.browser.engine.bing', null, 'Bing') },
        ],
        onChange: (value) => {
          void apiFetch('/api/browser/settings', {
            method: 'POST',
            body: JSON.stringify({ search_engine: value }),
          })
            .then(() => toast(t('settings.browser.engineSaved', null, 'Search engine saved')))
            .catch(() => toast(t('settings.browser.engineSaveFailed', null, 'Could not save the search engine'), { type: 'error' }));
        },
      });

      const adblock = toggleRow({
        label: t('settings.browser.adblock', null, 'Block ads and trackers'),
        hint: t('settings.browser.adblockHint', null, 'The Browser window filters page requests through the shell.'),
        checked: settings.adblock !== false,
        onChange: (checked) => {
          void apiFetch('/api/browser/settings', {
            method: 'POST',
            body: JSON.stringify({ adblock: checked }),
          })
            .then(() => {
              // The Browser window owns the shell bridge: it applies this to the
              // running shell and reloads the open page for the scriptlet half.
              window.dispatchEvent(
                new CustomEvent('settings:change', { detail: { section: 'browser' } }),
              );
            })
            .catch(() => {});
        },
      });

      const open = button({
        label: t('settings.browser.open', null, 'Open Browser'),
        variant: 'ghost',
        onClick: () => openCoreWindow('browser'),
      });
      const clear = button({
        label: t('settings.browser.clearHistory', null, 'Clear history'),
        variant: 'ghost',
        onClick: async () => {
          try {
            await apiFetch('/api/browser/history/clear', { method: 'POST' });
            toast(t('settings.browser.historyCleared', null, 'Browsing history cleared'));
          } catch (_) {
            toast(t('settings.browser.clearFailed', null, 'Could not clear history'), { type: 'error' });
          }
        },
      });
      const actions = el('div', 'settings-inline-actions');
      actions.append(open, clear);

      const storage = el(
        'p',
        'settings-hint',
        settings.downloads_dir
          ? t('settings.browser.downloadsDir', { path: settings.downloads_dir }, `Downloads are saved to ${settings.downloads_dir}`)
          : t('settings.browser.downloadsDefault', null, 'Downloads are saved to your Downloads folder.'),
      );

      // Which plugin opens links: the browser (default) or any installed plugin
      // declaring the `open-urls` capability. The stored choice is shown only
      // while that plugin is installed — the router falls back to the default.
      const label = (name) => name.charAt(0).toUpperCase() + name.slice(1);
      const names = linkPlugins(catalog).map((p) => p.name);
      const fallback = resolveLinkPlugin(catalog);
      const stored = getLinkPlugin();
      const links = names.length
        ? select({
          value: names.includes(stored) ? stored : fallback,
          options: names.map((name) => ({
            value: name,
            label: name === fallback ? t('settings.browser.linkPluginDefault', { name: label(name) }, `${label(name)} (default)`) : label(name),
          })),
          onChange: (value) => {
            setLinkPlugin(value);
            toast(t('settings.browser.linkHandlerSaved', null, 'Link handler saved'));
          },
        })
        : null;
      const linksHint = names.length
        ? t('settings.browser.linksHint', null, 'Links clicked anywhere — chat, mail, documents — open in this plugin.')
        : t('settings.browser.noLinkPlugin', null, 'No plugin that can open links is installed.');

      holder.replaceChildren(...settingsGroups([
        heading(t('settings.browser.search', null, 'Search')),
        engine,
        el('p', 'settings-hint', t('settings.browser.searchHint', null, 'Used when you type words instead of an address. A self-hosted SearXNG (SEARXNG_URL) takes precedence when configured.')),
        heading(t('settings.browser.links', null, 'Links')),
        ...(links ? [links] : []),
        el('p', 'settings-hint', linksHint),
        heading(t('settings.browser.privacy', null, 'Privacy')),
        adblock,
        heading(t('settings.browser.storage', null, 'Storage')),
        storage,
        heading(t('settings.browser.actions', null, 'Actions')),
        actions,
      ]));
    })();

    return [holder];
  }

  /* ── Surface ────────────────────────────────────────────────── */


  return buildBrowser();
}
