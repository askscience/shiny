import { apiFetch, getVoiceLang, setVoiceLang } from './api.js';
import { setSphereState, setVoiceReady } from './sphere.js';

const SILENCE_TIMEOUT_MS = 8000;

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

const overlay = document.getElementById('voice-overlay');
const percentEl = document.getElementById('voice-prep-percent');
const statusEl = document.getElementById('voice-overlay-status');
const titleEl = document.getElementById('voice-overlay-title');

function setProgress(pct, status) {
  if (percentEl) percentEl.textContent = `${Math.round(pct)}%`;
  if (statusEl && status) statusEl.textContent = status;
}

function clearSilenceTimer() {
  if (silenceTimer) {
    clearTimeout(silenceTimer);
    silenceTimer = null;
  }
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
  }, SILENCE_TIMEOUT_MS);
}

export async function prepareVoice() {
  const lang = getVoiceLang();
  setVoiceReady(false);
  setSphereState('downloading');
  overlay.classList.remove('hidden');
  titleEl.textContent = `Preparing voice (${lang.toUpperCase()})`;
  setProgress(10, 'Checking models…');

  let status;
  try {
    status = await apiFetch(`/api/voice/status?lang=${lang}`);
  } catch (e) {
    status = { vosk: 'missing', stt_lang: 'en' };
  }

  sttLang = status.stt_lang || lang;

  if (status.vosk === 'missing') {
    setProgress(30, 'Downloading Vosk speech model…');
    await apiFetch('/api/voice/download', {
      method: 'POST',
      body: JSON.stringify({ lang }),
    });
  }

  setProgress(70, 'Loading speech recognizer…');
  await initVosk(sttLang);

  setProgress(100, 'Ready');
  await sleep(400);
  overlay.classList.add('hidden');
  setVoiceReady(true);
  setSphereState('idle');
}

async function initVosk(lang) {
  if (voskModel) {
    voskModel.terminate?.();
    voskModel = null;
  }

  const modelUrl = `/api/voice/models/vosk/${lang}.tar.gz`;
  voskModel = await Vosk.createModel(modelUrl);
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

export async function startListening(mode) {
  if (listening) return;
  listenMode = mode;
  listening = true;
  setSphereState(mode === 'continuous' ? 'conversation' : 'listening');

  try {
    mediaStream = await navigator.mediaDevices.getUserMedia({
      video: false,
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        channelCount: 1,
        sampleRate: 16000,
      },
    });

    audioContext = new AudioContext({ sampleRate: 16000 });
    recognizer = new voskModel.KaldiRecognizer(16000);
    recognizer.setWords(false);

    recognizer.on('result', (msg) => {
      const text = msg.result?.text?.trim();
      if (text) {
        clearSilenceTimer();
        stopListening();
        window.dispatchEvent(new CustomEvent('voice:result', { detail: { text, mode: listenMode } }));
      }
    });

    const source = audioContext.createMediaStreamSource(mediaStream);
    processor = audioContext.createScriptProcessor(4096, 1, 1);
    processor.onaudioprocess = (e) => {
      if (!listening) return;
      try {
        recognizer.acceptWaveform(e.inputBuffer);
        const data = e.inputBuffer.getChannelData(0);
        let sum = 0;
        for (let i = 0; i < data.length; i++) sum += data[i] * data[i];
        const level = Math.min(1, Math.sqrt(sum / data.length) * 10);
        window.dispatchEvent(new CustomEvent('voice:level', { detail: level }));
      } catch (_) {}
    };
    source.connect(processor);
    processor.connect(audioContext.destination);

    armSilenceTimer();
  } catch (e) {
    listening = false;
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

export async function speak(text, lang) {
  if (currentAudio) {
    currentAudio.pause();
    currentAudio = null;
  }

  const voiceLang = lang || getVoiceLang();
  try {
    const blob = await apiFetch('/api/tts', {
      method: 'POST',
      body: JSON.stringify({ text, lang: voiceLang, voice: 'M1' }),
      responseType: 'blob',
    });
    const url = URL.createObjectURL(blob);
    currentAudio = new Audio(url);
    await new Promise((resolve, reject) => {
      currentAudio.onended = () => { URL.revokeObjectURL(url); resolve(); };
      currentAudio.onerror = reject;
      currentAudio.play().catch(reject);
    });
  } catch (e) {
    console.warn('TTS failed:', e);
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: 'Voice playback unavailable', type: 'error' },
    }));
  }
}

export async function changeLanguage(lang) {
  setVoiceLang(lang);
  await prepareVoice();
}

export function isListening() {
  return listening;
}

export function getListenMode() {
  return listenMode;
}
