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
// A real (tiny) event bus: recognition.js subscribes to `voice:speaking` and
// `voice:barge-in` when it loads, so a dispatch has to reach those listeners.
const listeners = new Map();
globalThis.window = {
  addEventListener: (type, handler) => {
    if (!listeners.has(type)) listeners.set(type, []);
    listeners.get(type).push(handler);
  },
  removeEventListener: () => {},
  dispatchEvent: (event) => {
    events.push(event);
    for (const handler of listeners.get(event.type) || []) handler(event);
    return true;
  },
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

// A partial is revised as the decoder hears more ("pi" → "pi tempo" → …). It
// must never become a turn: that is what turned one sentence into six.
voskHandlers.partialresult({ result: { partial: `${WAKE_PHRASE} turn on the` } });
await settle();
assert.equal(resultEvents().length, 0, 'a partial is never dispatched as a request');

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

/* ── 4. The conversation window ────────────────────────────────
 * After a wake turn the recogniser re-opens on its own, without the phrase —
 * but never while a reply is audible, and only when Settings asks for it.
 * The engines here are Vosk so the whole loop can be driven deterministically:
 * Vosk emits a result the moment the fake recogniser is told to. */

const { isSelfEcho } = await import('../transcriptGuard.js');
const prefs = await import('../preferences.js');

// The text-level backstop: the assistant's own reply must never be mistaken
// for a request, while a short answer stays valid (it legitimately appears in
// the question the assistant just asked).
const REPLY = 'Domani a Milano piove tutto il giorno, con una temperatura intorno ai quindici gradi.';
assert.equal(isSelfEcho(REPLY.slice(0, 60), REPLY), true, 'a slice of the reply is the assistant echoing itself');
assert.equal(isSelfEcho('si', REPLY), false, 'a short answer is not an echo');
assert.equal(isSelfEcho('', REPLY), false, 'nothing said is not an echo');
assert.equal(isSelfEcho('accendi la luce', REPLY), false, 'an unrelated request is not an echo');

// The defaults are what a wake session gets without touching Settings: in
// conversation, and with no timer — it lasts until a gesture or a stop phrase.
assert.equal(prefs.getConversation(), true, 'conversation mode defaults on');
assert.equal(prefs.getConversationTimeout(), 0, 'and has no time limit by default');

prefs.setConversation(true);
await prepared('vosk');

// A wake turn: phrase alone, then the request.
events.length = 0;
await recognition.startListening('wake');
voskHandlers.result({ result: { text: WAKE_PHRASE } });
await settle();
voskHandlers.result({ result: { text: 'che tempo fa domani' } });
await settle();
assert.equal(resultEvents().length, 1);
assert.equal(resultEvents()[0].detail.mode, 'wake');
assert.equal(resultEvents()[0].detail.correction, false, 'a plain request is not a correction');

// While the reply plays no recogniser is live, so it cannot transcribe (and
// answer) the assistant.
window.dispatchEvent(new globalThis.CustomEvent('voice:speaking', { detail: { speaking: true, text: REPLY } }));
assert.equal(recognition.isListening(), false, 'playback closes the recogniser');
const micBeforePlayback = micAcquisitions;
feedFrame(0.05);
await settle();
assert.equal(micAcquisitions, micBeforePlayback, 'no microphone opens while a reply is audible');

// The reply ends: the window opens by itself and the next request needs no
// wake phrase.
window.dispatchEvent(new globalThis.CustomEvent('voice:speaking', { detail: { speaking: false, text: REPLY } }));
await settle();
assert.equal(recognition.isListening(), true, 'the window re-opens after the reply');
assert.equal(micAcquisitions, micBeforePlayback + 1, 'the window opens one microphone');

events.length = 0;
feedFrame(0.05);
voskHandlers.result({ result: { text: 'e a Roma invece' } });
await settle();
assert.equal(resultEvents().length, 1, 'a follow-up needs no wake word');
assert.equal(resultEvents()[0].detail.text, 'e a Roma invece');
assert.equal(resultEvents()[0].detail.mode, 'continuous', 'window turns are continuous');

// "Basta" ends the window instead of becoming a request.
window.dispatchEvent(new globalThis.CustomEvent('voice:speaking', { detail: { speaking: false, text: 'A Roma invece è sereno.' } }));
await settle();
events.length = 0;
voskHandlers.result({ result: { text: 'basta' } });
await settle();
assert.equal(resultEvents().length, 0, 'the stop phrase closes the window without a turn');
recognition.stopListening();
await settle();

/* ── 5. Interruption marks the next turn as a correction ─────── */

prefs.setConversation(false);
await prepared('vosk');
events.length = 0;
window.dispatchEvent(new globalThis.CustomEvent('voice:barge-in'));
await recognition.startListening('single');
voskHandlers.result({ result: { text: 'no intendo milano' } });
await settle();
assert.equal(resultEvents().length, 1);
assert.equal(resultEvents()[0].detail.correction, true, 'a request after an interruption is a correction');

// The flag is consumed: the turn after it is a plain request again.
events.length = 0;
await recognition.startListening('single');
voskHandlers.result({ result: { text: 'e domani' } });
await settle();
assert.equal(resultEvents()[0].detail.correction, false, 'the correction flag does not leak');
recognition.stopListening();
await settle();

/* ── 6. Nothing is uploaded for a reply that is playing ──────── */

await prepared('whisper');
sttChunks.length = 0;
await recognition.startListening('single');
feedFrame(0.05);
await settle();
assert.ok(sttChunks.length > 0, 'a live session does stream to Whisper');
recognition.stopListening();
await settle();

// Now the same session with a reply playing: the recogniser is closed, so the
// microphone cannot feed the engine at all.
window.dispatchEvent(new globalThis.CustomEvent('voice:speaking', { detail: { speaking: true, text: REPLY } }));
sttChunks.length = 0;
const chunksWhileSpeaking = sttChunks.length;
feedFrame(0.05);
await settle();
assert.equal(sttChunks.length, chunksWhileSpeaking, 'no audio reaches the engine while speaking');
assert.equal(recognition.isListening(), false);

/* ── 6b. A continuous Whisper turn finalizes on trailing silence ───
 * The endpointer has to treat the window like a real request, or the user's
 * speech would never be dispatched at all. */

prefs.setConversation(true);
await prepared('whisper');
events.length = 0;
await recognition.startListening('continuous');
feedFrame(0.05);                          // speech starts the utterance
await new Promise((resolve) => setTimeout(resolve, 950)); // trailing silence
feedFrame(0);                             // the endpointer sees it
await settle();
assert.equal(resultEvents().length, 1, 'a continuous whisper turn finalizes on silence');
assert.equal(resultEvents()[0].detail.mode, 'continuous');
recognition.stopListening();
await settle();

/* ── 7. With the window off, behaviour is exactly as before ──── */

prefs.setConversation(false);
await prepared('vosk');
await recognition.startListening('wake');
voskHandlers.result({ result: { text: WAKE_PHRASE } });
await settle();
voskHandlers.result({ result: { text: 'che ore sono' } });
await settle();
const micWithWindowOff = micAcquisitions;
window.dispatchEvent(new globalThis.CustomEvent('voice:speaking', { detail: { speaking: true, text: 'Sono le tre.' } }));
window.dispatchEvent(new globalThis.CustomEvent('voice:speaking', { detail: { speaking: false, text: 'Sono le tre.' } }));
await settle();
assert.equal(recognition.isListening(), false, 'without the setting no window opens');
assert.equal(micAcquisitions, micWithWindowOff, 'and no microphone is opened');
recognition.stopListening();
await settle();

console.log('Voice conversation-window checks passed');
