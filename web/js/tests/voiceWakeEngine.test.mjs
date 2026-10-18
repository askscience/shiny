/**
 * The wake listener must not cost a Whisper pass per utterance.
 *
 * A wake session sits armed in Vosk while it waits for the phrase — locally, in
 * WASM, with nothing uploaded — and hands the session to the engine the user
 * chose the moment the phrase lands, so the request itself is transcribed by
 * Whisper (unless Settings/Power Saver resolve to Vosk).
 *
 * This drives the real module with a fake Vosk model, a fake microphone and a
 * stubbed API, and asserts which engine each phase actually streams to. No
 * bundler, no browser: `scripts/test-web.sh` runs it with plain node.
 */
import './helpers/i18n.mjs';
import assert from 'node:assert/strict';

/* ── Browser surface the voice modules touch ─────────────────── */

// The preference store writes the desktop surface onto :root as it loads; the
// i18n harness only provides the element, so give it style/dataset too.
document.documentElement.style = { setProperty() {}, removeProperty() {} };
document.documentElement.dataset = {};

// The voice bar resolves its element at import time and then toggles state
// classes on it, so the stub has to behave like a real classList.
class ClassList extends Set {
  remove(...names) { for (const name of names) this.delete(name); }
  toggle(name, force) {
    const on = force === undefined ? !this.has(name) : !!force;
    if (on) this.add(name); else this.delete(name);
    return on;
  }
}
document.getElementById = () => ({
  classList: new ClassList(),
  addEventListener() {},
  style: { setProperty() {} },
});

const events = [];
globalThis.window = {
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: (event) => { events.push(event); return true; },
  matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
};
globalThis.CustomEvent = class {
  constructor(type, options) { this.type = type; this.detail = options?.detail; }
};

let lastProcessor = null;
globalThis.AudioContext = class {
  constructor() { this.sampleRate = 16000; this.destination = {}; }
  createMediaStreamSource() { return { connect() {}, disconnect() {} }; }
  createScriptProcessor() {
    lastProcessor = { onaudioprocess: null, connect() {}, disconnect() {} };
    return lastProcessor;
  }
  createBuffer(channels, length) {
    return {
      length,
      numberOfChannels: channels,
      getChannelData: () => new Float32Array(length),
    };
  }
  close() {}
};

let micAcquisitions = 0;
let stoppedTracks = 0;
navigator.mediaDevices = {
  getUserMedia: async () => {
    micAcquisitions++;
    return { getTracks: () => [{ stop: () => stoppedTracks++ }] };
  },
};

/* ── Fake Vosk: records what the wake phase fed it ───────────── */

let voskFrames = 0;
let voskModelLoads = 0;
const voskHandlers = {};
globalThis.Vosk = {
  async createModel(url) {
    voskModelLoads++;
    assert.match(url, /\/api\/voice\/models\/vosk\/en\.tar\.gz/, 'wake phase loads the Vosk model');
    return {
      terminate() {},
      KaldiRecognizer: class {
        setWords() {}
        on(event, handler) { voskHandlers[event] = handler; }
        acceptWaveform() { voskFrames++; }
      },
    };
  },
};

/* ── Stubbed API: records every STT round trip ──────────────── */

const sttChunks = [];
function json(data) {
  return {
    ok: true,
    status: 200,
    statusText: 'OK',
    text: async () => JSON.stringify(data),
    blob: async () => new Blob([JSON.stringify(data)]),
    body: null,
  };
}
globalThis.fetch = async (url, options = {}) => {
  const path = String(url);
  if (path.includes('/api/voice/status')) {
    return json({
      success: true, whisper: 'ready', vosk: 'ready', stt_lang: 'en', supertonic: 'ready',
      whisper_models: { tiny: { present: true } }, whisper_downloads: {},
      qwen: 'unavailable', qwen_models: {}, qwen_speakers: [], qwen_downloads: {},
    });
  }
  if (path.includes('/api/voice/stt/chunk')) {
    sttChunks.push({ url: path, bytes: options.body?.byteLength ?? 0 });
    return json({ text: 'turn on the lights', final: false });
  }
  if (path.includes('/api/voice/stt/close')) return json({ text: '', final: true });
  if (path.includes('/api/voice/whisper/download')) return json({ success: true });
  if (path.includes('/api/voice/download')) return json({ success: true, data: { status: 'ready' } });
  return json({ success: true });
};

const recognition = await import('../voice/recognition.js');
const { getAiName, setSttEngine, setWakeWord } = await import('../preferences.js');

const name = getAiName();
assert.equal(name.toUpperCase(), "PEAK'D!", 'harness relies on the default assistant name');
const WAKE_PHRASE = 'hey peak d';

/* ── Driving a session ───────────────────────────────────────── */

/** Feed one 4096-frame audio block at `amp` (square wave, so RMS == amp). */
function feedFrame(amp) {
  const frame = new Float32Array(4096);
  if (amp > 0) for (let i = 0; i < frame.length; i++) frame[i] = i % 2 ? amp : -amp;
  const buffer = { length: frame.length, numberOfChannels: 1, getChannelData: () => frame };
  lastProcessor.onaudioprocess({ inputBuffer: buffer });
}

const settle = () => new Promise((resolve) => setImmediate(resolve));
const resultEvents = () => events.filter((e) => e.type === 'voice:result');

async function prepared(engine) {
  setSttEngine(engine);
  setWakeWord(true);
  await recognition.prepareVoice();
  assert.ok(voskModelLoads > 0, 'prepareVoice loads the Vosk model for the wake listener');
}

/* ── 1. Whisper selected: the wake phase still runs on Vosk ──── */

await prepared('whisper');
await recognition.startListening('wake');
assert.equal(micAcquisitions, 1);

const voskFramesBeforeIdle = voskFrames;
feedFrame(0);          // room noise
feedFrame(0.05);       // somebody talking near the device
await settle();
assert.ok(voskFrames > voskFramesBeforeIdle, 'the wake listener runs locally in Vosk');
assert.equal(sttChunks.length, 0, 'an armed wake listener uploads nothing');

// The phrase arrives alone: the wake listener hands the session over.
voskHandlers.partialresult({ result: { partial: WAKE_PHRASE } });
voskHandlers.result({ result: { text: WAKE_PHRASE } });
assert.equal(micAcquisitions, 1, 'the hand-off does not reopen the microphone');

const voskFramesAtHandOff = voskFrames;
feedFrame(0.05);       // the user starts the request
await settle();
assert.ok(sttChunks.length > 0, 'the request itself streams to Whisper');
assert.match(sttChunks[0].url, /model=tiny/, 'the configured Whisper model is used');
assert.match(sttChunks[0].url, /prompt=/, 'the wake session still biases the decoder');
assert.equal(voskFrames, voskFramesAtHandOff, 'Vosk stops receiving audio after the hand-off');
recognition.stopListening();
await settle();

/* ── 2. Vosk selected: nothing is ever sent to Whisper ──────── */

sttChunks.length = 0;
await prepared('vosk');
await recognition.startListening('wake');
feedFrame(0.05);
voskHandlers.partialresult({ result: { partial: WAKE_PHRASE } });
voskHandlers.result({ result: { text: WAKE_PHRASE } });
const voskFramesOnCommand = voskFrames;
feedFrame(0.05);
await settle();
assert.equal(sttChunks.length, 0, 'a Vosk user never opens a Whisper session');
assert.ok(voskFrames > voskFramesOnCommand, 'the Vosk command phase keeps decoding locally');
recognition.stopListening();
await settle();

/* ── 3. Phrase and request in one breath ────────────────────── */

events.length = 0;
sttChunks.length = 0;
await prepared('whisper');
await recognition.startListening('wake');
voskHandlers.result({ result: { text: `${WAKE_PHRASE} turn on the lights` } });
await settle();
const said = resultEvents();
assert.equal(said.length, 1, 'one wake turn dispatches one request');
assert.equal(said[0].detail.text, 'turn on the lights', 'the agent gets the request, not the phrase');
assert.equal(said[0].detail.mode, 'wake');
feedFrame(0.05);
await settle();
assert.equal(sttChunks.length, 0, 'a one-breath request needs no Whisper session');

assert.equal(stoppedTracks >= 1, true, 'sessions release their microphone');
console.log('Voice wake-engine checks passed');
