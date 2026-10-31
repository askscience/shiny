/**
 * spokenReply.js — the caption of a spoken answer, word by word.
 *
 * While the assistant speaks, its reply is shown in #reply-text (centred above
 * the bar) with every word dim, and each word lights up as it is said. The TTS
 * engine returns one audio clip without word timings, so the timing is an
 * estimate: the clip's progress (currentTime / duration) mapped onto the words
 * weighted by their length, which tracks speech closely enough for a caption.
 *
 * playback.js announces `voice:speaking { speaking, text, audio }`; nothing else
 * is needed. The caption lingers fully lit for a moment after the voice stops.
 * The error banner agent.js writes into the same element is left alone — a new
 * caption simply replaces it.
 */

const LINGER_MS = 2600;
/** Speech rate used until the clip's duration is known (characters/second). */
const FALLBACK_CPS = 15;
/** Light a word a touch before it is heard; perception lags the audio. */
const LEAD = 0.02;

let words = [];
let weights = [];
let total = 0;
let raf = 0;
let hideTimer = 0;
let startedAt = 0;

const reducedMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

/** Plain text for display: markdown marks and link syntax dropped. */
function plain(text) {
  return String(text || '')
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/[*_`#>~]+/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}

function render(el, text) {
  el.textContent = '';
  el.classList.remove('is-error');
  words = [];
  weights = [];
  plain(text).split(' ').forEach((word, i) => {
    if (i) el.appendChild(document.createTextNode(' '));
    const span = document.createElement('span');
    span.className = 'reply-word';
    span.textContent = word;
    el.appendChild(span);
    words.push(span);
    weights.push(word.length + 1);
  });
  total = weights.reduce((a, b) => a + b, 0) || 1;
}

function light(progress) {
  let acc = 0;
  const reach = (progress + LEAD) * total;
  for (let i = 0; i < words.length; i += 1) {
    acc += weights[i];
    words[i].classList.toggle('is-said', acc - weights[i] * 0.5 <= reach);
  }
}

function follow(audio) {
  cancelAnimationFrame(raf);
  const tick = () => {
    const duration = audio && Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : 0;
    const progress = duration
      ? audio.currentTime / duration
      : ((performance.now() - startedAt) / 1000) * FALLBACK_CPS / total;
    light(Math.min(1, progress));
    if (progress < 1) raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);
}

function onSpeaking(event) {
  const el = document.getElementById('reply-text');
  if (!el) return;
  const { speaking, text, audio } = event.detail || {};
  clearTimeout(hideTimer);
  if (speaking) {
    if (!plain(text)) return;
    render(el, text);
    el.classList.remove('hidden');
    startedAt = performance.now();
    if (reducedMotion()) light(1);
    else follow(audio);
    return;
  }
  cancelAnimationFrame(raf);
  if (!words.length || !words[0].isConnected) return;
  light(1);
  hideTimer = setTimeout(() => {
    el.classList.add('hidden');
    words = [];
  }, LINGER_MS);
}

export function initSpokenReply() {
  window.addEventListener('voice:speaking', onSpeaking);
}
