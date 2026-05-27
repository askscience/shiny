import { setOrbTheme } from './orbCanvas.js';

const STORAGE_KEY = 'ui.theme';

function updateToggleIcons(theme) {
  const btn = document.getElementById('theme-toggle-btn');
  if (!btn) return;
  const sun = btn.querySelector('.icon-sun');
  const moon = btn.querySelector('.icon-moon');
  const isLight = theme === 'light';
  sun?.classList.toggle('hidden', isLight);
  moon?.classList.toggle('hidden', !isLight);
  btn.title = isLight ? 'Dark mode' : 'Light mode';
  btn.setAttribute('aria-label', isLight ? 'Switch to dark mode' : 'Switch to light mode');
}

export function applyTheme(theme) {
  const next = theme === 'light' ? 'light' : 'dark';
  document.documentElement.dataset.theme = next;
  updateToggleIcons(next);
  setOrbTheme(next);
  window.dispatchEvent(new CustomEvent('theme:change', { detail: { theme: next } }));
}

export function initTheme() {
  const saved = localStorage.getItem(STORAGE_KEY);
  applyTheme(saved === 'light' ? 'light' : 'dark');

  document.getElementById('theme-toggle-btn')?.addEventListener('click', () => {
    const next = document.documentElement.dataset.theme === 'light' ? 'dark' : 'light';
    localStorage.setItem(STORAGE_KEY, next);
    applyTheme(next);
  });
}
