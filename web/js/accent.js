export const DEFAULT_ACCENT = '#4a7fd4';

function hexToRgb(hex) {
  const h = hex.replace('#', '');
  const n = parseInt(h.length === 3 ? h.split('').map((c) => c + c).join('') : h, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function lighten(hex, amount) {
  const [r, g, b] = hexToRgb(hex);
  const mix = (c) => Math.min(255, Math.round(c + (255 - c) * amount));
  return `#${[mix(r), mix(g), mix(b)].map((c) => c.toString(16).padStart(2, '0')).join('')}`;
}

export function applyAccent(hex) {
  const [r, g, b] = hexToRgb(hex);
  const root = document.documentElement;
  const light = lighten(hex, 0.25);
  root.style.setProperty('--accent', hex);
  root.style.setProperty('--accent-soft', `rgba(${r}, ${g}, ${b}, 0.14)`);
  root.style.setProperty('--accent-glow', `rgba(${r}, ${g}, ${b}, 0.35)`);
  root.style.setProperty('--gradient-brand', `linear-gradient(135deg, ${hex} 0%, ${light} 50%, #5eead4 100%)`);
  root.style.setProperty(
    '--gradient-mesh',
    `radial-gradient(ellipse 80% 60% at 20% 10%, rgba(${r}, ${g}, ${b}, 0.18) 0%, transparent 55%), radial-gradient(ellipse 70% 50% at 80% 90%, rgba(94, 234, 212, 0.12) 0%, transparent 50%), radial-gradient(ellipse 50% 40% at 60% 30%, rgba(167, 139, 250, 0.08) 0%, transparent 45%)`,
  );
  window.dispatchEvent(new CustomEvent('accent:change', { detail: { accent: hex } }));
}

export function getStoredAccent() {
  return localStorage.getItem('ui.accent') || DEFAULT_ACCENT;
}

export function initAccent() {
  applyAccent(getStoredAccent());
  window.addEventListener('theme:change', () => {
    applyAccent(getStoredAccent());
  });
}
