/**
 * deskGreeting.js — the desktop's greeting, set as type.
 *
 * "Good / afternoon, / Friday." in three stacked lines on the wallpaper, under
 * every window. Core only writes the words; whether and how it shows is the
 * theme's call (#desk-greeting is `display: none` unless a theme's app.css
 * turns it on), so themes without it pay nothing but one hidden element.
 *
 * The greeting follows the time of day and the interface language, and is
 * refreshed every minute and on `locale:change`.
 */
import { t, getLocale } from './i18n.js';

/** [first hour, key, English]; the small hours still count as evening. */
const PARTS = [
  [5, 'morning', 'Good morning,'],
  [12, 'afternoon', 'Good afternoon,'],
  [18, 'evening', 'Good evening,'],
];

function greetingFor(hour) {
  let part = PARTS[PARTS.length - 1];
  for (const p of PARTS) if (hour >= p[0]) part = p;
  return t(`desk.greeting.${part[1]}`, null, part[2]);
}

/** The lines to set: the greeting split after its first word, then the day. */
export function greetingLines(date = new Date(), locale = 'en') {
  const greeting = greetingFor(date.getHours());
  const cut = greeting.indexOf(' ');
  const head = cut > 0 ? [greeting.slice(0, cut), greeting.slice(cut + 1)] : [greeting];
  let day = date.toLocaleDateString(locale, { weekday: 'long' });
  day = day.charAt(0).toLocaleUpperCase(locale) + day.slice(1);
  return [...head, `${day}.`];
}

function render(el) {
  let locale = 'en';
  try { locale = getLocale() || 'en'; } catch (_) { /* keep English */ }
  const lines = greetingLines(new Date(), locale);
  const key = lines.join('|');
  if (el.dataset.lines === key) return;
  el.dataset.lines = key;
  el.textContent = '';
  lines.forEach((text, i) => {
    const line = document.createElement('span');
    line.className = 'desk-line';
    const inner = document.createElement('span');
    inner.style.setProperty('--i', String(i));
    inner.textContent = text;
    line.appendChild(inner);
    el.appendChild(line);
  });
}

export function initDeskGreeting() {
  const el = document.getElementById('desk-greeting');
  if (!el) return;
  render(el);
  setInterval(() => render(el), 60_000);
  window.addEventListener('locale:change', () => render(el));
}
