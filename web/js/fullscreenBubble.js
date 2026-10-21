import { controlButton } from '../ui/components/button.js';
/**
 * fullscreenBubble.js — the top-bar bubble for a fullscreen plugin window.
 *
 * A fullscreen window gives up its own title-bar stripe (the app name plus
 * close / exit-fullscreen). Those controls move here: a glass bubble on the
 * left of the top bar, the same bubble as the workspace switcher. It rides the
 * top-edge reveal of the bar itself (fullscreen.css), so the way out of
 * fullscreen appears with the rest of the chrome instead of a strip pinned
 * over the app.
 *
 * fullscreen.js owns the state and announces every change on
 * `fullscreen:change`; this module only mirrors that state onto the bubble and
 * routes the buttons to the same actions as the window's own controls.
 */
import { isCoreWindow, coreWindowTitle } from './coreWindows.js';
import { closeCoreWindow, closePluginWindow } from './tiles.js';
import { fullscreenWindow, exitWindowFullscreen } from './fullscreen.js';
import { icon } from '../ui/index.js';
import { t } from './i18n.js';

let bubble = null;
let initialized = false;

function pluginLabel(name) {
  return isCoreWindow(name) ? coreWindowTitle(name) : name.charAt(0).toUpperCase() + name.slice(1);
}

function makeButton(className, iconName, label) {
  const btn = controlButton();
  btn.type = 'button';
  btn.className = `fullscreen-bubble-btn ${className}`;
  btn.title = label;
  btn.setAttribute('aria-label', label);
  btn.appendChild(icon(iconName, { size: 14 }));
  return btn;
}

/** Build the bubble for `name`, replacing whatever it held before. */
function build(name) {
  const core = isCoreWindow(name);
  bubble.textContent = '';

  // Minimal chrome: just the two controls (close and exit fullscreen) — no
  // app icon, so the left of the bar stays uncluttered. Close closes the
  // window; it does not deactivate the plugin.
  const shown = pluginLabel(name);
  const close = makeButton(
    'fullscreen-bubble-btn--close',
    'ui/close',
    t('desktop.closeWindow', { name: shown }, `Close ${shown}`),
  );
  close.addEventListener('click', (e) => {
    e.stopPropagation();
    if (core) {
      closeCoreWindow(name); // handles leaving fullscreen itself
      return;
    }
    // closePluginWindow hands the desktop its workspace back before the window
    // goes away, so a closed fullscreen app never leaves a sealed empty one.
    closePluginWindow(name);
  });

  const exit = makeButton('fullscreen-bubble-btn--exit', 'ui/compress',
    t('desktop.exitFullscreenFor', { name: shown }, `Exit full screen ${shown}`));
  exit.addEventListener('click', (e) => {
    e.stopPropagation();
    exitWindowFullscreen();
  });

  bubble.append(close, exit);
  bubble.dataset.plugin = name;
  bubble.classList.remove('hidden');
}

/** Show the bubble for the fullscreen window, or hide it when there is none. */
function render() {
  if (!bubble) return;
  const name = document.body.classList.contains('fs-active') && fullscreenWindow();
  if (!name) {
    bubble.classList.add('hidden');
    bubble.textContent = '';
    delete bubble.dataset.plugin;
    return;
  }
  // A different app, or one coming back after being hidden, needs a rebuild.
  if (bubble.dataset.plugin === name && !bubble.classList.contains('hidden')) return;
  build(name);
}

export function initFullscreenBubble() {
  if (initialized) return;
  initialized = true;
  bubble = document.getElementById('fullscreen-bubble');
  if (!bubble) return;
  window.addEventListener('fullscreen:change', render);
  render();
}
