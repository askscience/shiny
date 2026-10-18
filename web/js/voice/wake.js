/** Pure wake-phrase matching, shared by both recognition engines. */
function normalizeSpeech(text) {
  return text.toLowerCase().replace(/[^\w\s,]/g, ' ').replace(/\s+/g, ' ').trim();
}

/** Plain Levenshtein distance — used only on short wake-word tokens. */
function editDistance(a, b) {
  if (a === b) return 0;
  if (!a.length) return b.length;
  if (!b.length) return a.length;
  let prev = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    const cur = [i];
    for (let j = 1; j <= b.length; j++) {
      cur[j] = Math.min(
        prev[j] + 1,
        cur[j - 1] + 1,
        prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1),
      );
    }
    prev = cur;
  }
  return prev[b.length];
}

function tokenMatches(got, want) {
  if (got === want) return true;
  if (!got) return false;
  const tolerance = Math.max(got.length, want.length) >= 5 ? 2 : 1;
  return editDistance(got, want) <= tolerance;
}

/**
 * Index just past a wake phrase ending at `tokens[s …]`, or null.
 * The name must be either the whole utterance, right after "hey", or right
 * after a clipped "hey" (tiny Whisper renders "Hey Peak'd" as "K. Beak").
 */
function matchWakeName(tokens, nameTokens) {
  for (let s = 0; s + nameTokens.length <= tokens.length; s++) {
    let matched = true;
    for (let k = 0; k < nameTokens.length; k++) {
      if (!tokenMatches(tokens[s + k], nameTokens[k])) {
        matched = false;
        break;
      }
    }
    if (!matched) continue;
    if (s === 0) return nameTokens.length;
    if (tokenMatches(tokens[s - 1], 'hey')) return s + nameTokens.length;
    if (s === 1 && tokens[0].length <= 2) return s + nameTokens.length;
  }
  return null;
}

/**
 * Pull the request out of a transcript that starts with (or contains) the wake
 * phrase, e.g. "hey <name>, turn on the lights" → "turn on the lights".
 *
 * Matching is fuzzy on the name on purpose: the tiny Whisper model regularly
 * renders an unusual assistant name as a near-homophone ("Peak'd" → "Beak"),
 * and a wake phrase that only matches perfectly is a wake phrase that never
 * fires. Returns `''` when the phrase was heard with nothing after it, and
 * `null` when the phrase is absent.
 */
export function extractAfterWake(text, name) {
  const nameTokens = normalizeSpeech(name).split(' ').filter(Boolean);
  if (!nameTokens.length) return null;
  const tokens = normalizeSpeech(text).split(' ').filter(Boolean);

  // The name arrives either as separate words ("peak d") or merged into one
  // ("peakd" / "peaked"), so both shapes are worth a try.
  const shapes = [nameTokens];
  if (nameTokens.length > 1) shapes.push([nameTokens.join('')]);

  for (const shape of shapes) {
    const end = matchWakeName(tokens, shape);
    if (end !== null) return tokens.slice(end).join(' ').trim();
  }
  return null;
}

