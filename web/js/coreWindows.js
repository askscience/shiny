/**
 * coreWindows.js — the built-in windows that live on the desktop next to
 * plugin windows.
 *
 * They behave like plugin surfaces once opened (tile, focus, fullscreen,
 * workspaces, context menu), but they are shipped with the app and are
 * opened/closed instead of activated/deactivated. `load` is lazy so the
 * initial paint never pays for them.
 */

export const CORE_WINDOWS = {
  settings: { title: 'Settings', icon: 'ui/settings', load: () => import('./settingsWindow.js') },
  plugins: { title: 'Plugins', icon: 'ui/puzzle', load: () => import('./pluginsWindow.js') },
};

export function isCoreWindow(name) {
  return Object.prototype.hasOwnProperty.call(CORE_WINDOWS, name);
}

export function coreWindowTitle(name) {
  return CORE_WINDOWS[name]?.title || name;
}

/** A theme icon path for a built-in window (plugins ship their own SVG). */
export function coreWindowIcon(name) {
  return CORE_WINDOWS[name]?.icon || 'ui/puzzle';
}
