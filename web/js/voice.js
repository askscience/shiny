import { apiFetch, getVoiceLang, setVoiceLang } from './api.js';
import { setSphereState, setVoiceReady } from './sphere.js';
import { getAiName, getTtsVoice, getTtsSpeed, getSilenceTimeout } from './preferences.js';

const WAKE_WAIT_TIMEOUT_MS = 15000;
const WAKE_COMMAND_TIMEOUT_MS = 8000;

let voskModel = null;
let recognizer = null;
let audioContext = null;
let mediaStream = null;
let processor = null;
let listening = false;
let listenMode = 'single';
let currentAudio = null;
let sttLang = 'en';
let silenceTimer = null;
let wakeDetected = false;
let awaitingCommand = false;

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

function normalizeSpeech(text) {
  return text.toLowerCase().replace(/[^\w\s,]/g, ' ').replace(/\s+/g, ' ').trim();
}

function escapeRegex(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function extractAfterWake(text) {
  const aiName = normalizeSpeech(getAiName());
  const norm = normalizeSpeech(text);
  if (!aiName) return null;

  const patterns = [
    new RegExp(`^hey[,\\s]+${escapeRegex(aiName)}[,\\s]*(.*)$`),
    new RegExp(`^hey\\s+${escapeRegex(aiName)}[,\\s]*(.*)$`),
  ];
  for (const re of patterns) {
    const m = norm.match(re);
    if (m) return (m[1] || '').trim();
  }

  const inline = norm.indexOf(`hey ${aiName}`);
  if (inline >= 0) {
    return norm.slice(inline + `hey ${aiName}`.length).replace(/^[,.\s]+/, '').trim();
  }
  return null;
}

function armSilenceTimer() {
  clearSilenceTimer();
  if (listenMode !== 'single') return;
  silenceTimer = setTimeout(() => {
    silenceTimer = null;
    if (!listening) return;
    cancelListening();
    setSphereState('idle');
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: "Didn't catch that", type: 'info' },
    }));
    window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'silence' } }));
  }, getSilenceTimeout());
}

function armWakeWaitTimer() {
  clearSilenceTimer();
  silenceTimer = setTimeout(() => {
    silenceTimer = null;
    if (!listening || wakeDetected || awaitingCommand) return;
    cancelListening();
    setSphereState('idle');
    window.dispatchEvent(new CustomEvent('voice:cancelled', { detail: { reason: 'silence' } }));
  }, WAKE_WAIT_TIMEOUT_MS);
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

export async function prepareVoice() {
  const lang = getVoiceLang();
  // Voice loads silently in the background — no progress card, no
  // notification. The orb just stays dimmed (setVoiceReady(false)) until the
  // recognizer is usable; tapping early already gives its own feedback.
  setVoiceReady(false);
  setSphereState('downloading');

  let status;
  try {
    status = await apiFetch(`/api/voice/status?lang=${lang}`);
  } catch (e) {
    status = { vosk: 'missing', stt_lang: 'en' };
  }

  sttLang = status.stt_lang || lang;

  if (status.vosk === 'missing') {
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

export async function startListening(mode) {
  if (listening) return;
  listenMode = mode;
  listening = true;
  resetWakeState();
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

        // Vosk wants a mono 16 kHz buffer; downmix once and reuse it.
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
        recognizer.acceptWaveform(monoBuffer);

        const rms = (sum) => Math.sqrt(sum / n);
        const lvl = (v) => Math.min(1, v * 10);
        const rl = rms(sumL);
        const rr = rms(sumR);
        const total = rl + rr + 1e-6;
        window.dispatchEvent(new CustomEvent('voice:level', {
          detail: {
            level: lvl(rms(sumM)),
            left: lvl(rl),
            right: lvl(rr),
            // -1 hard left … +1 hard right.
            pan: Math.max(-1, Math.min(1, (rr - rl) / total)),
          },
        }));
      } catch (_) {}
    };
    source.connect(processor);
    processor.connect(audioContext.destination);

    if (mode === 'single') armSilenceTimer();
    else if (mode === 'wake') armWakeWaitTimer();
  } catch (e) {
    listening = false;
    resetWakeState();
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
  if (!listening && !processor && !mediaStream) return;
  listening = false;
  resetWakeState();
  clearSilenceTimer();
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
 * can be spoken after the finger lifts. Letting go also restarts the wake
 * window, so a long hold does not eat into the time left to talk.
 *
 * Returns true while a wake session is still listening.
 */
export function releaseWakeHold() {
  if (listenMode !== 'wake' || !listening) return false;
  if (!wakeDetected && !awaitingCommand) armWakeWaitTimer();
  return true;
}

export async function speak(text, lang) {
  if (currentAudio) {
    currentAudio.pause();
    currentAudio = null;
  }

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
    const url = URL.createObjectURL(blob);
    currentAudio = new Audio(url);

    // Keep the orb alive with the ASSISTANT's voice: the mic is already
    // stopped by now, so without this the animation would freeze exactly
    // when the answer arrives. Same signal shape as the microphone, but the
    // assistant speaks from the centre (pan 0).
    const stopPulse = attachTtsPulse(currentAudio);

    await new Promise((resolve, reject) => {
      currentAudio.onended = () => { stopPulse(); URL.revokeObjectURL(url); resolve(); };
      currentAudio.onerror = () => { stopPulse(); reject(new Error('playback failed')); };
      currentAudio.play().catch((e) => { stopPulse(); reject(e); });
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
