/**
 * batteryShared.js — pure helpers for the top-bar battery chip (`hudBattery.js`).
 *
 * Kept import-free on purpose: `web/js/tests/batteryShared.test.mjs` loads it in
 * Node with no DOM. The server computes a coarse `level` (0–4) too; the two use
 * the same thresholds so a stray `percent` can never disagree with the glyph.
 */

/** Map a percentage to the chip's 0–4 icon bucket (mirrors the Rust service). */
export function batteryLevel(percent) {
  if (percent == null || Number.isNaN(Number(percent))) return null;
  const p = Math.max(0, Math.min(100, Number(percent)));
  if (p <= 10) return 0;
  if (p <= 30) return 1;
  if (p <= 60) return 2;
  if (p <= 85) return 3;
  return 4;
}

/**
 * Which chip icon to draw. A charging pack always gets the bolt so it reads at
 * a glance, whatever the level. When the kernel gives no percentage we fall
 * back to the plain full outline (the chip still shows the state label).
 */
export function batteryIconName(status) {
  if (status && status.state === 'charging') return 'hud/battery-charging';
  const level = status?.level ?? batteryLevel(status?.percent);
  return level == null ? 'hud/battery-4' : `hud/battery-${level}`;
}

/** The chip's short label — the charging state, else a neutral "Battery". */
export function chipLabel(status) {
  if (!status || !status.available) return 'Battery';
  switch (status.state) {
    case 'charging': return 'Charging';
    case 'full': return 'Full';
    case 'discharging': return 'On battery';
    case 'idle': return 'Plugged in';
    default: return 'Battery';
  }
}

/** The percentage text, or '' when unknown. */
export function chipPercent(status) {
  return status && status.percent != null ? `${status.percent}%` : '';
}

/**
 * The chip's state class: charging, low (on battery and nearly empty, so it
 * warns), dim (plugged in and full/idle, nothing to shout about) or none.
 */
export function chipStateClass(status) {
  if (!status || !status.available) return '';
  if (status.state === 'charging') return 'is-charging';
  if (status.state === 'discharging') {
    const level = status.level ?? batteryLevel(status.percent);
    return level != null && level <= 1 ? 'is-low' : '';
  }
  if (status.state === 'full' || status.state === 'idle') return 'is-dim';
  return '';
}

/** True when the pack is low or critical and not charging. */
export function isLow(status) {
  return chipStateClass(status) === 'is-low';
}

/** Compact "2 h 5 min" style duration from seconds. */
export function formatDuration(seconds) {
  if (seconds == null || !Number.isFinite(Number(seconds)) || seconds <= 0) return '';
  const mins = Math.round(Number(seconds) / 60);
  if (mins < 1) return '<1 min';
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  if (h === 0) return `${m} min`;
  if (m === 0) return `${h} h`;
  return `${h} h ${m} min`;
}

/** A "2 h 5 min left" / "1 h 10 min to full" line, or '' when unknown. */
export function timeLabel(status) {
  if (!status || !status.available) return '';
  if (status.state === 'discharging') {
    const left = formatDuration(status.time_to_empty_secs);
    return left ? `${left} left` : '';
  }
  if (status.state === 'charging') {
    const full = formatDuration(status.time_to_full_secs);
    return full ? `${full} to full` : '';
  }
  return '';
}
