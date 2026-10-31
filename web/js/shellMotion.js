/**
 * shellMotion.js — the shell's part of the motion language.
 *
 * - publishes the spring tokens (motionStyle.js) once at boot;
 * - the top bar's theme button: one tap switches to the next theme through
 *   the language's view transition, opening from the button itself;
 * - depth: the accent aura element behind the voice states, and the pointer
 *   position the wallpaper and the greeting drift with (/ui/motion.css).
 *
 * The motion language itself is a preference (Settings → Appearance →
 * Motion, `desktop.surface.motion`), applied by preferences.js.
 */
import { installMotionTokens, motionStyle, switchTheme, nextTheme } from '../ui/index.js';
import { t } from './i18n.js';

function initThemeButton() {
  const btn = document.getElementById('hud-theme');
  if (!btn) return;
  let busy = false;
  btn.addEventListener('click', async () => {
    if (busy) return;
    busy = true;
    try {
      await switchTheme(await nextTheme(), { origin: btn });
    } finally {
      busy = false;
    }
  });
  const label = () => {
    const text = t('hud.theme', null, 'Switch theme');
    btn.title = text;
    btn.setAttribute('aria-label', text);
  };
  label();
  window.addEventListener('locale:change', label);
}

function initAura() {
  if (document.getElementById('motion-aura')) return;
  const aura = document.createElement('div');
  aura.id = 'motion-aura';
  aura.setAttribute('aria-hidden', 'true');
  aura.append(document.createElement('span'), document.createElement('span'));
  const background = document.getElementById('background');
  if (background?.parentNode) background.after(aura);
  else document.body.prepend(aura);
}

/** Depth only: where the pointer is, −1…1 on each axis, for the parallax. */
function initPointerDepth() {
  const root = document.documentElement;
  let frame = 0;
  window.addEventListener('pointermove', (event) => {
    if (motionStyle() !== 'depth' || frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      root.style.setProperty('--pointer-x', ((event.clientX / window.innerWidth) * 2 - 1).toFixed(3));
      root.style.setProperty('--pointer-y', ((event.clientY / window.innerHeight) * 2 - 1).toFixed(3));
    });
  }, { passive: true });
}

export function initShellMotion() {
  installMotionTokens();
  initThemeButton();
  initAura();
  initPointerDepth();
}
