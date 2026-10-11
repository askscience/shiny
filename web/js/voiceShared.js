/**
 * voiceShared.js — the TTS voice sets, shared by the preference store, the
 * Settings window and the speaker.
 *
 * Supertonic and Qwen3-TTS share no voice names: Supertonic speaks with its
 * M1…F5 style voices, Qwen3-TTS with the nine CustomVoice speakers. A voice
 * only means something together with its engine — `vivian` handed to
 * Supertonic is a 400 from the sidecar, not a fallback — so the preference
 * store keeps one voice per engine and everything that resolves a voice goes
 * through `voiceForEngine()`.
 *
 * Import-free on purpose: `web/js/tests/voiceShared.test.mjs` loads it in Node
 * with no DOM.
 */

/** Supertonic style voices (M1…M5 male, F1…F5 female). */
export const SUPERTONIC_VOICES = ['M1', 'M2', 'M3', 'M4', 'M5', 'F1', 'F2', 'F3', 'F4', 'F5'];

/** The nine built-in Qwen3-TTS CustomVoice speakers. */
export const QWEN_SPEAKERS = [
  'serena', 'vivian', 'uncle_fu', 'ryan', 'aiden', 'ono_anna', 'sohee', 'eric', 'dylan',
];

/** The voice each engine speaks with when the user has not picked one. */
export const ENGINE_DEFAULT_VOICE = { supertonic: 'M1', qwen: 'vivian' };

/** The voices `engine` accepts. */
export function voicesForEngine(engine) {
  return engine === 'qwen' ? QWEN_SPEAKERS : SUPERTONIC_VOICES;
}

/**
 * `voice` when it belongs to `engine`, else '' ("engine default"). This is the
 * guard that keeps a voice from one engine out of a request to the other.
 */
export function voiceForEngine(engine, voice) {
  const trimmed = (voice || '').trim();
  return voicesForEngine(engine).includes(trimmed) ? trimmed : '';
}
