/**
 * levelMeter — a peak meter with hold and a latching clip light, the one
 * Studio's mixer uses, for any window that shows audio levels.
 *
 *   const m = levelMeter({ vertical: false });
 *   m.update(peakLinear);          // call once per frame (or per poll)
 *   m.updateDb(-12.5);             // or with dBFS
 *   m.reset();
 *
 * Ballistics: instant attack, about 20 dB/s release per 60 Hz frame, a 1.5 s
 * peak hold, and a clip light that stays on until clicked. The fill is a
 * transform (no layout), so dozens of meters cost nothing to animate.
 */

export const METER_FLOOR_DB = -60;

export function gainToDb(g) {
  return 20 * Math.log10(Math.max(1e-9, g));
}

/** Meter position 0..1 for a linear peak (-60 dBFS … +6 dBFS). */
export function meterFrac(peak) {
  const db = gainToDb(peak);
  if (!Number.isFinite(db) || db <= METER_FLOOR_DB) return 0;
  return Math.min(1, Math.max(0, (db - METER_FLOOR_DB) / (6 - METER_FLOOR_DB)));
}

/**
 * @param {object} o
 * @param {boolean} [o.vertical]
 * @param {boolean} [o.compact]
 * @param {string}  [o.clipLabel]  tooltip of the clip light
 * @param {string}  [o.className]
 */
export function levelMeter({ vertical = true, compact = false, clipLabel = '', className = '' } = {}) {
  const wrap = document.createElement('div');
  wrap.className = `ui-meter${vertical ? '' : ' ui-meter--h'}${compact ? ' ui-meter--compact' : ''}${className ? ` ${className}` : ''}`;
  wrap.setAttribute('role', 'meter');
  wrap.setAttribute('aria-valuemin', String(METER_FLOOR_DB));
  wrap.setAttribute('aria-valuemax', '6');
  const fill = document.createElement('span');
  fill.className = 'ui-meter-fill';
  const hold = document.createElement('span');
  hold.className = 'ui-meter-hold';
  const clip = document.createElement('span');
  clip.className = 'ui-meter-clip';
  if (clipLabel) clip.title = clipLabel;
  clip.addEventListener('click', (e) => { e.stopPropagation(); clip.classList.remove('is-on'); });
  wrap.append(fill, hold, clip);

  const st = { peak: 0, holdV: 0, holdT: 0 };
  wrap.update = (peak, now = performance.now()) => {
    st.peak = peak > st.peak ? peak : st.peak * 0.89;
    if (st.peak >= st.holdV || now - st.holdT > 1500) { st.holdV = st.peak; st.holdT = now; }
    const f = meterFrac(st.peak);
    fill.style.transform = vertical ? `scaleY(${f.toFixed(4)})` : `scaleX(${f.toFixed(4)})`;
    const hf = meterFrac(st.holdV);
    if (vertical) hold.style.bottom = `${(hf * 100).toFixed(2)}%`;
    else hold.style.left = `${(hf * 100).toFixed(2)}%`;
    hold.style.opacity = hf > 0.01 ? '1' : '0';
    if (peak >= 0.999) clip.classList.add('is-on');
    wrap.setAttribute('aria-valuenow', gainToDb(st.peak).toFixed(1));
  };
  wrap.updateDb = (db, now) => wrap.update(10 ** (db / 20), now);
  wrap.reset = () => { st.peak = 0; st.holdV = 0; wrap.update(0); };
  wrap.update(0);
  return wrap;
}
