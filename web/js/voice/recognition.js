import { createVoicePlayback } from './playback.js';
import { extractAfterWake, isStopPhrase } from './wake.js';
import { floatTo16BitPCM, acquireMicrophone } from './capture.js';
import { apiFetch, getVoiceLang, getVoiceLangExplicit } from '../api.js';
import { cleanTranscript, isSelfEcho } from '../transcriptGuard.js';
import { setSphereState, setVoiceReady, setConversationMode } from '../sphere.js';
import {
  getAiName, getSilenceTimeout, getWhisperModel, getEffectiveSttEngine, getWakeWord,
  getConversation, getConversationTimeout,
} from '../preferences.js';
import { t } from '../i18n.js';

/* Wake mode: the long-press arms the wake listener and it stays armed until
 * the wake phrase is heard or the user taps to cancel. There is deliberately no
 * "waiting for the wake word" timeout — being timed out mid-thought is the
 * whole complaint. Idle listening stays cheap because nothing is uploaded to
 * the STT server unless the local VAD hears speech (see pushWhisperAudio). */
const WAKE_COMMAND_TIMEOUT_MS = 8000;

/* ── faster-whisper streaming tuning ─────────────────────────
 * Whisper has no incremental decoder, so the sidecar re-decodes the tail of
 * the utterance and commits the stable prefix (LocalAgreement). The browser
 * only has to ship audio steadily and decide when the user stopped talking —
 * Whisper gives no end-of-speech signal of its own.
 */
const WHISPER_CHUNK_MS = 500;
/** Trailing silence that ends an utterance (after speech was heard). */
const WHISPER_ENDPOINT_SILENCE_MS = 900;
/** Frame RMS above which we count a frame as speech, not room noise. */
const WHISPER_SPEECH_RMS = 0.012;
/** Drop audio if the network cannot keep up, rather than growing forever. */
const WHISPER_MAX_QUEUED_SECONDS = 20;
/* Local voice-activity gate. While nobody is talking there is nothing worth
 * transcribing, so the mic is monitored locally and only speech (plus a short
 * run-up and tail) is uploaded. That is what makes an all-day open wake
 * listener affordable: silence costs no network and no Whisper CPU. */
const WHISPER_PREROLL_MS = 400;
const WHISPER_HANGOVER_MS = 700;

/** Status polls while the sidecar starts (it may boot with the server). */
const WHISPER_READY_ATTEMPTS = 20;
/** Longer window when the bundled model still has to be fetched. */
const WHISPER_DOWNLOAD_ATTEMPTS = 40;
const WHISPER_READY_DELAY_MS = 700;
/** How often a fallback to Vosk re-checks whether the sidecar came up. */
const WHISPER_RECOVERY_POLL_MS = 5000;
/**
 * Every voice request is bounded. apiFetch has no timeout, and one request
 * that never answered used to hang prepareVoice() for good: the bar sat in
 * `warming` — dimmed, pointer-events off — until the page was reloaded.
 */
const VOICE_FETCH_TIMEOUT_MS = 8000;
/** The Vosk model download/parse is local and quick; a stall is a failure. */
const VOSK_MODEL_TIMEOUT_MS = 30000;
/** The one-time Vosk model fetch is a real download and may take a while. */
const VOSK_DOWNLOAD_TIMEOUT_MS = 120000;

let voskModel = null;
let recognizer = null;
let audioContext = null;
let mediaStream = null;
let processor = null;
let listening = false;
let listenMode = 'single';
let sttLang = 'en';          // Vosk model language (lang_map's vosk_stt_lang)
let voiceLang = 'en';        // ISO-639-1 passed to faster-whisper
let silenceTimer = null;
let wakeDetected = false;
let awaitingCommand = false;
/**
 * A request was already heard after the wake phrase (a partial may show it
 * before the phrase-only case can be decided). The endpointer uses this to know
 * that "hey <name>, what's the weather" was spoken in one breath and should be
 * finalized and dispatched — instead of waiting for a command that is already
 * there.
 */
let wakeRemainderSeen = false;

// Engine selection, resolved in prepareVoice().
let sttEngine = 'whisper';
/**
 * The engine the *current* session is actually streaming to.
 *
 * A wake session always listens with **Vosk** when its model is loaded: a
 * listener that stays armed for minutes must cost a local WASM recogniser, not
 * a Whisper pass per utterance. The moment the wake phrase is heard the session
 * swaps to `sttEngine` — the engine the user chose in Settings (Whisper), or
 * Vosk when that is what Settings/Power Saver resolve to — so the request
 * itself gets the accuracy and the language model the user picked. `sttEngine`
 * stays the configured choice; this is only what the audio callback feeds.
 */
let sessionEngine = 'whisper';
/** True once the Vosk model is loaded, so the wake listener can use it. */
let voskReady = false;
/** Language of the loaded Vosk model, so a reload only happens when it differs. */
let voskLang = '';
let whisperModel = 'tiny';
let whisperReady = false;
/** True when this boot kicked off a model download that is still running. */
let whisperPendingDownload = false;
/**
 * True while the session is running on the Vosk fallback because
 * faster-whisper was not up yet. Cleared once the sidecar answers and the
 * session upgrades back — see scheduleWhisperRecovery().
 */
let whisperRetry = false;
let whisperRecoveryTimer = null;
/** An engine/language change arrived while a session was live; apply after. */
let pendingReconfigure = false;
let reconfiguringVoice = false;
let voicePreparing = false;

// faster-whisper streaming session state.
let whisperSession = null;
let whisperQueue = [];
let whisperQueuedSamples = 0;
let whisperSending = false;
let whisperFinalizing = false;
let whisperLastPartial = '';
/** Local VAD state: true while the user is (probably) talking. */
let whisperSpeechActive = false;
let whisperLastVoiceAt = 0;
/** Run-up audio kept while idle, flushed when speech starts. */
let whisperPreroll = [];
let whisperPrerollSamples = 0;
/** Bumped on every listen start/stop so stale async work is ignored. */
let listenToken = 0;
let sawSpeech = false;
let lastVoiceAt = 0;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/* ── Conversation window ────────────────────────────────────────
 * Once the wake word has been heard, the assistant keeps listening for a while
 * after each answer so the user can follow up without saying the phrase again.
 * The window is what turns a wake session into a conversation, and it is the
 * only listening the app opens without a gesture — so it is bounded by
 * `voice.conversation_timeout`, it never opens unless Settings asks for it, and
 * the orb shows `conversation` for as long as it is armed.
 *
 * The microphone rule that makes this safe: the recogniser is open **only while
 * the assistant is silent**. A reply closing the session is what stops the
 * assistant from transcribing (and answering) itself — not echo cancellation.
 * The one ear that stays open during a reply is the barge-in monitor, which
 * measures energy and never transcribes.
 */
const CONVERSATION_LISTEN_MODE = 'continuous';
/** How long after a reply an echo-shaped transcript is still treated as ours. */
const ECHO_FILTER_WINDOW_MS = 2500;

let conversationTimer = null;
/** When the window's budget was last (re)armed, so speech does not restart it
 *  on every audio frame. */
let conversationArmedAt = 0;
/** True while this session began with the wake word (or already is a window). */
let conversationSession = false;
/** A reply is audibly playing right now. */
let playbackActive = false;
/** The last reply the assistant spoke — what the echo filter compares against. */
let playbackText = '';
/** When that reply stopped playing. */
let playbackEndedAt = 0;
/** An interruption happened: the next turn is a correction of that turn. */
let correctionPending = false;

function clearSilenceTimer() {
  if (silenceTimer) {
    clearTimeout(silenceTimer);
    silenceTimer = null;
  }
}

function resetWakeState() {
  wakeDetected = false;
  awaitingCommand = false;
  wakeRemainderSeen = false;
}

function resetEndpointing() {
  sawSpeech = false;
  lastVoiceAt = 0;
  whisperSpeechActive = false;
  whisperLastVoiceAt = 0;
  whisperPreroll = [];
  whisperPrerollSamples = 0;
}

function armSilenceTimer() {
  clearSilenceTimer();
  if (listenMode !== 'single') return;
  silenceTimer = setTimeout(() => {
    silenceTimer = null;
    if (!listening) return;
    // The utterance has already been sent for a final decode: cancelling now
    // would discard text that is seconds away. Measured round trips run 1–6 s
    // against an 8 s budget, so this race is reachable — give it a short grace
    // window instead, and re-arm if the decode is somehow still going.
    if (whisperFinalizing) {
      armSilenceTimer();
      return;
    }
    cancelListening();
    setSphereState('idle');
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: t('voice.didntCatchThat', null, "Didn't catch that"), type: 'info' },
    }));
    window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'silence' } }));
  }, getSilenceTimeout());
}

function armWakeCommandTimer() {
  clearSilenceTimer();
  silenceTimer = setTimeout(() => {
    silenceTimer = null;
    if (!listening || !awaitingCommand) return;
    cancelListening();
    setSphereState('idle');
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: t('voice.didntCatchThat', null, "Didn't catch that"), type: 'info' },
    }));
    window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'silence' } }));
  }, WAKE_COMMAND_TIMEOUT_MS);
}

/* ── Conversation window ────────────────────────────────────── */

function clearConversationTimer() {
  if (conversationTimer) {
    clearTimeout(conversationTimer);
    conversationTimer = null;
  }
}

/** (Re)start the window's silence budget — every burst of speech pushes it out. */
function armConversationTimer() {
  clearConversationTimer();
  conversationArmedAt = performance.now();
  const timeout = getConversationTimeout();
  // No limit (the default): a conversation lasts until the user ends it — a tap
  // for one thing only, a double tap to type, or a stop phrase.
  if (!timeout) return;
  conversationTimer = setTimeout(() => {
    conversationTimer = null;
    if (!listening || listenMode !== CONVERSATION_LISTEN_MODE) return;
    closeConversation();
  }, timeout);
}

/**
 * Listen for the next request without the wake word.
 *
 * Called when a reply stops playing. This is the only listening the app opens
 * without a gesture, so it needs the Settings switch, a wake session to have
 * happened, and it always comes back to a bounded window.
 */
async function openConversationWindow() {
  if (playbackActive || listening || !conversationSession || !getConversation()) return;
  try {
    await startListening(CONVERSATION_LISTEN_MODE);
  } catch (e) {
    // A microphone that will not open ends the conversation, not the app; the
    // next press reports it.
    console.warn('Conversation window could not open:', e);
    closeConversation();
    return;
  }
  // A reply can start while the microphone is still opening — and a gesture can
  // take the microphone first — so the window only claims a session it actually
  // opened, and it is never live during playback (the rule the design rests on).
  if (playbackActive || !listening || listenMode !== CONVERSATION_LISTEN_MODE) return;
  armConversationTimer();
  setConversationMode(true);
}

/**
 * End the conversation without arming anything else.
 *
 * A tap means "one thing only" and a double tap means "I'll type": both leave
 * the conversation, and neither should leave a wake listener running behind the
 * gesture that follows.
 */
export function leaveConversation() {
  conversationSession = false;
  correctionPending = false;
  clearConversationTimer();
  if (listening) stopListening();
  setConversationMode(false);
  setSphereState('idle');
}

/** End the window (timeout or stop phrase) and go back to waiting for the wake word. */
function closeConversation() {
  leaveConversation();
  // Hands free again: back to the armed wake listener, exactly as a long-press
  // leaves it (Vosk, local, nothing uploaded) — or closed, if the wake word is
  // off and there is nothing left to wait for.
  if (getWakeWord()) startListening('wake').catch(() => {});
}

/** True while the app is in conversation mode (listening without the phrase). */
export function isConversationOpen() {
  return conversationSession && listening && listenMode === CONVERSATION_LISTEN_MODE;
}

/** True when `said` is (mostly) the reply the assistant just spoke. */
function isEchoTurn(said) {
  return !!said && !!playbackText
    && performance.now() - playbackEndedAt <= ECHO_FILTER_WINDOW_MS
    && isSelfEcho(said, playbackText);
}

/**
 * Drop a transcript that is the assistant's own reply picked up by the
 * microphone. Not a turn: inside a window the listening simply continues, and
 * outside one the bar goes back to rest.
 */
function dropEchoTurn(said, mode) {
  console.warn("Dropped a transcript that repeats the assistant's reply:", said);
  if (mode === CONVERSATION_LISTEN_MODE && getConversation()) {
    stopListening();
    void openConversationWindow();
    return;
  }
  setSphereState('idle');
}

/**
 * Mark that the next request corrects the turn just interrupted.
 *
 * The voice path sets this itself, from `voice:barge-in`; `app.js` calls this
 * when the user stops an answer by tapping or long-pressing, which is the same
 * intent — "no, I meant…" — and should land as a correction too. The flag is
 * consumed by the next dispatched turn, so it cannot leak into a later one.
 */
export function markCorrectionPending() {
  correctionPending = true;
}

window.addEventListener('voice:barge-in', () => {
  correctionPending = true;
});

/**
 * A reply is audible, or has just stopped.
 *
 * `speaking: true` closes any listening that a previous turn left open: the
 * recogniser must never be live while the assistant talks, or it would
 * transcribe — and answer — the assistant itself. The barge-in monitor is a
 * separate stream and keeps running, which is what still lets the user
 * interrupt. `speaking: false` is the other half of the deal: this is the
 * moment the conversation window re-opens.
 */
window.addEventListener('voice:speaking', (e) => {
  const detail = e.detail || {};
  if (detail.text) playbackText = detail.text;
  if (detail.speaking) {
    playbackActive = true;
    clearConversationTimer();
    if (listening) stopListening();
    return;
  }
  playbackActive = false;
  playbackEndedAt = performance.now();
  void openConversationWindow();
});

function dispatchVoiceResult(text) {
  clearSilenceTimer();
  const mode = listenMode;
  // Nothing invented ever reaches the agent: a credits-only result is the same
  // as hearing nothing, so the turn is dropped instead of answered.
  const said = cleanTranscript(text);
  if (isEchoTurn(said)) {
    // The window opens onto the tail of the assistant's own audio, and the
    // decoder types it out. A wake listener keeps waiting for the phrase; a
    // window keeps waiting for a real request.
    if (mode === CONVERSATION_LISTEN_MODE) dropEchoTurn(said, mode);
    return;
  }
  stopListening();
  if (!said) {
    setSphereState('idle');
    window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'silence' } }));
    return;
  }
  // A wake turn (and anything said inside a window) is a conversation: the
  // window re-opens after the answer, and the reply is what the echo filter
  // compares against.
  if (mode === 'wake' || mode === CONVERSATION_LISTEN_MODE) conversationSession = true;
  // An interruption just before this request makes it a correction of the turn
  // the user stopped, so the agent knows what "no, intendo Milano" refers to.
  const correction = correctionPending;
  correctionPending = false;
  window.dispatchEvent(new CustomEvent('voice:result', { detail: { text: said, mode, correction } }));
}

function handleWakeTranscript(text, isFinal) {
  // An invented partial can otherwise read as "the wake phrase was heard",
  // arming a command window that then answers the credits.
  text = cleanTranscript(text);
  if (!text) return;
  if (awaitingCommand) {
    if (!isFinal || !text) return;
    // If the wake phrase gets repeated along with the request ("hey <name>,
    // turn on the lights"), hand the agent only the request.
    dispatchVoiceResult(extractAfterWake(text, getAiName()) || text);
    return;
  }

  const remainder = extractAfterWake(text, getAiName());
  if (remainder === null) return;

  wakeDetected = true;
  if (remainder) {
    // The request is part of this utterance. Remember it so the endpointer
    // finalizes it, but never dispatch a *partial*: a partial is revised as the
    // decoder hears more ("pi" → "pi tempo" → "pi tempo so …"), and dispatching
    // every revision turned one sentence into one confused turn per revision.
    wakeRemainderSeen = true;
    if (isFinal) dispatchVoiceResult(remainder);
    return;
  }

  if (isFinal) {
    awaitingCommand = true;
    setSphereState('listening');
    armWakeCommandTimer();
    // The phrase is behind us: hand the rest of the session to the engine the
    // user chose, so the request itself is not transcribed by the small Vosk
    // model that only had to recognise a name.
    swapToCommandEngine();
  }
}

/**
 * Move an armed wake session from the Vosk wake listener to the command engine.
 *
 * Only the destination changes — the microphone, the audio graph and the
 * session stay exactly as they are. That is what keeps the hand-off silent:
 * nothing is reopened, no second permission prompt appears, and the first word
 * after the wake phrase is not clipped. When the user's engine *is* Vosk, or
 * Vosk was never available and the wake phase already ran on Whisper, there is
 * nothing to move.
 */
function swapToCommandEngine() {
  if (sttEngine !== 'whisper' || sessionEngine === 'whisper') return;

  recognizer = null; // the wake listener's job is done
  whisperLastPartial = '';
  whisperQueue = [];
  whisperQueuedSamples = 0;
  whisperSpeechActive = false;
  whisperPreroll = [];
  whisperPrerollSamples = 0;
  whisperSession = newSessionId();
  // The wake phrase was speech, but it belongs to the previous engine: the
  // endpointing clock starts over with the command, so the pause a user takes
  // to think before speaking it is not read as "the utterance ended" — which
  // would finalize an empty session and answer "Didn't catch that". The
  // Whisper wake path always did this by clearing `sawSpeech` when the phrase
  // landed; the hand-off has to do the same.
  sawSpeech = false;
  lastVoiceAt = 0;
  sessionEngine = 'whisper';
}

function handleTranscript(text, isFinal) {
  if (!text) return;
  if (listenMode === 'wake') {
    handleWakeTranscript(text, isFinal);
    return;
  }
  if (!isFinal) return;
  // "Basta" / "stop" on its own ends the conversation instead of becoming a
  // request — and because only the whole utterance counts, "stop the music"
  // still reaches the agent.
  if (listenMode === CONVERSATION_LISTEN_MODE && isStopPhrase(cleanTranscript(text))) {
    closeConversation();
    return;
  }
  dispatchVoiceResult(text);
}

/* ── faster-whisper transport ───────────────────────────────── */

function newSessionId() {
  const uuid = globalThis.crypto?.randomUUID?.();
  if (uuid) return uuid;
  return `stt-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function enqueueWhisper(pcm) {
  whisperQueue.push(pcm);
  whisperQueuedSamples += pcm.length;
  const max = WHISPER_MAX_QUEUED_SECONDS * 16000;
  while (whisperQueuedSamples > max && whisperQueue.length > 1) {
    whisperQueuedSamples -= whisperQueue.shift().length;
  }
}

function drainWhisperQueue() {
  if (!whisperQueue.length) return null;
  let total = 0;
  for (const chunk of whisperQueue) total += chunk.length;
  const out = new Int16Array(total);
  let offset = 0;
  for (const chunk of whisperQueue) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  whisperQueue = [];
  whisperQueuedSamples = 0;
  return out;
}

/**
 * Local voice-activity gate for the faster-whisper path.
 *
 * Idle silence is not worth a round trip or a Whisper pass, so audio is only
 * queued while speech is (probably) present — plus a short run-up kept in a
 * ring buffer so the first syllable is never clipped, and a tail so trailing
 * words are not cut. With this, an open wake listener costs one cheap RMS loop
 * per audio frame until somebody actually speaks.
 */
function pushWhisperAudio(pcm, isSpeech, now) {
  if (isSpeech) {
    if (!whisperSpeechActive) {
      whisperSpeechActive = true;
      for (const chunk of whisperPreroll) enqueueWhisper(chunk);
      whisperPreroll = [];
      whisperPrerollSamples = 0;
    }
    enqueueWhisper(pcm);
    whisperLastVoiceAt = now;
    return;
  }

  if (whisperSpeechActive) {
    if (now - whisperLastVoiceAt <= WHISPER_HANGOVER_MS) {
      enqueueWhisper(pcm);
      return;
    }
    whisperSpeechActive = false;
  }

  // Idle: remember just enough audio to prepend to the next utterance.
  whisperPreroll.push(pcm);
  whisperPrerollSamples += pcm.length;
  const maxPreroll = Math.round((WHISPER_PREROLL_MS / 1000) * 16000);
  while (whisperPrerollSamples > maxPreroll && whisperPreroll.length > 1) {
    whisperPrerollSamples -= whisperPreroll.shift().length;
  }
}

async function postWhisperChunk(pcm, isFinal, session) {
  const params = new URLSearchParams({ session, lang: voiceLang, model: whisperModel });
  if (isFinal) params.set('final', 'true');
  // Wake mode biases the decoder with the wake phrase: without it the tiny
  // model turns an unusual assistant name into a homophone ("Peak'd"→"Beak"),
  // and the wake word stops firing.
  if (listenMode === 'wake') params.set('prompt', `Hey ${getAiName()}`);
  // A zero-length body is still a valid flush: the sidecar decodes the whole
  // utterance and drops the session.
  const body = pcm && pcm.length
    ? pcm.buffer.slice(pcm.byteOffset, pcm.byteOffset + pcm.byteLength)
    : new ArrayBuffer(0);
  return apiFetch(`/api/voice/stt/chunk?${params.toString()}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/octet-stream' },
    body,
  });
}

function applyWhisperPartial(res) {
  const text = (res?.text || '').trim();
  if (text) whisperLastPartial = text;
  // Only wake mode acts on partials; single-shot waits for the final pass.
  if (listenMode === 'wake' && !whisperFinalizing) handleTranscript(text, false);
}

async function pumpWhisperQueue(token) {
  if (whisperSending || !whisperReady || !whisperSession) return;
  whisperSending = true;
  const session = whisperSession;
  try {
    while (whisperQueue.length && listening && token === listenToken && session === whisperSession) {
      const pcm = drainWhisperQueue();
      if (!pcm) break;
      const res = await postWhisperChunk(pcm, false, session);
      if (token !== listenToken || session !== whisperSession) return;
      applyWhisperPartial(res);
    }
  } catch (e) {
    if (token === listenToken) whisperFailed(e);
  } finally {
    whisperSending = false;
  }
}

/**
 * End-of-utterance: send whatever is buffered, ask for one accurate pass over
 * the whole utterance, and hand the text to the agent.
 *
 * Whisper never says "the user stopped talking", so this is driven by the
 * trailing-silence detector in the audio callback.
 */
async function finalizeWhisper() {
  if (!listening || whisperFinalizing || sessionEngine !== 'whisper') return;
  const token = listenToken;
  const session = whisperSession;
  if (!session) return;

  whisperFinalizing = true;
  const mode = listenMode;
  let text = whisperLastPartial;

  try {
    const pcm = drainWhisperQueue();
    if (pcm && pcm.length) {
      const res = await postWhisperChunk(pcm, false, session);
      if (token !== listenToken) return;
      const partial = (res?.text || '').trim();
      if (partial) text = partial;
    }
    const res = await postWhisperChunk(null, true, session);
    if (token !== listenToken) return;
    const finalText = (res?.text || '').trim();
    if (finalText) text = finalText;
  } catch (e) {
    if (token !== listenToken) return;
    // A failed flush must not throw away what we already heard.
    if (!text) {
      whisperFinalizing = false;
      whisperFailed(e);
      return;
    }
  } finally {
    // Always released, even when this run was superseded: a stale finalize
    // that returned early used to leave this latch set, and every later turn
    // then bailed at the guard above and transcribed nothing for good.
    whisperFinalizing = false;
  }

  if (token !== listenToken) return;
  clearSilenceTimer();
  stopListening();

  if (text && isEchoTurn(text)) {
    dropEchoTurn(text, mode);
    return;
  }
  if (text) {
    // A wake-session request can still carry the phrase it followed ("hey
    // <name>, turn on the lights") — the decoder is biased with that phrase, so
    // it often does. Hand the agent only the request, exactly as the partial
    // path does.
    const said = mode === 'wake' ? extractAfterWake(text, getAiName()) || text : text;
    if (mode === 'wake' || mode === CONVERSATION_LISTEN_MODE) conversationSession = true;
    const correction = correctionPending;
    correctionPending = false;
    window.dispatchEvent(new CustomEvent('voice:result', { detail: { text: said, mode, correction } }));
    return;
  }
  setSphereState('idle');
  window.dispatchEvent(new CustomEvent('app:toast', {
    detail: { message: t('voice.didntCatchThat', null, "Didn't catch that"), type: 'info' },
  }));
  window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'silence' } }));
}

function whisperFailed(e) {
  console.warn('Faster-whisper STT failed:', e);
  const message = /not running|not responding|Failed to fetch|502/i.test(e?.message || '')
    ? t('voice.whisperNotResponding', null,
      'Faster Whisper is not responding — pick Vosk in Settings → Voice')
    : (e?.message || t('voice.recognitionFailed', null, 'Speech recognition failed'));
  if (!listening) return;
  stopListening();
  setSphereState('idle');
  window.dispatchEvent(new CustomEvent('app:toast', {
    detail: { message, type: 'error' },
  }));
  window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'error' } }));
}

/** Trailing-silence endpointing: Whisper has no VAD on the wire. */
function trackEndpointing(rms) {
  if (sessionEngine !== 'whisper' || !listening || whisperFinalizing) return;
  const now = performance.now();

  if (rms > WHISPER_SPEECH_RMS) {
    sawSpeech = true;
    lastVoiceAt = now;
    // Keep the "waiting for speech" cap from cutting off a long sentence.
    if (listenMode === 'single') armSilenceTimer();
    return;
  }
  if (!sawSpeech) return;
  if (now - lastVoiceAt < WHISPER_ENDPOINT_SILENCE_MS) return;

  // "Hey <name>" then a pause: that pause is the user getting ready to speak,
  // not the end of the request — keep the session open for the command. A
  // request already heard in the same breath is different: the user is done
  // speaking, so fall through and finalize it.
  if (listenMode === 'wake' && wakeDetected && !awaitingCommand && !wakeRemainderSeen) {
    sawSpeech = false;
    awaitingCommand = true;
    setSphereState('listening');
    armWakeCommandTimer();
    return;
  }

  const expecting = listenMode === 'single'
    || listenMode === CONVERSATION_LISTEN_MODE
    || (listenMode === 'wake' && (awaitingCommand || wakeRemainderSeen));
  if (!expecting) return;
  sawSpeech = false;
  void finalizeWhisper();
}

/* ── Voice preparation ──────────────────────────────────────── */

/** Reject when `promise` has not settled within `ms` — a stuck model load must
 *  not hold the voice bar hostage. */
function withTimeout(promise, ms, message) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(message)), ms);
    promise.then(
      (value) => { clearTimeout(timer); resolve(value); },
      (err) => { clearTimeout(timer); reject(err); },
    );
  });
}

/** apiFetch with a hard timeout, for the calls prepareVoice() awaits. */
async function voiceFetch(path, options = {}, timeoutMs = VOICE_FETCH_TIMEOUT_MS) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await apiFetch(path, { ...options, signal: controller.signal });
  } catch (e) {
    if (e?.name === 'AbortError') console.warn(`Voice request timed out: ${path}`);
    throw e;
  } finally {
    clearTimeout(timer);
  }
}

async function fetchVoiceStatus(lang) {
  try {
    return await voiceFetch(`/api/voice/status?lang=${encodeURIComponent(lang)}`);
  } catch (_) {
    return null;
  }
}

const hasWhisperModel = (status, key) => status?.whisper_models?.[key]?.present === true;

/**
 * Make sure the faster-whisper sidecar is up and the chosen model is on disk.
 *
 * The tiny model is bundled with the app, so a fresh install needs no
 * download; this only has to wait for the sidecar (it is started alongside
 * the server) and fall back to tiny when the optional small model is absent.
 */
async function ensureWhisperReady(lang, initial) {
  let status = initial;
  let downloading = false;

  if (status?.whisper === 'ready' && !hasWhisperModel(status, whisperModel)) {
    if (whisperModel !== 'tiny' && hasWhisperModel(status, 'tiny')) {
      whisperModel = 'tiny';
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: {
          message: t('voice.smallModelFallback', null, 'Small model not downloaded — using Tiny'),
          type: 'info',
        },
      }));
    } else {
      // Fresh checkout: fetch the bundled tiny model rather than going silent.
      whisperModel = 'tiny';
      downloading = true;
      try {
        await voiceFetch('/api/voice/whisper/download', {
          method: 'POST',
          body: JSON.stringify({ model: 'tiny' }),
        });
      } catch (_) { /* status polling below decides readiness */ }
    }
  }

  // A bundled model needs a moment for the sidecar to load; a download needs
  // much longer, so give it a real chance before falling back to Vosk.
  const attempts = downloading ? WHISPER_DOWNLOAD_ATTEMPTS : WHISPER_READY_ATTEMPTS;
  for (let attempt = 0; attempt < attempts; attempt++) {
    if (status?.whisper === 'ready' && hasWhisperModel(status, whisperModel)) return status;
    await sleep(WHISPER_READY_DELAY_MS);
    status = await fetchVoiceStatus(lang);
  }
  whisperPendingDownload = downloading;
  return status;
}

/**
 * A fallback to Vosk is not forever.
 *
 * The sidecar boots with the server and can still be loading its model when
 * the page asks — and the page only asks for a few seconds. Rather than let
 * that timing decide the engine for the whole session, keep checking quietly
 * and upgrade when it answers.
 *
 * The swap is held until nothing is listening: startListening() sets up the
 * recogniser or Whisper session the session actually streams to
 * (`sessionEngine`), and the per-frame reader branches on that, so flipping the
 * preference mid-session would send audio to an engine that was never set up.
 * (The one deliberate mid-session hand-off is the wake → command swap, which
 * sets its own engine up first — see swapToCommandEngine.)
 *
 * Deliberately invisible: no toast, no status line. The upgrade only makes
 * the next thing the user says transcribe better.
 */
function scheduleWhisperRecovery() {
  if (whisperRecoveryTimer) return;
  whisperRecoveryTimer = setInterval(async () => {
    if (!whisperRetry || listening) return;
    // The user asked for Vosk outright (or Power Saver is forcing it); a probe
    // would only fight that choice.
    if (getEffectiveSttEngine() === 'vosk') return;
    const status = await fetchVoiceStatus(voiceLang);
    if (!status || status.whisper !== 'ready' || !hasWhisperModel(status, whisperModel)) return;
    if (listening) return; // something started during the probe — try again later
    sttEngine = 'whisper';
    whisperReady = true;
    whisperRetry = false;
    clearInterval(whisperRecoveryTimer);
    whisperRecoveryTimer = null;
  }, WHISPER_RECOVERY_POLL_MS);
}

export async function prepareVoice() {
  voicePreparing = true;
  try {
    return await prepareVoiceInner();
  } catch (e) {
    // Whatever went wrong, the bar must never be left dimmed in `warming` for
    // good: the recognition chain is bounded, so a rejection here is a real
    // failure, and enabling the bar in the error state lets the next gesture
    // say what is wrong instead of "warming up" forever. (One stalled request
    // used to do exactly that — see voiceFetch.)
    console.warn('Voice preparation failed:', e);
    setVoiceReady(true);
    setSphereState('error');
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: {
        message: t('voice.unavailable', null, 'Voice unavailable — you can still double-tap to type'),
        type: 'error',
      },
    }));
  } finally {
    voicePreparing = false;
    // An engine/language change that arrived while this prepare was in flight
    // re-runs now (no-op when the prepare read the new values already).
    if (pendingReconfigure) maybeReconfigureVoice();
  }
}

async function prepareVoiceInner() {
  const lang = getVoiceLang();
  // Voice loads silently in the background — no progress card, no
  // notification. The voice bar stays dimmed (setVoiceReady(false)) and `warming`
  // until the recognizer is usable; a gesture that arrives before then waits for
  // readiness (app.js tapOrb) instead of being dropped.
  setVoiceReady(false);
  setSphereState('warming');

  voiceLang = lang;
  sttEngine = getEffectiveSttEngine();
  whisperModel = getWhisperModel();

  let status = await fetchVoiceStatus(lang);
  sttLang = status?.stt_lang || lang;

  if (sttEngine === 'whisper') {
    status = await ensureWhisperReady(lang, status);
    whisperReady = status?.whisper === 'ready' && hasWhisperModel(status, whisperModel);
    if (!whisperReady) {
      sttEngine = 'vosk';
      // The sidecar may simply not be up yet: keep an eye out and upgrade the
      // session if it arrives, instead of making this moment the final word.
      whisperRetry = true;
      scheduleWhisperRecovery();
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: {
          message: whisperPendingDownload
            ? t('voice.whisperDownloading', null, 'Faster Whisper model is downloading — using Vosk for now')
            : t('voice.whisperUnavailable', null, 'Faster Whisper unavailable — using Vosk'),
          type: 'info',
        },
      }));
    }
  } else {
    whisperReady = false;
  }

  if (sttEngine === 'vosk') {
    // A failed status probe is treated as "missing": attempting the download
    // is harmless when the archive is already there, and initVosk would fail
    // anyway when it is not.
    try {
      if ((status?.vosk || 'missing') === 'missing') await downloadVoskModel(lang);
    } catch (e) {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: t('voice.modelDownloadFailed', null, 'Voice model download failed'), type: 'error' },
      }));
      setVoiceReady(true);
      setSphereState('error');
      return;
    }

    try {
      await loadVosk();
    } catch (e) {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: t('voice.modelLoadFailed', null, 'Speech model failed to load'), type: 'error' },
      }));
      setVoiceReady(true);
      setSphereState('error');
      return;
    }
  } else if (getWakeWord()) {
    // The wake listener runs on Vosk even when the command engine is Whisper,
    // so its model has to be in place before the first long-press. Failing here
    // is not fatal — the wake phase then falls back to the configured engine —
    // so it is logged rather than toasted at a user who never asked for Vosk.
    try {
      if ((status?.vosk || 'missing') === 'missing') await downloadVoskModel(lang);
      await loadVosk();
    } catch (e) {
      voskReady = false;
      console.warn('Vosk model unavailable for the wake listener:', e);
    }
  }

  setVoiceReady(true);
  setSphereState('idle');
}

/** Ask the server to fetch and pack the Vosk archive for `lang`. */
async function downloadVoskModel(lang) {
  // The server downloads the model inside this request, so it gets a bigger
  // budget than an ordinary poll.
  await voiceFetch('/api/voice/download', {
    method: 'POST',
    body: JSON.stringify({ lang }),
  }, VOSK_DOWNLOAD_TIMEOUT_MS);
}

/**
 * Load the Vosk model for the resolved STT language. A model that is already
 * loaded for that language is reused — re-parsing the archive for every
 * long-press would be pure latency.
 */
async function loadVosk() {
  if (voskModel && voskLang === sttLang) {
    voskReady = true;
    return;
  }
  try {
    await initVosk(sttLang);
  } catch (e) {
    voskReady = false;
    voskLang = '';
    throw e;
  }
  voskLang = sttLang;
  voskReady = true;
}

async function initVosk(lang) {
  if (voskModel) {
    voskModel.terminate?.();
    voskModel = null;
  }

  // The `v` query param is a cache-buster: vosk-browser derives its
  // IndexedDB folder name from the whole URL, so bumping it forces a fresh
  // download of a re-packed model (e.g. after the ivector-layout fix).
  const modelUrl = `/api/voice/models/vosk/${lang}.tar.gz?v=2`;
  voskModel = await withTimeout(
    Vosk.createModel(modelUrl),
    VOSK_MODEL_TIMEOUT_MS,
    'Vosk model load timed out',
  );
}

/* ── Listening ──────────────────────────────────────────────── */

export async function startListening(mode) {
  if (listening) return;
  listenMode = mode;
  listening = true;
  const token = ++listenToken;
  resetWakeState();
  resetEndpointing();
  whisperLastPartial = '';
  whisperQueue = [];
  whisperQueuedSamples = 0;
  whisperFinalizing = false;
  // A wake session starts on Vosk when its model is loaded — the configured
  // engine takes over the moment the phrase is heard (see
  // swapToCommandEngine). Every other session streams to the configured engine
  // for its whole life.
  sessionEngine = mode === 'wake' && voskReady ? 'vosk' : sttEngine;
  setSphereState(mode === 'single' ? 'listening' : 'conversation');

  try {
    // The Vosk model is resolved before the microphone opens. A failed prepare
    // leaves the engine on Vosk with no model loaded and the bar enabled in the
    // error state on purpose, so this gesture is the retry (the failure may
    // have been a timeout) — and a real failure has to be said, not end in a
    // null dereference with the microphone left running behind it.
    if (sessionEngine === 'vosk' && !(voskModel && voskLang === sttLang)) {
      setSphereState('warming');
      try {
        await loadVosk();
      } catch (_) {
        throw new Error(t('voice.modelLoadFailedHint', null,
          'Speech model failed to load — pick another engine in Settings → Voice'));
      }
      setSphereState(mode === 'single' ? 'listening' : 'conversation');
    }

    const stream = await acquireMicrophone(navigator.mediaDevices, {
      video: false,
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        // Ask for stereo: the glow leans away from the loud side. Mono mics
        // simply report pan 0 (both channels identical).
        channelCount: { ideal: 2 },
        sampleRate: 16000,
      },
    }, () => token === listenToken && listening);
    if (!stream) return;
    mediaStream = stream;

    audioContext = new AudioContext({ sampleRate: 16000 });

    // The permission prompt can outlive the gesture that started it: if the
    // session was cancelled meanwhile, release the fresh stream instead of
    // leaving the microphone open.
    if (token !== listenToken || !listening) {
      mediaStream?.getTracks().forEach((t) => t.stop());
      try { audioContext.close(); } catch (_) {}
      mediaStream = null;
      audioContext = null;
      return;
    }

    if (sessionEngine === 'whisper') {
      whisperSession = newSessionId();
    } else {
      recognizer = new voskModel.KaldiRecognizer(16000);
      recognizer.setWords(false);

      recognizer.on('result', (msg) => {
        const text = msg.result?.text?.trim();
        handleTranscript(text, true);
      });

      recognizer.on('partialresult', (msg) => {
        const text = msg.result?.partial?.trim();
        if (listenMode === 'wake') handleTranscript(text, false);
      });
    }

    const source = audioContext.createMediaStreamSource(mediaStream);
    processor = audioContext.createScriptProcessor(4096, 2, 1);
    let monoBuffer = null;
    processor.onaudioprocess = (e) => {
      if (!listening) return;
      try {
        const input = e.inputBuffer;
        const n = input.length;
        const left = input.getChannelData(0);
        const right = input.numberOfChannels > 1 ? input.getChannelData(1) : left;

        // Both engines want a mono 16 kHz buffer; downmix once and reuse it.
        if (!monoBuffer || monoBuffer.length !== n) {
          monoBuffer = audioContext.createBuffer(1, n, 16000);
        }
        const mono = monoBuffer.getChannelData(0);
        let sumL = 0;
        let sumR = 0;
        let sumM = 0;
        for (let i = 0; i < n; i++) {
          const l = left[i];
          const r = right[i];
          sumL += l * l;
          sumR += r * r;
          const m = (l + r) * 0.5;
          sumM += m * m;
          mono[i] = m;
        }

        const rms = (sum) => Math.sqrt(sum / n);
        const lvl = (v) => Math.min(1, v * 10);
        const level = rms(sumM);

        // One speech decision drives both the upload gate and the endpointer.
        if (sessionEngine === 'whisper') {
          pushWhisperAudio(floatTo16BitPCM(mono), level > WHISPER_SPEECH_RMS, performance.now());
          void pumpWhisperQueue(token);
        } else {
          recognizer.acceptWaveform(monoBuffer);
        }

        const rl = rms(sumL);
        const rr = rms(sumR);
        const total = rl + rr + 1e-6;
        window.dispatchEvent(new CustomEvent('voice:level', {
          detail: {
            level: lvl(level),
            left: lvl(rl),
            right: lvl(rr),
            // -1 hard left … +1 hard right.
            pan: Math.max(-1, Math.min(1, (rr - rl) / total)),
          },
        }));

        // Inside the window, speech pushes the silence budget out — the same
        // idea as the single-shot timer, and independent of the engine. The
        // 500 ms guard keeps it to a couple of timer restarts per second rather
        // than one per audio frame.
        if (listenMode === CONVERSATION_LISTEN_MODE && level > WHISPER_SPEECH_RMS
            && performance.now() - conversationArmedAt > 500) {
          armConversationTimer();
        }

        trackEndpointing(level);
      } catch (_) {}
    };
    source.connect(processor);
    processor.connect(audioContext.destination);

    if (mode === 'single') armSilenceTimer();
    // The window's budget is silence-based: armConversationTimer() restarts on
    // every burst of speech, so thinking out loud does not end the session.
    if (mode === CONVERSATION_LISTEN_MODE) armConversationTimer();
    // Wake mode arms no timer on purpose: it listens until the wake word.
  } catch (e) {
    // Release whatever the attempt already acquired: a failure after the
    // microphone opened (no WebAudio, say) must not leave capture running —
    // and the same teardown drops any whisper session opened for the attempt.
    stopListening();
    setSphereState('error');
    throw new Error(normalizeMicError(e));
  }
}

function normalizeMicError(e) {
  if (e.name === 'NotAllowedError' || e.name === 'PermissionDeniedError') {
    return t('voice.micAccessDenied', null, 'Microphone access denied');
  }
  if (e.name === 'NotFoundError' || /not be found/i.test(e.message || '')) {
    return t('voice.micNotFound', null, 'No microphone found');
  }
  return e.message || t('ctx.microphoneUnavailable', null, 'Microphone unavailable');
}

export function stopListening() {
  const finishedSession = whisperSession;
  const wasActive = listening || processor || mediaStream;

  listenToken++;
  listening = false;
  resetWakeState();
  resetEndpointing();
  clearSilenceTimer();
  clearConversationTimer();
  whisperQueue = [];
  whisperQueuedSamples = 0;
  whisperFinalizing = false;
  whisperSession = null;

  // Tell the sidecar to drop its buffer. A cancelled session is best-effort:
  // the sidecar also expires idle sessions on its own.
  if (finishedSession) {
    apiFetch(`/api/voice/stt/close?session=${encodeURIComponent(finishedSession)}`, {
      method: 'POST',
    }).catch(() => {});
  }

  if (!wasActive) return;
  try {
    processor?.disconnect();
  } catch (_) {}
  try {
    audioContext?.close();
  } catch (_) {}
  mediaStream?.getTracks().forEach((t) => t.stop());
  processor = null;
  audioContext = null;
  mediaStream = null;
  recognizer = null;

  // Power Saver or a language change may have arrived mid-session; re-resolve
  // now that the microphone is closed.
  if (pendingReconfigure) maybeReconfigureVoice();
}

/**
 * Re-resolve the speech engines after a power-mode change or an interface
 * language change. The TTS engine is read per reply, so only STT needs
 * re-initialising — and only when idle, with a retry flagged until the current
 * session ends. The voice language follows the interface language unless the
 * user picked a separate one in Settings (see getVoiceLang()).
 */
function maybeReconfigureVoice() {
  const engineStale = getEffectiveSttEngine() !== sttEngine;
  const langStale = !getVoiceLangExplicit() && getVoiceLang() !== voiceLang;
  if (!engineStale && !langStale) {
    pendingReconfigure = false;
    return;
  }
  // A live session gets the new setup once it ends; a prepare already in
  // flight re-checks from prepareVoice()'s completion path (the staleness
  // check above makes that a no-op when it read the new values already).
  if (listening || document.body.classList.contains('orb-speaking') || voicePreparing) {
    pendingReconfigure = true;
    return;
  }
  if (reconfiguringVoice) return;
  pendingReconfigure = false;
  reconfiguringVoice = true;
  prepareVoice().finally(() => { reconfiguringVoice = false; });
}

window.addEventListener('power:changed', maybeReconfigureVoice);
window.addEventListener('locale:change', maybeReconfigureVoice);

export function cancelListening() {
  // An explicit cancel ends the conversation too: the next reply must not
  // re-open a window the user has just closed by tapping.
  conversationSession = false;
  correctionPending = false;
  clearConversationTimer();
  setConversationMode(false);
  stopListening();
  window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'user' } }));
}

/**
 * Long-press release.
 *
 * Holding the voice bar only *arms* wake recognition — the hold is the trigger for
 * "hey <name>", not the thing that keeps the microphone open. Releasing must
 * therefore leave the recogniser running so the phrase (and then the request)
 * can be spoken after the finger lifts. There is no expiry on that: the wake
 * listener runs until the phrase arrives or the user taps to cancel.
 *
 * Returns true while a wake session is still listening.
 */
export function releaseWakeHold() {
  if (listenMode !== 'wake' || !listening) return false;
  return true;
}

/* ── Text-to-speech ─────────────────────────────────────────── */

const playback = createVoicePlayback({ onIdle: () => {
  if (pendingReconfigure) maybeReconfigureVoice();
} });
export function speak(text, lang) { return playback.speak(text, lang); }
export function stopSpeaking() { return playback.stopSpeaking(); }
export function startBargeInMonitor() { return playback.startBargeInMonitor(); }
export function stopBargeInMonitor() { return playback.stopBargeInMonitor(); }
export function isBargeInMonitoring() { return playback.isBargeInMonitoring(); }

export function isListening() {
  return listening;
}

export function isWakeAwaitingCommand() {
  return listenMode === 'wake' && (wakeDetected || awaitingCommand);
}

/** Which STT engine the last prepareVoice() settled on (for diagnostics/UI). */
export function getActiveSttEngine() {
  return sttEngine;
}
