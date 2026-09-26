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
import { closeCoreWindow, deactivatePlugin } from './tiles.js';
import { fullscreenWindow, exitWindowFullscreen } from './fullscreen.js';
import { icon } from '../ui/index.js';

let bubble = null;
let initialized = false;

function pluginLabel(name) {
  return isCoreWindow(name) ? coreWindowTitle(name) : name.charAt(0).toUpperCase() + name.slice(1);
}

function makeButton(className, iconName, label) {
  const btn = document.createElement('button');
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

  // Minimal chrome: just the two controls (deactivate / close and exit
  // fullscreen) — no app icon, so the left of the bar stays uncluttered.
  const close = makeButton(
    'fullscreen-bubble-btn--close',
    'ui/close',
    core ? `Close ${pluginLabel(name)}` : `Deactivate ${pluginLabel(name)}`,
  );
  close.addEventListener('click', (e) => {
    e.stopPropagation();
    if (core) {
      closeCoreWindow(name); // handles leaving fullscreen itself
      return;
    }
    // Hand the desktop its workspace back before the window goes away, so a
    // closed fullscreen app can never leave a sealed, empty workspace behind.
    if (fullscreenWindow() === name) exitWindowFullscreen();
    void deactivatePlugin(name);
  });

  const exit = makeButton('fullscreen-bubble-btn--exit', 'ui/compress', `Exit full screen ${pluginLabel(name)}`);
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
