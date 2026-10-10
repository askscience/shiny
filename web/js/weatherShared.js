/**
 * weatherShared.js — pure weather helpers for the top-bar chip and its
 * forecast menu (`hudLeft.js`, `weatherMenu.js`).
 *
 * Kept import-free on purpose: `web/js/tests/weatherShared.test.mjs` loads it
 * in Node with no DOM.
 */

/** WMO weather-code groups, as the words the chip and the menu show. */
export const WMO_LABEL = {
  0: 'Clear',
  1: 'Mainly clear',
  2: 'Partly cloudy',
  3: 'Cloudy',
  45: 'Fog',
  48: 'Fog',
  51: 'Drizzle',
  53: 'Drizzle',
  55: 'Drizzle',
  61: 'Rain',
  63: 'Rain',
  65: 'Heavy rain',
  71: 'Snow',
  73: 'Snow',
  75: 'Snow',
  80: 'Showers',
  81: 'Showers',
  82: 'Heavy showers',
  95: 'Thunderstorm',
  96: 'Thunderstorm',
  99: 'Thunderstorm',
};

/** The `insights/weather-*` icon stem for a WMO weather code. */
export function weatherIconStem(code) {
  switch (code) {
    case 0: return 'weather-sun';
    case 1:
    case 2:
    case 3: return 'weather-partly';
    case 45:
    case 48: return 'weather-fog';
    case 51:
    case 53:
    case 55: return 'weather-drizzle';
    case 61:
    case 63:
    case 65:
    case 80:
    case 81:
    case 82: return 'weather-rain';
    case 71:
    case 73:
    case 75:
    case 77: return 'weather-snow';
    case 95:
    case 96:
    case 99: return 'weather-storm';
    default: return 'weather-cloud';
  }
}

/** The code's label, e.g. 61 -> "Rain". */
export function weatherLabel(code) {
  return WMO_LABEL[code] ?? 'Cloudy';
}

/** Whole degrees, or `null` when the API gave nothing usable. */
export function roundTemp(value) {
  if (value == null || value === '') return null;
  const n = Number(value);
  return Number.isFinite(n) ? Math.round(n) : null;
}

/** "21°" for a temperature, "—" when unknown. */
export function tempText(value) {
  const t = roundTemp(value);
  return t == null ? '—' : `${t}°`;
}

/**
 * A forecast row's day name: the first row is the located place's "Today",
 * the rest are short weekday names. `dateIso` is Open-Meteo's "YYYY-MM-DD".
 */
export function forecastDayLabel(dateIso, index, locale = 'en-US') {
  if (index === 0) return 'Today';
  const [y, m, d] = String(dateIso).split('-').map(Number);
  if (!y || !m || !d) return '';
  return new Date(y, m - 1, d).toLocaleDateString(locale, { weekday: 'short' });
}
