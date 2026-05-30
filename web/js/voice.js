import { apiFetch, getVoiceLang, setVoiceLang } from './api.js';
import { setSphereState, setVoiceReady } from './sphere.js';
import { getAiName } from './preferences.js';

const SILENCE_TIMEOUT_MS = 8000;
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

function createDownloadCard(lang) {
  const existing = document.getElementById('voice-download-card');
  if (existing) existing.remove();

  const container = document.getElementById('insight-cards');
  if (!container) return null;

  const card = document.createElement('div');
  card.id = 'voice-download-card';
  card.className = 'insight-card';
  card.innerHTML = `
    <div class="insight-card-icon">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="20" height="20">
        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
        <polyline points="7 10 12 15 17 10"/>
        <line x1="12" y1="15" x2="12" y2="3"/>
      </svg>
    </div>
    <div class="insight-card-body">
      <div class="insight-card-title">Preparing voice (${lang.toUpperCase()})</div>
      <div class="voice-download-bar">
        <div class="voice-download-bar-fill" style="width: 10%"></div>
      </div>
      <div class="voice-download-text">Checking models…</div>
    </div>
  `;
  container.appendChild(card);
  return card;
}

function updateDownloadCard(card, pct, status) {
  if (!card) return;
  const fill = card.querySelector('.voice-download-bar-fill');
  const text = card.querySelector('.voice-download-text');
  if (fill) fill.style.width = `${Math.round(pct)}%`;
  if (text && status) text.textContent = status;
}

function removeDownloadCard(card) {
  if (card) card.remove();
}

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
  }, SILENCE_TIMEOUT_MS);
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
    if (isFinal && text) dispatchVoiceResult(text);
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
  setVoiceReady(false);
  setSphereState('downloading');

  const card = createDownloadCard(lang);

  let status;
  try {
    status = await apiFetch(`/api/voice/status?lang=${lang}`);
  } catch (e) {
    status = { vosk: 'missing', stt_lang: 'en' };
  }

  sttLang = status.stt_lang || lang;

  if (status.vosk === 'missing') {
    updateDownloadCard(card, 30, 'Downloading Vosk speech model…');
    try {
      await apiFetch('/api/voice/download', {
        method: 'POST',
        body: JSON.stringify({ lang }),
      });
    } catch (e) {
      updateDownloadCard(card, 0, 'Download failed');
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: 'Voice model download failed', type: 'error' },
      }));
      await sleep(2000);
      removeDownloadCard(card);
      setVoiceReady(true);
      setSphereState('error');
      return;
    }
  }

  updateDownloadCard(card, 70, 'Loading speech recognizer…');
  try {
    await initVosk(sttLang);
  } catch (e) {
    updateDownloadCard(card, 0, 'Model load failed');
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: 'Speech model failed to load', type: 'error' },
    }));
    await sleep(2000);
    removeDownloadCard(card);
    setVoiceReady(true);
    setSphereState('error');
    return;
  }

  updateDownloadCard(card, 100, 'Ready');
  await sleep(600);
  removeDownloadCard(card);
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
  resetWakeState();
  setSphereState(mode === 'single' ? 'listening' : 'conversation');

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
      handleTranscript(text, true);
    });

    recognizer.on('partialresult', (msg) => {
      const text = msg.result?.partial?.trim();
      if (listenMode === 'wake') handleTranscript(text, false);
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

/** Long-press release: cancel wake wait if phrase not heard yet. */
export function releaseWakeHold() {
  if (listenMode !== 'wake' || !listening) return;
  if (!wakeDetected && !awaitingCommand) {
    cancelListening();
    setSphereState('idle');
  }
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

export function isWakeAwaitingCommand() {
  return listenMode === 'wake' && (wakeDetected || awaitingCommand);
}
