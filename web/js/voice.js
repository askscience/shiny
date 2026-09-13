import { apiFetch, getVoiceLang } from './api.js';
import { setSphereState, setVoiceReady } from './sphere.js';
import {
  getAiName, getTtsVoice, getTtsSpeed, getSilenceTimeout,
  getSttEngine, getWhisperModel,
} from './preferences.js';

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

/* Barge-in: speaking over the assistant stops it and starts a new request.
 * The threshold sits well above the speech threshold used for transcription so
 * the assistant's own voice (imperfectly cancelled by the browser's AEC) does
 * not cut it off, and a short guard window after playback starts gives the
 * echo canceller time to converge. */
const BARGE_IN_RMS = 0.03;
const BARGE_IN_HOLD_MS = 350;
const BARGE_IN_SETTLE_MS = 800;
const BARGE_IN_TTS_GUARD_MS = 700;
/** Status polls while the sidecar starts (it may boot with the server). */
const WHISPER_READY_ATTEMPTS = 20;
/** Longer window when the bundled model still has to be fetched. */
const WHISPER_DOWNLOAD_ATTEMPTS = 40;
const WHISPER_READY_DELAY_MS = 700;
/** How often a fallback to Vosk re-checks whether the sidecar came up. */
const WHISPER_RECOVERY_POLL_MS = 5000;

let voskModel = null;
let recognizer = null;
let audioContext = null;
let mediaStream = null;
let processor = null;
let listening = false;
let listenMode = 'single';
let currentAudio = null;
let sttLang = 'en';          // Vosk model language (lang_map's vosk_stt_lang)
let voiceLang = 'en';        // ISO-639-1 passed to faster-whisper
let silenceTimer = null;
let wakeDetected = false;
let awaitingCommand = false;

// Engine selection, resolved in prepareVoice().
let sttEngine = 'whisper';
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
/** Resolver for the TTS playback currently in flight (stopSpeaking). */
let stopPlayback = null;
/** Bumped by stopSpeaking() so a reply still being fetched is dropped too. */
let speakToken = 0;
/** Barge-in monitor (mic stays open while the assistant answers). */
let barge = { stream: null, ctx: null, proc: null, source: null, since: 0, startedAt: 0, fired: true };
let bargeGuardUntil = 0;
/** Bumped on every listen start/stop so stale async work is ignored. */
let listenToken = 0;
let sawSpeech = false;
let lastVoiceAt = 0;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function clearSilenceTimer() {
  if (silenceTimer) {
    clearTimeout(silenceTimer);
    silenceTimer = null;
  }
}

function resetWakeState() {
  wakeDetected = false;
  awaitingCommand = false;
}

function resetEndpointing() {
  sawSpeech = false;
  lastVoiceAt = 0;
  whisperSpeechActive = false;
  whisperLastVoiceAt = 0;
  whisperPreroll = [];
  whisperPrerollSamples = 0;
}

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
function extractAfterWake(text) {
  const nameTokens = normalizeSpeech(getAiName()).split(' ').filter(Boolean);
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
      detail: { message: "Didn't catch that", type: 'info' },
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
      detail: { message: "Didn't catch that", type: 'info' },
    }));
    window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'silence' } }));
  }, WAKE_COMMAND_TIMEOUT_MS);
}

function dispatchVoiceResult(text) {
  clearSilenceTimer();
  const mode = listenMode;
  stopListening();
  window.dispatchEvent(new CustomEvent('voice:result', { detail: { text, mode } }));
}

function handleWakeTranscript(text, isFinal) {
  if (awaitingCommand) {
    if (!isFinal || !text) return;
    // If the wake phrase gets repeated along with the request ("hey <name>,
    // turn on the lights"), hand the agent only the request.
    dispatchVoiceResult(extractAfterWake(text) || text);
    return;
  }

  const remainder = extractAfterWake(text);
  if (remainder === null) return;

  wakeDetected = true;
  if (remainder) {
    dispatchVoiceResult(remainder);
    return;
  }

  if (isFinal) {
    awaitingCommand = true;
    setSphereState('listening');
    armWakeCommandTimer();
  }
}

function handleTranscript(text, isFinal) {
  if (!text) return;
  if (listenMode === 'wake') {
    handleWakeTranscript(text, isFinal);
    return;
  }
  if (isFinal) dispatchVoiceResult(text);
}

/* ── faster-whisper transport ───────────────────────────────── */

function newSessionId() {
  const uuid = globalThis.crypto?.randomUUID?.();
  if (uuid) return uuid;
  return `stt-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function floatTo16BitPCM(float32) {
  const out = new Int16Array(float32.length);
  for (let i = 0; i < float32.length; i++) {
    const s = Math.max(-1, Math.min(1, float32[i]));
    out[i] = s < 0 ? s * 0x8000 : s * 0x7fff;
  }
  return out;
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
  if (!listening || whisperFinalizing || sttEngine !== 'whisper') return;
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

  if (text) {
    window.dispatchEvent(new CustomEvent('voice:result', { detail: { text, mode } }));
    return;
  }
  setSphereState('idle');
  window.dispatchEvent(new CustomEvent('app:toast', {
    detail: { message: "Didn't catch that", type: 'info' },
  }));
  window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'silence' } }));
}

function whisperFailed(e) {
  console.warn('Faster-whisper STT failed:', e);
  const message = /not running|not responding|Failed to fetch|502/i.test(e?.message || '')
    ? 'Faster Whisper is not responding — pick Vosk in Settings → Voice'
    : (e?.message || 'Speech recognition failed');
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
  if (sttEngine !== 'whisper' || !listening || whisperFinalizing) return;
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
  // not the end of the request — keep the session open for the command.
  if (listenMode === 'wake' && wakeDetected && !awaitingCommand) {
    sawSpeech = false;
    awaitingCommand = true;
    setSphereState('listening');
    armWakeCommandTimer();
    return;
  }

  const expecting = listenMode === 'single' || (listenMode === 'wake' && awaitingCommand);
  if (!expecting) return;
  sawSpeech = false;
  void finalizeWhisper();
}

/* ── Voice preparation ──────────────────────────────────────── */

async function fetchVoiceStatus(lang) {
  try {
    return await apiFetch(`/api/voice/status?lang=${encodeURIComponent(lang)}`);
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
        detail: { message: 'Small model not downloaded — using Tiny', type: 'info' },
      }));
    } else {
      // Fresh checkout: fetch the bundled tiny model rather than going silent.
      whisperModel = 'tiny';
      downloading = true;
      try {
        await apiFetch('/api/voice/whisper/download', {
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
 * The swap is held until nothing is listening: startListening() resolves the
 * engine into a Vosk recognizer (or a whisper session) for that session's
 * whole life, and the per-frame reader branches on sttEngine, so flipping it
 * mid-session would send audio to an engine that was never set up.
 *
 * Deliberately invisible: no toast, no status line. The upgrade only makes
 * the next thing the user says transcribe better.
 */
function scheduleWhisperRecovery() {
  if (whisperRecoveryTimer) return;
  whisperRecoveryTimer = setInterval(async () => {
    if (!whisperRetry || listening) return;
    // The user asked for Vosk outright; a probe would only fight that choice.
    if (getSttEngine() === 'vosk') return;
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
  const lang = getVoiceLang();
  // Voice loads silently in the background — no progress card, no
  // notification. The orb just stays dimmed (setVoiceReady(false)) until the
  // recognizer is usable; tapping early already gives its own feedback.
  setVoiceReady(false);
  setSphereState('downloading');

  voiceLang = lang;
  sttEngine = getSttEngine();
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
            ? 'Faster Whisper model is downloading — using Vosk for now'
            : 'Faster Whisper unavailable — using Vosk',
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
    if ((status?.vosk || 'missing') === 'missing') {
      try {
        await apiFetch('/api/voice/download', {
          method: 'POST',
          body: JSON.stringify({ lang }),
        });
      } catch (e) {
        window.dispatchEvent(new CustomEvent('app:toast', {
          detail: { message: 'Voice model download failed', type: 'error' },
        }));
        setVoiceReady(true);
        setSphereState('error');
        return;
      }
    }

    try {
      await initVosk(sttLang);
    } catch (e) {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: 'Speech model failed to load', type: 'error' },
      }));
      setVoiceReady(true);
      setSphereState('error');
      return;
    }
  }

  setVoiceReady(true);
  setSphereState('idle');
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
  voskModel = await Vosk.createModel(modelUrl);
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
  setSphereState(mode === 'single' ? 'listening' : 'conversation');

  try {
    mediaStream = await navigator.mediaDevices.getUserMedia({
      video: false,
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        // Ask for stereo: the orb leans away from the loud side. Mono mics
        // simply report pan 0 (both channels identical).
        channelCount: { ideal: 2 },
        sampleRate: 16000,
      },
    });

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

    if (sttEngine === 'whisper') {
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
        if (sttEngine === 'whisper') {
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

        trackEndpointing(level);
      } catch (_) {}
    };
    source.connect(processor);
    processor.connect(audioContext.destination);

    if (mode === 'single') armSilenceTimer();
    // Wake mode arms no timer on purpose: it listens until the wake word.
  } catch (e) {
    listening = false;
    resetWakeState();
    resetEndpointing();
    clearSilenceTimer();
    setSphereState('error');
    const msg = normalizeMicError(e);
    throw new Error(msg);
  }
}

function normalizeMicError(e) {
  if (e.name === 'NotAllowedError' || e.name === 'PermissionDeniedError') {
    return 'Microphone access denied';
  }
  if (e.name === 'NotFoundError' || /not be found/i.test(e.message || '')) {
    return 'No microphone found';
  }
  return e.message || 'Microphone unavailable';
}

export function stopListening() {
  const finishedSession = whisperSession;
  const wasActive = listening || processor || mediaStream;

  listenToken++;
  listening = false;
  resetWakeState();
  resetEndpointing();
  clearSilenceTimer();
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
}

export function cancelListening() {
  stopListening();
  window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'user' } }));
}

/**
 * Long-press release.
 *
 * Holding the orb only *arms* wake recognition — the hold is the trigger for
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

export async function speak(text, lang) {
  stopSpeaking();
  const token = ++speakToken;

  // Nothing sits under the orb while the answer is fetched and read aloud.
  // This is a class rather than a direct hide so the CSS can keep the dock's
  // space — collapsing it would drop the orb and start it moving again.
  document.body.classList.add('orb-speaking');

  const voiceLang = lang || getVoiceLang();
  try {
    const blob = await apiFetch('/api/tts', {
      method: 'POST',
      body: JSON.stringify({
        text,
        lang: voiceLang,
        voice: getTtsVoice() || 'M1',
        speed: getTtsSpeed(),
      }),
      responseType: 'blob',
    });
    // Stopped while the audio was still being synthesised: say nothing.
    if (token !== speakToken) return;
    const url = URL.createObjectURL(blob);
    currentAudio = new Audio(url);

    // Keep the orb alive with the ASSISTANT's voice: the mic is already
    // stopped by now, so without this the animation would freeze exactly
    // when the answer arrives. Same signal shape as the microphone, but the
    // assistant speaks from the centre (pan 0).
    const stopPulse = attachTtsPulse(currentAudio);

    await new Promise((resolve, reject) => {
      const finish = () => {
        stopPlayback = null;
        stopPulse();
        URL.revokeObjectURL(url);
        resolve();
      };
      // stopSpeaking() resolves the same promise, so callers awaiting playback
      // (the agent turn) never hang when the user talks over the answer.
      stopPlayback = finish;
      currentAudio.onended = finish;
      currentAudio.onerror = () => {
        stopPlayback = null;
        stopPulse();
        reject(new Error('playback failed'));
      };
      // Give the echo canceller a moment before barge-in can fire on the
      // assistant's own voice.
      bargeGuardUntil = performance.now() + BARGE_IN_TTS_GUARD_MS;
      currentAudio.play().catch((e) => {
        stopPlayback = null;
        stopPulse();
        reject(e);
      });
    });
  } catch (e) {
    console.warn('TTS failed:', e);
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: 'Voice playback unavailable', type: 'error' },
    }));
  } finally {
    document.body.classList.remove('orb-speaking');
  }
}

/**
 * Cut the assistant off mid-sentence.
 *
 * Resolves the in-flight `speak()` promise so the turn that is awaiting
 * playback finishes immediately instead of hanging until the audio ends.
 * Returns true when something was actually playing.
 */
export function stopSpeaking() {
  const wasPlaying = !!stopPlayback;
  const stop = stopPlayback;
  stopPlayback = null;
  speakToken += 1; // a reply still being fetched must not start playing
  if (currentAudio) {
    try { currentAudio.pause(); } catch (_) {}
    currentAudio = null;
  }
  if (stop) {
    try { stop(); } catch (_) {}
  }
  document.body.classList.remove('orb-speaking');
  return wasPlaying;
}

/* ── Barge-in (talking over the assistant) ──────────────────── */

/**
 * Watch the microphone while the assistant is thinking or speaking.
 *
 * The mic has to stay open for the user to be able to interrupt, which is a
 * deliberate trade: `echoCancellation` plus a threshold well above the speech
 * threshold (and a short guard right after playback starts) keep the
 * assistant's own voice from cutting itself off.
 */
export async function startBargeInMonitor() {
  if (barge.stream) return;
  let stream = null;
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      video: false,
      audio: { echoCancellation: true, noiseSuppression: true, channelCount: { ideal: 1 } },
    });
    const Ctx = window.AudioContext || window.webkitAudioContext;
    const ctx = new Ctx();
    const source = ctx.createMediaStreamSource(stream);
    const proc = ctx.createScriptProcessor(1024, 1, 1);
    barge = {
      stream, ctx, proc, source,
      since: 0,
      startedAt: performance.now(),
      fired: false,
    };
    proc.onaudioprocess = (e) => {
      if (barge.fired || !barge.stream) return;
      const now = performance.now();
      if (now - barge.startedAt < BARGE_IN_SETTLE_MS || now < bargeGuardUntil) return;
      const data = e.inputBuffer.getChannelData(0);
      let sum = 0;
      for (let i = 0; i < data.length; i++) sum += data[i] * data[i];
      if (Math.sqrt(sum / data.length) > BARGE_IN_RMS) {
        if (!barge.since) barge.since = now;
        if (now - barge.since >= BARGE_IN_HOLD_MS) {
          barge.fired = true;
          window.dispatchEvent(new CustomEvent('voice:barge-in'));
        }
      } else {
        barge.since = 0;
      }
    };
    source.connect(proc);
    // A ScriptProcessor is only pulled while connected to a destination; it
    // writes nothing, so this stays silent.
    proc.connect(ctx.destination);
  } catch (_) {
    if (stream) stream.getTracks().forEach((t) => t.stop());
    stopBargeInMonitor();
  }
}

export function stopBargeInMonitor() {
  const { stream, ctx, proc, source } = barge;
  barge = { stream: null, ctx: null, proc: null, source: null, since: 0, startedAt: 0, fired: true };
  try { source?.disconnect(); } catch (_) {}
  try { proc?.disconnect(); } catch (_) {}
  try { ctx?.close(); } catch (_) {}
  stream?.getTracks().forEach((t) => t.stop());
}

export function isBargeInMonitoring() {
  return !!barge.stream;
}

/** WebAudio context for analysing the assistant's playback (lazily created). */
let ttsContext = null;

function analyserContext() {
  if (!ttsContext || ttsContext.state === 'closed') {
    const Ctx = window.AudioContext || window.webkitAudioContext;
    if (!Ctx) return null;
    ttsContext = new Ctx();
  }
  return ttsContext;
}

/** Unlock the analysis context on the first gesture, so it is already
 *  running by the time the assistant answers (creating one later would be
 *  suspended by the autoplay policy and we would lose the pulse). */
function unlockAnalyserContext() {
  const ctx = analyserContext();
  if (!ctx) {
    window.removeEventListener('pointerdown', unlockAnalyserContext);
    return;
  }
  if (ctx.state === 'suspended') ctx.resume().catch(() => {});
  if (ctx.state === 'running') window.removeEventListener('pointerdown', unlockAnalyserContext);
}
window.addEventListener('pointerdown', unlockAnalyserContext);

/**
 * Drive `voice:level` from an <audio> element's own output, so the orb keeps
 * reacting while the assistant talks. Returns a stop() that releases the
 * analyser and lets the orb settle. Falls back to a no-op when WebAudio can't
 * analyse (a suspended context would mute playback, so we never risk that).
 */
function attachTtsPulse(audio) {
  const ctx = analyserContext();
  if (!ctx || ctx.state !== 'running') {
    if (ctx && ctx.state === 'suspended') ctx.resume().catch(() => {});
    return () => {};
  }

  let source = null;
  let analyser = null;
  try {
    source = ctx.createMediaElementSource(audio);
    analyser = ctx.createAnalyser();
    analyser.fftSize = 1024;
    analyser.smoothingTimeConstant = 0.55;
    source.connect(analyser);
    analyser.connect(ctx.destination);
  } catch (_) {
    try { source?.disconnect(); } catch (_) {}
    try { analyser?.disconnect(); } catch (_) {}
    return () => {};
  }

  const data = new Float32Array(analyser.fftSize);
  let raf = 0;
  let stopped = false;

  const tick = () => {
    if (stopped || audio.ended || audio.paused) return;
    analyser.getFloatTimeDomainData(data);
    let sum = 0;
    for (let i = 0; i < data.length; i++) sum += data[i] * data[i];
    const level = Math.min(1, Math.sqrt(sum / data.length) * 3.4);
    window.dispatchEvent(new CustomEvent('voice:level', {
      detail: { level, pan: 0, source: 'tts' },
    }));
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);

  return () => {
    if (stopped) return;
    stopped = true;
    if (raf) cancelAnimationFrame(raf);
    try { source.disconnect(); } catch (_) {}
    try { analyser.disconnect(); } catch (_) {}
    // Let the orb fall back to rest instead of freezing on the last frame.
    window.dispatchEvent(new CustomEvent('voice:level', {
      detail: { level: 0, pan: 0, source: 'tts' },
    }));
  };
}

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
