/**
 * transcriptGuard.js — keep Whisper's invented text out of the conversation.
 *
 * Decoding silence makes Whisper hallucinate, and because it learned from
 * subtitled video the invention is usually the subtitle track's own
 * boilerplate: "Sottotitoli e revisione a cura di QTSS", "Subtitles by …",
 * "thanks for watching", "like and subscribe", translator credits, a bare URL.
 * It is reported in Italian, English, German, French, Spanish, Portuguese,
 * Russian, Ukrainian, Czech, Romanian, Turkish, Arabic, Chinese, Welsh and the
 * Nordic languages, so the detector keys off language-independent shapes
 * (URLs, domains, markup, credits-by phrasing, outro calls to action) as well
 * as a short list of known phrases. One credit line can also be repeated
 * hundreds of times by a decoder loop; that collapse is handled here too.
 *
 * The faster-whisper sidecar filters this at the source (voice/whisper_server.py
 * mirrors these rules). This module is the second line of defence, so an
 * interrupted result, the Vosk engine, or an older sidecar can never hand
 * invented text to the agent — which is what made the assistant answer the
 * credits and then stop.
 */

const HALLUCINATION_PHRASES = [
  /sottotitol\w*(\s+\w+){0,4}\s+a\s+cura\s+di/,
  /revisione\s+a\s+cura\s+di/,
  /sottotitol\w*\s+(creat|realizz|offert|fornit)\w*/,
  /\bsubs?\s+(by|from)\b/,
  /\bsubtitles?\s+(by|from|created|provided)\b/,
  /\bcaptions?\s+by\b/,
  /\btranslat(ed|ion|or|ions)\s+(by|from)\b/,
  /tradott\w*\s+da/,
  /traduc\w*\s+por/,
  /traduction\s+(par|de)/,
  /untertitel\w*\s+(von|f.r|durch)/,
  /untertitelung\b/,
  /subtitul\w*\s+por/,
  /legendas?\s+por/,
  /amara\s*(org|com)?/,
  /qtss/,
  /like\s+and\s+subscribe/,
  /don\s*t\s+forget\s+to\s+(like|subscribe)/,
  /subscribe\s+to\s+(the|my|our)\s+channel/,
  /thanks?\s+for\s+(watching|viewing)/,
  /grazie\s+per\s+(la\s+)?visione/,
  /danke\s+f.rs?\s+zuschauen/,
  /merci\s+d\s+avoir\s+regard/,
  /gracias\s+por\s+(ver|su\s+visita)/,
  /obrigad\w*\s+por\s+(assistir|ver)/,
  /продолжение\s+следует/,
  /дякую\s+за\s+перегляд/,
  /спасибо\s+за\s+просмотр/,
  /ترجمة/,
  /نانسي\s+قنقر/,
  /info\s+un\s+libro\s+pubblico/,
  /^(the\s+end|silence|blank\s+audio)$/,
];

/** A bare URL or domain is not something a person dictates. */
const URL_RE = /(?:https?:\/\/|www\.)[^\s]+|\b[\w-]+\.(?:com|org|net|info|co|uk|it|de|fr|es|ru|cz|pl|nl|se|dk|no|fi|tr|gr|pt|br|ar|cn|jp|kr)\b/;
/** Markup glyphs glued into transcript runs are decoder noise, not speech. */
const MARKUP_RE = /[♪♫#*_~|<>[\]{}]/;
/** A lone one of these, from a long utterance, is silence being decorated. */
const FILLER_ONLY = new Set([
  'grazie', 'grazie mille', 'ok', 'okay', 'si', 'no', 'ciao', 'pronto',
  'thank you', 'thanks', 'bye', 'hello', 'hey', 'yeah', 'yep', 'yes',
  'right', 'sure', 'please', 'you',
]);
const MIN_BOILERPLATE_WORDS = 4;

function foldText(text) {
  return (text || '')
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .replace(/['’]/g, ' ')
    .replace(/[^\p{L}\p{N}\s]/gu, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

function wordsOf(text) {
  return foldText(text).split(' ').filter(Boolean);
}

/** True for a run with no letters or digits at all ("♪♪♪", "..."). */
function symbolsOnly(text) {
  return !/[\p{L}\p{N}]/u.test(text || '');
}

/** True when the text is nothing but one phrase repeated back-to-back — the
 *  decoder loop that turns a single credit line into hundreds. */
function isRepeatedPhrase(text) {
  const words = wordsOf(text);
  for (let size = MIN_BOILERPLATE_WORDS; size <= Math.floor(words.length / 2); size++) {
    const period = words.slice(0, size);
    let reps = 0;
    let i = 0;
    while (i + size <= words.length && period.every((w, j) => words[i + j] === w)) {
      reps += 1;
      i += size;
    }
    if (reps < 2) continue;
    // A trailing partial repetition must still follow the period (that is a
    // window cut mid-loop, not a sentence that happens to start alike).
    if (words.slice(i).every((w, j) => w === period[j])) return true;
  }
  return false;
}

/** True when the text carries a subtitle/credit/CTA fingerprint. */
function looksInvented(text) {
  const raw = text || '';
  if (!foldText(raw) && (raw.trim() || URL_RE.test(raw))) return true;
  if (URL_RE.test(raw) && !foldText(raw.replace(URL_RE, ' '))) return true;
  if (MARKUP_RE.test(raw)) return true;
  const folded = foldText(raw);
  if (HALLUCINATION_PHRASES.some((re) => re.test(folded))) return true;
  if (folded.includes('pubblico') && folded.includes('libro')) return true;
  return false;
}

/**
 * The part of a transcript a human actually said. Returns '' when the whole
 * result was invented, so callers can drop the turn instead of answering it.
 */
export function cleanTranscript(text) {
  const raw = (text || '').trim();
  if (!raw) return '';
  if (symbolsOnly(raw) || symbolsOnly(raw.replace(URL_RE, ' '))) return '';
  if (isRepeatedPhrase(raw)) return '';
  if (looksInvented(raw)) {
    const kept = raw
      .split(/(?<=[.!?…])\s+|\n+/)
      .filter((s) => s.trim() && !symbolsOnly(s) && !looksInvented(s))
      .map((s) => s.replace(URL_RE, ' ').replace(/\s+/g, ' ').replace(/^[\s.,:;\-–—]+|[\s.,:;\-–—]+$/g, '').trim())
      .filter(Boolean)
      .join(' ')
      .trim();
    return FILLER_ONLY.has(foldText(kept)) ? '' : kept;
  }
  return raw;
}
