/**
 * pluginHud.js — optional top-bar contributions from installed plugins.
 *
 * Unlike a plugin's window surface (`web/plugin.js`, loaded only while the
 * plugin is active), a plugin's `web/hud.js` is loaded for the plugin simply
 * being **installed**. It lets a plugin keep a persistent top-bar chip — e.g.
 * the Updates plugin's update badge next to the battery — even when its window
 * is closed.
 *
 * Contract: `web/hud.js` default-exports `{ install(ctx), uninstall() }`.
 * `install` is called once per page load; `uninstall` when the plugin is
 * uninstalled. The plugin owns its DOM.
 */
import { refreshPluginCatalog } from './activePlugins.js';
import { loadPluginCatalog } from './i18n.js';

const modules = new Map(); // plugin name -> hud module
const hudUrls = new Map(); // plugin name -> the module's URL (for catalog reloads)
let started = false;

async function refresh(force = false) {
  const plugins = await refreshPluginCatalog({ force });
  const installed = new Set(plugins.filter((p) => p.hud).map((p) => p.name));

  // Remove chips for plugins that are gone.
  for (const [name, api] of [...modules]) {
    if (!installed.has(name)) {
      try {
        api.uninstall?.();
      } catch (_) {
        /* a broken uninstall must not block the rest */
      }
      modules.delete(name);
    }
  }

  for (const p of plugins) {
    if (!p.hud || modules.has(p.name)) continue;
    try {
      const moduleUrl = new URL(`../plugins/${p.name}/hud.js`, import.meta.url);
      await loadPluginCatalog(moduleUrl);
      const mod = await import(moduleUrl.href);
      const api = mod.default || mod;
      api.install?.({ plugin: p.name });
      modules.set(p.name, api);
      hudUrls.set(p.name, moduleUrl);
    } catch (err) {
      console.warn(`plugin hud '${p.name}' failed to load`, err);
    }
  }
}

window.addEventListener('locale:change', () => {
  for (const url of hudUrls.values()) void loadPluginCatalog(url);
});

export function initPluginHud() {
  void refresh();
  if (started) return;
  started = true;
  window.addEventListener('plugins:changed', () => void refresh(true));
  window.addEventListener('storage', (e) => {
    if (e.key === 'plugins.changed') void refresh(true);
  });
}
