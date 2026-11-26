// host.js — the page side of the game's LiteAuraBridge.
//
// The game (game/src/bridge.rs) recognises no speech, runs no model and has
// no voice of its own. This page gives it LiteAura's:
//
//   * speech in: the shell's own Vosk build and the model of the user's
//     voice language, run here in a worker with a restricted vocabulary;
//     partial and final results go to the game with their times;
//   * the language model: the plugin's /api/happywalk/brain route, which asks
//     the model chosen in Settings and checks the answer;
//   * the voice out: the core's /api/tts with the user's voice settings.
//
// Everything is asynchronous and reports how long it took. Times are
// performance.now() milliseconds, the clock the game uses too.

const LOG_KEEP = 200;
/** Above this level a block of audio counts as speech (0..1 RMS). */
const SPEECH_RMS = 0.012;
/** Audio is handed to the recogniser in blocks of this many frames (64 ms). */
const BLOCK_FRAMES = 1024;
const SAMPLE_RATE = 16000;

const notice = document.getElementById('notice');
const logView = document.getElementById('log');
const logLines = [];

function log(line) {
  const stamped = `${(performance.now() / 1000).toFixed(3)} ${line}`;
  logLines.push(stamped);
  if (logLines.length > LOG_KEEP) logLines.shift();
  console.info(`[happywalk] ${line}`);
  if (!logView.hidden) {
    logView.textContent = logLines.join('\n');
    logView.scrollTop = logView.scrollHeight;
  }
}

function say(text) {
  notice.textContent = text || '';
  notice.hidden = !text;
}

window.addEventListener('keydown', (event) => {
  if (event.key !== 'F2') return;
  logView.hidden = !logView.hidden;
  logView.textContent = logLines.join('\n');
});

/* ── The shell's context and the user's settings ─────────────── */

let context = { locale: document.documentElement.lang || 'en' };
window.addEventListener('message', (event) => {
  const data = event.data;
  if (event.origin !== window.location.origin || data?.type !== 'shiny:context') return;
  context = { locale: data.locale || context.locale };
  // The page wears the shell's theme.
  if (data.theme) {
    document.documentElement.dataset.theme = data.theme;
    document.getElementById('theme-tokens').href = `/themes/${data.theme}/tokens.css`;
    document.getElementById('theme-components').href = `/themes/${data.theme}/components.css`;
  }
  if (data.mode) document.documentElement.style.colorScheme = data.mode;
});

function stored(key) {
  try { return window.localStorage.getItem(key); } catch (_) { return null; }
}

/** A per-profile setting of the shell, as it mirrors them in localStorage. */
function setting(base) {
  let id = null;
  try { id = JSON.parse(stored('traveler') || 'null')?.id; } catch (_) { /* none */ }
  return (id && stored(`${base}.${id}`)) || stored(base);
}

function voiceLang() {
  return setting('voice.lang') || String(context.locale || 'en').split('-')[0];
}

function apiUrl(path) {
  return new URL(`../../api/${String(path).replace(/^\/+/, '')}`, window.location.href).href;
}

/** Same-origin fetch as the logged-in user. */
function apiFetch(path, init = {}) {
  const headers = new Headers(init.headers || {});
  const token = stored('token');
  if (token && !headers.has('Authorization')) headers.set('Authorization', `Bearer ${token}`);
  return fetch(apiUrl(path), { ...init, headers, credentials: 'same-origin' });
}

/* ── The game ────────────────────────────────────────────────── */

let game = null;
const status = { voice: 'starting', lang: '', detail: '', speaking: false };

function report(change) {
  Object.assign(status, change);
  game?.hw_status(JSON.stringify(status));
}

/* ── Speech in ───────────────────────────────────────────────── */

let model = null;
let recognizer = null;
let grammar = [];
let audioContext = null;
/** While the character speaks, the microphone is not fed to the recogniser. */
let deaf = false;
/** What the microphone delivered, for the log and for tests. */
const audio = { blocks: 0, peak: 0, results: 0 };
/** When the player's voice was last heard, and whether it has stopped since. */
let lastLoudAt = 0;
let quietSince = 0;

function loadScript(src) {
  return new Promise((resolve, reject) => {
    const script = document.createElement('script');
    script.src = src;
    script.onload = resolve;
    script.onerror = () => reject(new Error(`could not load ${src}`));
    document.head.appendChild(script);
  });
}

function makeRecognizer() {
  recognizer?.remove();
  recognizer = null;
  if (!model) return;
  // A grammar is a JSON list of the phrases to expect; "[unk]" takes the rest.
  const words = grammar.length ? JSON.stringify([...grammar, '[unk]']) : undefined;
  let lastPartial = '';
  const next = new model.KaldiRecognizer(SAMPLE_RATE, words);
  next.setWords(false);
  const deliver = (text, isFinal) => {
    audio.results += 1;
    const clean = String(text || '').replaceAll('[unk]', ' ').replace(/\s+/g, ' ').trim();
    const now = performance.now();
    if (!clean) {
      // Nothing was understood, but a final result still ends the sentence:
      // the game has to know, or the next one would be read as its sequel.
      if (isFinal && lastPartial) game?.hw_heard('', true, now, 0);
      if (isFinal) lastPartial = '';
      return;
    }
    // The recogniser repeats its partial result with every block of audio:
    // only a change is news.
    if (!isFinal && clean === lastPartial) return;
    lastPartial = isFinal ? '' : clean;
    const speechEnd = quietSince ? lastLoudAt : 0;
    log(`heard "${clean}" ${isFinal ? 'final' : 'partial'}${speechEnd ? ` +${(now - speechEnd).toFixed(0)} ms after speech` : ''}`);
    game?.hw_heard(clean, isFinal, now, speechEnd);
  };
  next.on('partialresult', (message) => deliver(message.result?.partial, false));
  next.on('result', (message) => deliver(message.result?.text, true));
  recognizer = next;
}

async function startListening() {
  const lang = voiceLang();
  report({ voice: 'loading-model', lang });
  const started = performance.now();
  const reply = await apiFetch(`voice/status?lang=${encodeURIComponent(lang)}`);
  // This core route answers with its fields beside `success`, not in `data`.
  const voice = (await reply.json().catch(() => null)) || {};
  const sttLang = voice.stt_lang || lang;
  if (voice.vosk === 'missing') {
    log(`the ${sttLang} speech model is not installed: asking the server to fetch it`);
    await apiFetch('voice/download', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ lang }),
    });
  }
  if (!globalThis.Vosk) await loadScript('/vendor/vosk-browser/vosk.js');
  model = await globalThis.Vosk.createModel(`/api/voice/models/vosk/${sttLang}.tar.gz?v=2`);
  log(`latency model-load ${(performance.now() - started).toFixed(0)} ms (${sttLang})`);

  const stream = await navigator.mediaDevices.getUserMedia({
    video: false,
    audio: { echoCancellation: true, noiseSuppression: true, channelCount: 1, sampleRate: SAMPLE_RATE },
  });
  audioContext = new AudioContext({ sampleRate: SAMPLE_RATE });
  if (mix) tapeSound(mix.context.createMediaStreamSource(stream), false);
  makeRecognizer();
  const source = audioContext.createMediaStreamSource(stream);
  const processor = audioContext.createScriptProcessor(BLOCK_FRAMES, 1, 1);
  // The recogniser gets a buffer of its own, as the shell does: the node's
  // input buffer is only valid inside the callback.
  const block = audioContext.createBuffer(1, BLOCK_FRAMES, SAMPLE_RATE);
  let failed = false;
  processor.onaudioprocess = (event) => {
    if (deaf || !recognizer) return;
    const samples = event.inputBuffer.getChannelData(0);
    block.copyToChannel(samples, 0);
    let sum = 0;
    for (let i = 0; i < samples.length; i++) sum += samples[i] * samples[i];
    const level = Math.sqrt(sum / samples.length);
    audio.blocks += 1;
    audio.peak = Math.max(audio.peak, level);
    const loud = level > SPEECH_RMS;
    const now = performance.now();
    if (loud) {
      lastLoudAt = now;
      quietSince = 0;
    } else if (lastLoudAt && !quietSince) {
      quietSince = now;
    }
    if (loud !== status.speaking) report({ speaking: loud });
    try {
      recognizer.acceptWaveform(block);
    } catch (error) {
      // Once is enough: this runs fifteen times a second.
      if (!failed) log(`the recogniser refused the audio: ${error?.message || error}`);
      failed = true;
    }
  };
  source.connect(processor);
  processor.connect(audioContext.destination);
  report({ voice: 'listening', lang: sttLang, detail: '' });
  log(`listening (${sttLang}${grammar.length ? `, ${grammar.length} phrases` : ', free speech'})`);
}

/* ── The wardrobe ────────────────────────────────────────────── */

// How the character looks is the player's: chosen in the wardrobe the first
// time, kept per profile, and changed whenever they like.

let wardrobe = null;
let wardrobeButton = null;
/** The city the game started in. */
let startCity = '';

function lookKey() {
  let id = null;
  try { id = JSON.parse(stored('traveler') || 'null')?.id; } catch (_) { /* none */ }
  return id ? `happywalk.look.${id}` : 'happywalk.look';
}

function savedLook() {
  try { return JSON.parse(stored(lookKey()) || 'null'); } catch (_) { return null; }
}

function cityKey() {
  return lookKey().replace('happywalk.look', 'happywalk.city');
}

function savedCity() {
  return stored(cityKey()) || '';
}

function keepCity(name) {
  try { window.localStorage.setItem(cityKey(), name); } catch (_) { /* storage blocked */ }
}

function keepLook(look) {
  try { window.localStorage.setItem(lookKey(), JSON.stringify(look)); } catch (_) { /* storage blocked */ }
}

async function openWardrobe() {
  if (!game) return;
  const { createWardrobe, word } = await import('./wardrobe.js');
  const city = savedCity() || startCity;
  const { iconButton } = await import('../../ui/index.js');
  const lang = voiceLang();
  const slot = document.getElementById('wardrobe-slot');
  if (!wardrobeButton) {
    wardrobeButton = iconButton({ icon: 'ui/user', label: word(lang, 'open'), onClick: openWardrobe });
    wardrobeButton.id = 'wardrobe-open';
    slot.appendChild(wardrobeButton);
  }
  if (!wardrobe) {
    const look = savedLook() || JSON.parse(game.hw_look_sample(0));
    wardrobe = createWardrobe({
      options: JSON.parse(game.hw_look_options()),
      look,
      city,
      lang,
      onChange: (next) => game.hw_look(JSON.stringify(next)),
      onRandom: () => {
        const next = JSON.parse(game.hw_look_sample(1 + Math.floor(Math.random() * 4294967295)));
        wardrobe.setLook(next);
        game.hw_look(JSON.stringify(next));
      },
      onDone: (next, town) => {
        keepLook(next);
        // A new name is a new city: the game builds it while the panel
        // closes.
        if (town !== savedCity()) log(`city: ${town}`);
        keepCity(town);
        game.hw_city(town);
        wardrobe.hidden = true;
        wardrobeButton.hidden = false;
        game.hw_wardrobe(false);
        log('look kept');
      },
    });
    slot.appendChild(wardrobe);
  }
  wardrobe.hidden = false;
  wardrobeButton.hidden = true;
  game.hw_wardrobe(true);
}

/* ── The notebook ────────────────────────────────────────────── */

// The case in play, drawn beside the game, a few times a second; how far
// the player has come is kept per profile.

function progressKey() {
  return lookKey().replace('happywalk.look', 'happywalk.progress');
}

async function startNotebook() {
  const { createNotebook } = await import('./notebook.js');
  const notebook = createNotebook({
    game,
    lang: voiceLang(),
    onProgress: (progress) => {
      try { window.localStorage.setItem(progressKey(), JSON.stringify(progress)); } catch (_) { /* storage blocked */ }
    },
  });
  document.getElementById('wardrobe-slot').appendChild(notebook.element);
  const kept = stored(progressKey());
  if (kept) game.hw_progress(kept);
  setInterval(() => notebook.update(game.hw_case()), 300);
}

/* ── Clips ───────────────────────────────────────────────────── */

// The last seconds of play are always on tape, so the funny moment can be
// kept after it happened. Two recorders run half a turn apart, each starting
// over every CLIP_TURN seconds; saving stops the one that has run longer,
// which holds between half a turn and a whole one. (One recorder with its
// oldest pieces dropped would not play: only the first piece of a recording
// carries the header.)
//
// The picture is the game's canvas, bubbles and transcript included; the
// sound is the character's voice and the player's microphone.

const CLIP_TURN = 30;
const CLIP_KEEP = 4;
const clips = [];
const recorders = [];
let clipStream = null;
let mix = null;

function clipMime() {
  return ['video/webm;codecs=vp9,opus', 'video/webm;codecs=vp8,opus', 'video/webm'].find((type) => window.MediaRecorder?.isTypeSupported(type)) || '';
}

function startRecorder(slot) {
  const recorder = new MediaRecorder(clipStream, { mimeType: clipMime(), videoBitsPerSecond: 2_500_000 });
  const tape = { recorder, parts: [], since: performance.now(), timer: 0 };
  recorder.ondataavailable = (event) => { if (event.data.size) tape.parts.push(event.data); };
  recorder.start();
  // Start over when the turn is up, unless it is being saved.
  tape.timer = setTimeout(() => {
    recorder.onstop = () => startRecorder(slot);
    recorder.stop();
  }, CLIP_TURN * 1000);
  recorders[slot] = tape;
}

function startClips() {
  const canvas = document.getElementById('happywalk-canvas');
  if (!window.MediaRecorder || !canvas.captureStream || new URLSearchParams(window.location.search).get('clips') === '0') {
    log('clips: off');
    return;
  }
  clipStream = canvas.captureStream(30);
  try {
    // A mixer of its own, at the device's rate (the recogniser's context
    // runs at 16 kHz): the voice and the microphone go to the tape.
    const context = new AudioContext();
    mix = { context, tape: context.createMediaStreamDestination() };
    for (const track of mix.tape.stream.getAudioTracks()) clipStream.addTrack(track);
  } catch (error) {
    log(`clips: no sound (${error?.message || error})`);
  }
  startRecorder(0);
  setTimeout(() => startRecorder(1), (CLIP_TURN / 2) * 1000);
  log(`clips: recording the last ${CLIP_TURN / 2} to ${CLIP_TURN} s (${clipMime() || 'default format'})`);
}

/** Send a source of sound to the tape (and to the speakers when `aloud`). */
function tapeSound(node, aloud) {
  if (!mix) return;
  node.connect(mix.tape);
  if (aloud) node.connect(mix.context.destination);
}

function saveClip() {
  const tape = recorders.filter(Boolean).sort((a, b) => a.since - b.since)[0];
  if (!tape || tape.recorder.state !== 'recording') {
    game?.hw_clip_saved(false, 0);
    return;
  }
  const slot = recorders.indexOf(tape);
  const seconds = (performance.now() - tape.since) / 1000;
  clearTimeout(tape.timer);
  tape.recorder.onstop = () => {
    const blob = new Blob(tape.parts, { type: tape.recorder.mimeType || 'video/webm' });
    const name = `happy-walk-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')}.webm`;
    const url = URL.createObjectURL(blob);
    clips.push({ name, url, seconds, bytes: blob.size });
    while (clips.length > CLIP_KEEP) URL.revokeObjectURL(clips.shift().url);
    // Hand it to the browser as a download: the shell files it.
    const link = document.createElement('a');
    link.href = url;
    link.download = name;
    document.body.appendChild(link);
    link.click();
    link.remove();
    log(`clip saved: ${name}, ${seconds.toFixed(0)} s, ${(blob.size / 1e6).toFixed(1)} MB`);
    game?.hw_clip_saved(blob.size > 0, seconds);
    startRecorder(slot);
  };
  tape.recorder.stop();
}

/* ── What the game calls ─────────────────────────────────────── */

let playing = null;

window.happywalkHost = {
  log,

  /** The phrases to expect (a JSON list); empty means free speech. */
  setGrammar(wordsJson) {
    try { grammar = JSON.parse(wordsJson) || []; } catch (_) { grammar = []; }
    if (model) makeRecognizer();
    log(`grammar: ${grammar.length ? grammar.join(', ') : 'free speech'}`);
  },

  /** Ask the character's brain; the answer goes to hw_brain_answered. */
  async brain(id, requestJson) {
    const started = performance.now();
    let answer = '';
    let note = '';
    try {
      const request = JSON.parse(requestJson);
      if (!request.lang) request.lang = voiceLang();
      const reply = await apiFetch('happywalk/brain', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(request),
      });
      const data = (await reply.json().catch(() => null))?.data;
      if (data?.answer) answer = JSON.stringify(data.answer);
      note = data ? `${data.model || 'no model'}${data.reason ? `, ${data.reason}` : ''}, server ${data.ms} ms` : `HTTP ${reply.status}`;
    } catch (error) {
      note = String(error?.message || error);
    }
    const ms = performance.now() - started;
    log(`brain #${id} ${ms.toFixed(0)} ms (${note})`);
    game?.hw_brain_answered(id, answer, ms);
  },

  /** Speak a line with the user's voice settings; hw_spoke when it starts. */
  async say(id, text) {
    const started = performance.now();
    let ok = false;
    try {
      const engine = setting('voice.tts_engine') === 'qwen' ? 'qwen' : 'supertonic';
      const body = { text, lang: voiceLang(), engine };
      const voice = setting(engine === 'qwen' ? 'voice.qwen_voice' : 'voice.tts_voice');
      const speed = Number(setting('voice.tts_speed'));
      if (voice) body.voice = voice;
      if (Number.isFinite(speed) && speed > 0) body.speed = Math.min(2, Math.max(0.7, speed));
      const reply = await apiFetch('tts', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!reply.ok) throw new Error(`HTTP ${reply.status}`);
      const url = URL.createObjectURL(await reply.blob());
      playing?.pause();
      const audio = new Audio(url);
      playing = audio;
      if (mix) {
        // Through the mixer, so the line is on the tape as well.
        if (mix.context.state === 'suspended') await mix.context.resume().catch(() => {});
        tapeSound(mix.context.createMediaElementSource(audio), true);
      }
      // The character must not hear itself: the microphone is ignored while
      // its line plays.
      const done = () => {
        URL.revokeObjectURL(url);
        if (playing === audio) { playing = null; deaf = false; }
      };
      audio.addEventListener('ended', done);
      audio.addEventListener('error', done);
      deaf = true;
      await audio.play();
      ok = true;
    } catch (error) {
      deaf = false;
      log(`voice #${id} failed: ${error?.message || error}`);
    }
    const ms = performance.now() - started;
    log(`voice #${id} ${ok ? 'speaking' : 'failed'} after ${ms.toFixed(0)} ms`);
    game?.hw_spoke(id, ok, ms);
  },

  /** The notebook of the case in play, as the game publishes it. */
  get case() {
    try { return JSON.parse(game?.hw_case() || 'null'); } catch (_) { return null; }
  },

  /** Open the wardrobe, where the player dresses the character. */
  openWardrobe,

  /** Keep the last seconds of play as a clip; hw_clip_saved follows. */
  saveClip,

  get clips() { return clips.map((clip) => ({ ...clip })); },

  /** For tests and the keyboard: as if the recogniser had heard `text`. */
  hear(text, isFinal = true) {
    const now = performance.now();
    log(`heard "${text}" ${isFinal ? 'final' : 'partial'} (typed)`);
    game?.hw_heard(String(text), !!isFinal, now, 0);
  },

  /** Where the character is and what it is doing (tests and debugging). */
  get state() {
    try { return JSON.parse(game?.hw_state() || 'null'); } catch (_) { return null; }
  },
  /** Put the character somewhere (tests and screenshots). */
  place(x, z, facing = 0) { game?.hw_place(x, z, facing); },

  /** How much the game draws (meshes, on screen, triangles): for measuring. */
  get render() {
    try { return JSON.parse(game?.hw_render() || 'null'); } catch (_) { return null; }
  },

  /** Where the cars and bikes are (tests and screenshots). */
  get traffic() {
    try { return JSON.parse(game?.hw_traffic() || '[]'); } catch (_) { return []; }
  },

  get status() { return { ...status }; },
  get audio() { return { ...audio, context: audioContext?.state || 'none' }; },
  get logLines() { return [...logLines]; },
};

/* ── Start ───────────────────────────────────────────────────── */

async function start() {
  say('Happy Walk…');
  const built = await fetch('game/build.json', { cache: 'no-store' }).then((r) => (r.ok ? r.json() : null)).catch(() => null);
  if (!built?.build) {
    say('The game is not built yet (plugins/happywalk/scripts/build-web.sh).');
    return;
  }
  const started = performance.now();
  const module = await import(`./game/${built.build}/happywalk_game.js`);
  await module.default();
  game = module;
  log(`latency game-load ${(performance.now() - started).toFixed(0)} ms (build ${built.build})`);
  say('');
  game.hw_status(JSON.stringify(status));
  // The city is the player's: its name (chosen in the wardrobe) is its
  // seed, so the same name is the same city every time. Before a name is
  // chosen the game starts in one it suggests. `?seed=` in the address
  // overrides it (tests).
  const { suggestTown } = await import('./wardrobe.js');
  startCity = savedCity() || suggestTown(voiceLang());
  const asked = Number(new URLSearchParams(window.location.search).get('seed'));
  const seed = Number.isInteger(asked) && asked > 0 ? asked >>> 0 : game.hw_city_seed(startCity);
  log(`city "${startCity}", seed ${seed}`);
  // Bevy takes over the page's event loop from here (it throws to do so).
  queueMicrotask(() => {
    try { game.hw_start(seed, window.devicePixelRatio || 1); } catch (error) {
      if (!String(error?.message || error).includes('Using exceptions for control flow')) throw error;
    }
  });
  // Its name, so the character knows where it lives (the seed already
  // matches, so nothing is rebuilt).
  if (!asked) game.hw_city(startCity);
  // The look kept from last time; with none, the first thing the player
  // does is choose one.
  const look = savedLook();
  if (look) game.hw_look(JSON.stringify(look));
  await openWardrobe().catch((error) => log(`wardrobe failed: ${error?.message || error}`));
  if (look) {
    wardrobe.hidden = true;
    wardrobeButton.hidden = false;
    game.hw_wardrobe(false);
  }
  await startNotebook().catch((error) => log(`notebook failed: ${error?.message || error}`));
  startClips();
  try {
    await startListening();
  } catch (error) {
    report({ voice: 'error', detail: String(error?.message || error) });
    log(`voice failed: ${error?.message || error}`);
  }
}

start();
