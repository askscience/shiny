/**
 * themeTransition.js — change theme with a view transition in the user's
 * motion language (motion.css draws it):
 *
 *   kinetic  the new theme rises like a curtain while the old one dims
 *   fluid    a circle of the new theme opens from where the change was asked
 *   depth    the old desk recedes into the dark and the new one arrives
 *
 * The snapshot of the new state is taken only once the theme's stylesheets
 * have loaded, so the transition never reveals a half-styled page. Without
 * the View Transitions API, or with reduced motion, the theme just changes.
 */
import { setTheme, listThemes, getActiveTheme } from './theme-loader.js';
import { reducedMotion } from './motion.js';
import { motionStyle } from './motionStyle.js';

const SHEETS = ['theme-tokens', 'theme-components', 'theme-app'];

/** Resolve once each theme <link> has loaded its new sheet (or ~1 s passed). */
function sheetsSettled() {
  const waits = SHEETS.map((id) => document.getElementById(id)).filter(Boolean).map((link) => new Promise((resolve) => {
    const done = () => { clearTimeout(timer); resolve(); };
    const timer = setTimeout(done, 1000);
    link.addEventListener('load', done, { once: true });
    link.addEventListener('error', done, { once: true });
  }));
  return Promise.all(waits);
}

/** The theme after the active one, in the installed order (wrapping). */
export async function nextTheme() {
  const themes = await listThemes();
  const i = themes.indexOf(getActiveTheme());
  return themes[(i + 1) % themes.length] || themes[0];
}

/**
 * Switch to `name`. `origin` (an element or a DOMRect) is where the change
 * was asked from: fluid's circle opens there.
 */
export async function switchTheme(name, { origin = null } = {}) {
  const root = document.documentElement;
  if (!document.startViewTransition || reducedMotion() || root.dataset.vt) return setTheme(name);
  const box = origin?.getBoundingClientRect ? origin.getBoundingClientRect() : origin;
  const x = box ? box.left + box.width / 2 : window.innerWidth / 2;
  const y = box ? box.top + box.height / 2 : window.innerHeight / 2;
  root.style.setProperty('--vt-x', `${((x / window.innerWidth) * 100).toFixed(2)}%`);
  root.style.setProperty('--vt-y', `${((y / window.innerHeight) * 100).toFixed(2)}%`);
  root.dataset.vt = motionStyle();
  let changed = false;
  try {
    const vt = document.startViewTransition(async () => {
      const loaded = sheetsSettled();
      changed = await setTheme(name);
      await loaded;
    });
    await vt.finished;
  } catch {
    // An aborted transition still applied (or will apply) the update.
  } finally {
    delete root.dataset.vt;
  }
  return changed;
}
