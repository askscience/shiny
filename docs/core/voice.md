# Voice

Shiny is a **voice-first** assistant. Speech runs entirely on the machine that
hosts it: recognition through a local faster-whisper sidecar (or Vosk compiled
to WebAssembly in the browser), synthesis through a local Supertonic sidecar,
with an optional higher-quality Qwen3-TTS sidecar. The browser owns the voice
bar, the gesture vocabulary, the microphone gate, wake-word matching and
barge-in; the core owns the HTTP surface, model inventory/downloads and the
proxies to the sidecars.

This document covers the whole stack. The sidecar processes and their launchers
have their own page: [Speech sidecars](../deployment/sidecars.md). The REST
surface is documented in [Voice API](../api/voice.md).

## Architecture

```
┌─ browser (web/js/) ──────────────────────────────────────────────┐
│ voice.js        orchestrator: mic, queue, endpointing, TTS, barge │
│ transcriptGuard.js  hallucination filter (second line of defence) │
│ sphere.js       voice bar + gesture vocabulary (tap/long/double)  │
│ preferences.js  per-user engine/voice/language, low-power AI      │
└───────────────┬──────────────────────────────────────────────────┘
                │ HTTP (Bearer/cookie)
┌───────────────▼─ core (src/) ────────────────────────────────────┐
│ api/voice.rs    /api/voice/* and /api/tts                        │
│ services/whisper.rs    WhisperClient  → 127.0.0.1:7789           │
│ services/qwen_tts.rs   QwenClient     → 127.0.0.1:7787           │
│ services/supertonic.rs re-export of the SDK SupertonicClient      │
│                        → 127.0.0.1:7788                          │
└───────────────┬──────────────────────────────────────────────────┘
                │ local HTTP
┌───────────────▼─ sidecars (voice/) ──────────────────────────────┐
│ whisper_server.py   FastAPI + faster-whisper (CTranslate2)       │
│ supertonic_server.py  `supertonic serve` with ORT override       │
│ qwentts.cpp tts-server  C++17/GGML Qwen3-TTS (Vulkan/CUDA/CPU)   │
└──────────────────────────────────────────────────────────────────┘
```

`voice/lang_map.json` is the shared language table for both directions; the
core, the Vosk downloader and the browser all resolve languages through it.

## Engines at a glance

| Engine | Direction | Where it runs | Sidecar / default port | Transport | Default |
|---|---|---|---|---|---|
| faster-whisper | STT | Local Python (CTranslate2) | `voice/whisper_server.py`, `127.0.0.1:7789` | raw PCM in → JSON partials | yes |
| Vosk | STT | Browser (WASM/Kaldi) | none — model served by core | `KaldiRecognizer` in-browser | fallback |
| Supertonic | TTS | Local Python (`supertonic serve`) | `voice/start_supertonic.sh`, `127.0.0.1:7788` | JSON → `audio/wav` | yes |
| Qwen3-TTS | TTS | Native `qwentts.cpp` (GGML) | `voice/start_qwen_tts.sh`, `127.0.0.1:7787` | JSON → streaming PCM | opt-in |

Source: `src/api/voice.rs`, `src/services/whisper.rs`, `src/services/qwen_tts.rs`,
`crates/shiny-plugin-sdk/src/services.rs` (SupertonicClient).

## Speech recognition (STT)

### faster-whisper streaming (LocalAgreement-2)

Whisper is a batch model with no incremental decoder. The sidecar
(`voice/whisper_server.py`) makes it feel live with the **LocalAgreement-2**
policy (Macháček et al., 2023): it repeatedly decodes the *uncommitted tail* of
the utterance and promotes the longest common prefix of two consecutive
hypotheses to **committed** text. Committed words never change again, so the
partial transcript grows steadily instead of flickering.

- **Wire format** — raw **16 kHz mono PCM16LE**, no container and no base64.
  One session per browser utterance, keyed by an opaque id the client generates.
- **Decode window** — after each commit the buffer is trimmed to the first
  uncommitted word, so only what Whisper is still unsure about is re-decoded.
- **Partials** run greedy (`beam_size=1`, no VAD) so nothing the user said is
  dropped between passes. The **final flush** decodes the whole stored
  utterance once with beam search + VAD for accuracy (`_decode`, accurate=true).
- **Cadence** — a decode only runs once enough new audio arrived:
  `max(WHISPER_MIN_CHUNK_MS, min(2 s, window × 0.3))`. The required interval grows
  with the uncommitted window, so a model that keeps changing its mind cannot
  make the loop quadratic (`whisper_server.py` `stt_chunk`).
- **Window** — a single Whisper window is 30 s; beyond `MAX_WINDOW_SECONDS`
  (28 s) the oldest audio is dropped and the hypothesis restarted. The whole
  utterance is kept (bounded by `WHISPER_MAX_UTTERANCE_SECONDS`, default 120 s)
  for the accurate final pass.
- **Sessions** — idle sessions expire after `WHISPER_SESSION_TTL_SECONDS`
  (default 120 s). CTranslate2 models are not safe for concurrent calls, so a
  process-wide decode lock serialises passes.
- **Skills** — `_HALLUCINATION_PHRASES`, `_segment_is_hallucination` and
  `clean_transcript` drop silence-induced boilerplate (subtitle credits,
  "thanks for watching", bare URLs) before it can be committed.

The core proxy is `WhisperClient` in `src/services/whisper.rs` (60 s chunk
timeout, 3 s health probe). On a connection error it surfaces:

> Faster-whisper is not running. Start it from Settings → Voice, or run
> `./voice/start_whisper.sh`.

### Vosk in-browser

Vosk is the original, fully client-side recogniser — no server-side speech
service and no audio leaves the machine. The model archive is served by the core
from `VOSK_MODELS_DIR` at `/api/voice/models/vosk/<lang>.tar.gz` (public route);
the browser loads it with `Vosk.createModel()` and runs a
`KaldiRecognizer(16000)`.

The `download_vosk.py` packer adds a top-level wrapper directory because
`vosk-browser`'s WASM loader always extracts with `strip_first_component = true`.
Without the wrapper the `ivector/` folder is flattened to the root and Vosk
fails with *"Ivector feature dimension mismatch"*. The browser cache-busts the
URL with `?v=2` because the IndexedDB folder name derives from the whole URL.

Vosk only runs at all because the app's CSP grants `script-src` **both**
`'wasm-unsafe-eval'` and `'unsafe-eval'` (`CONTENT_SECURITY_POLICY` in
`src/api/mod.rs`). `vosk-browser` starts its worker from a blob, the emscripten
glue inside that blob evaluates a string to boot, and a blob worker inherits the
document's policy — while `'wasm-unsafe-eval'` permits WebAssembly compilation
only. With just the wasm allowance, Chromium refuses the worker (*"Refused to
evaluate a string as JavaScript"*), every model load fails, and the user sees
*"Speech model failed to load — pick another engine in Settings → Voice"* for
every engine choice, because the failure is in the loader, not in the engine.
`web/js/tests/cspHomeShelfHash.test.mjs` fails if either directive disappears.

### Engine selection, fallback and upgrade

The browser resolves the STT engine in `prepareVoice()` (`web/js/voice.js`) from
`getEffectiveSttEngine()` — the user's Settings choice, unless low-power AI is
active, in which case it is forced to Vosk (see below).

| Situation | Behaviour |
|---|---|
| `whisper` selected and sidecar ready with the chosen model | streams to faster-whisper |
| `whisper` selected, model not present | falls back to `tiny` (with a toast) or downloads tiny |
| `whisper` selected, sidecar not answering | falls back to Vosk and starts a recovery poll |
| sidecar comes up later | a background poll upgrades the next session to faster-whisper |
| Vosk selected, archive missing | `/api/voice/download` fetches it server-side first |
| Vosk model fails to load | the bar stays enabled in the error state; the next listen gesture retries the load once and, if it still fails, toasts the reason instead of listening |
| low-power AI active (Power Saver) | forced to Vosk without overwriting the setting |
| inside a conversation window | the configured engine, re-opened after each reply (see below) |

Recovery (`scheduleWhisperRecovery`) polls every `WHISPER_RECOVERY_POLL_MS`
(5 s) while idle and **never flips the engine mid-session** — `startListening()`
sets up the recogniser or Whisper session a session actually streams to.
Readiness waits `WHISPER_READY_ATTEMPTS` (20) × 700 ms for a bundled model, or
`WHISPER_DOWNLOAD_ATTEMPTS` (40) when a download is in flight.

### The wake phase always listens with Vosk

The engine above governs the **request**. An armed wake listener is a different
job: it can sit open for minutes waiting for a phrase, so it runs on **Vosk** —
a local WASM recogniser that uploads nothing — whatever engine the user picked.
`prepareVoice()` therefore loads the Vosk model whenever the wake word is
enabled, not only when Vosk is the chosen engine; a failure there is logged
rather than toasted, because the wake phase then simply falls back to the
configured engine.

When the phrase arrives alone, `swapToCommandEngine()` hands the *same* session
to `getEffectiveSttEngine()`: the microphone stays open, no second permission
prompt appears, and everything from that moment is transcribed by the chosen
engine (Whisper, or Vosk when Settings or Power Saver resolve to it). The
endpointing clock restarts at the hand-off, so the pause a user takes before
speaking the request is not read as the end of the utterance. A phrase followed
by a request in one breath is dispatched directly and never opens a Whisper
session at all. `startListening()` records the phase in `sessionEngine`; the
per-frame reader branches on that, never on the preference.

### Languages and the STT language

`voice/lang_map.json` has 32 entries. `resolve_stt_lang()` in `src/api/voice.rs`
returns `(stt_lang, supertonic_lang)`:

- If the entry has `vosk_stt_fallback`, Vosk uses that language (normally `en`).
- If it has a `vosk_zip`, Vosk uses the requested language itself.
- `supertonic` is the TTS voice code for the language (`zh` maps to the English
  voice, `"en"`).
- Unknown languages fall back to `en`.

faster-whisper, by contrast, is handed the **ISO-639-1 code itself** from the
browser (`getVoiceLang()`), which skips language detection. The same setting
drives spoken replies and the assistant's reply language.

Which language that is: `getVoiceLang()` in
[`web/js/api.js`](../../web/js/api.js) returns the explicit per-user override
(`voice.lang.<id>`, Settings → Voice → Language) when one is set, and otherwise
the **interface language** (`ui.locale`, Settings → System). The interface
language itself follows the system/browser language until the user picks one, so
the default is the system language; choosing an interface language switches
recognition, spoken replies and the assistant's reply language together, and a
language picked under Voice keeps the voice separate from the interface. See
[interface language](i18n.md).

The full language table lives in [Sidecars → Languages](../deployment/sidecars.md#languages).

## Speech synthesis (TTS)

The core sends a `TtsRequest` to `src/api/voice.rs::tts`; the browser calls it
from `speak()` with `engine`, `voice`, `speed` and the resolved language.

### Supertonic (default)

Supertonic 3 is bundled, small and always available. The core's
`SupertonicClient` lives in the plugin SDK
(`crates/shiny-plugin-sdk/src/services.rs`); `src/services/supertonic.rs` merely
re-exports it so core and plugins share one implementation.

- **Health** — `GET /docs` (accepts 200 or 404) or `GET /` on the sidecar.
- **Request** — `POST /v1/audio/speech` with
  `{ model: "supertonic-3", input, voice, response_format: "wav", lang, speed }`.
- **Voice** — the default comes from `SUPERTONIC_VOICE` (`M1`). Settings offers
  `M1…M5` and `F1…F5`.
- **Speed** — clamped to `0.7…2.0` (client preference is also clamped there).
- **Response** — `audio/wav` bytes, returned verbatim by `/api/tts`.
- **Failures** — a 400 means the sidecar rejected the request itself: an unknown
  voice, a `lang` outside its list, or a model other than the one it serves. The
  voice the client sends is always one of the sidecar's own (`GET /v1/styles`
  reports exactly `M1…M5`/`F1…F5`), and an empty voice is treated as "no
  preference" rather than passed through as an unknown one. Every failed
  synthesis logs the sidecar's status and message to `data/shiny.log` — the
  browser only ever shows one flat toast, so that line is the only place the
  actual reason survives.
- **Nothing to say** — an empty reply is never synthesized. An image-only or
  song-only answer is stripped of its markup before it reaches the speaker and
  can come out empty, and the sidecar rejects an empty input — which used to end
  those turns in "Voice playback unavailable" for a reply that was only ever a
  picture. `speak()` returns before the request instead.

### Qwen3-TTS (opt-in, higher quality)

Qwen3-TTS runs as the native `qwentts.cpp` `tts-server` (`src/services/qwen_tts.rs`).
It is the only engine with a real Vulkan backend, so an AMD/Intel GPU is usable,
not just CUDA or CPU. `voice/detect_accel.py ggml` picks `Vulkan0 → CUDA0 → CPU`.

Qwen3-TTS has a documented model-intrinsic bug: it intermittently never emits
its codec EOS token, so generation runs to the frame cap and produces minutes of
audio for one sentence (QwenLM/Qwen3-TTS#118). The client defends in two ways:

1. **Per-request frame budget** derived from the text length
   (`FRAMES_PER_CHAR = 5`, `MIN_FRAMES = 50`), passed as `max_new_tokens`.
2. **Sentence fallback** — if a response reaches the budget (`produced + 2 >=
   budget`), the text is retried sentence by sentence; each short chunk is far
   less likely to derail. The splitter keeps decimals and domains intact and
   caps a chunk at 300 chars (`split_sentences`).
3. **Runaway-tail trim** (`trim_runaway_tail`) removes a trailing stretch of
   ≤ −44 dBFS quiet lasting ≥ 3 s, keeping a 250 ms lead-out. It runs on every
   response because a derailed read can also trail into near-silence.

The client requests **streaming PCM** (`response_format: "pcm"`), not buffered
WAV: the buffered path builds one giant vocoder graph for the whole utterance
and hangs the GPU. `/api/tts` wraps the returned 24 kHz mono s16le PCM in a RIFF
header (`pcm_to_wav`) so `new Audio(...)` can play it.

- **Models** — `0.6b-customvoice` (bundled, ~605 MB) and `1.7b-customvoice`
  (opt-in, ~1.2 GB), plus the shared `qwen-tokenizer-12hz-Q4_K_M.gguf` codec.
- **Speakers** — the nine CustomVoice speakers: `serena`, `vivian`, `uncle_fu`,
  `ryan`, `aiden`, `ono_anna`, `sohee`, `eric`, `dylan`. `vivian` is the client
  default when no voice is set.
- **Health** — `GET /v1/audio/voices` on the sidecar.

The engine choice is a client preference; the server reports a default
(`default_engine: "whisper"`, `tts_engine: "supertonic"`) but does not enforce
it. Each engine has its own voice set and its own stored pick
(`voice.tts_voice` for Supertonic, `voice.qwen_voice` for Qwen3-TTS — see
[`web/js/voiceShared.js`](../../web/js/voiceShared.js)), so switching engines in
Settings keeps both choices and a speaker name is never sent to an engine that
does not know it — Supertonic answers an unknown voice with a 400, which would
otherwise mute every reply. A pick made before the voices were split per engine
still lives under `voice.tts_voice`; it is read as a fallback for whichever
engine it validates against, so a Qwen user keeps their speaker instead of
silently reverting to the default one.

## The voice bar and gestures

The bar (`web/js/sphere.js`, styled by `/css/sphere.css`) is the assistant's
presence and **one element**: its body is the theme accent (or every accent at
once) and its own shadow — a halo plus a bloom — rises and swells with the audio
level, leaning with the stereo pan. Riding the top bar's row it takes that row's
height and rounding and follows the bar; on its own edge it takes the width and
height from Settings → Appearance, with the step line's text inside it.
`setSphereState(state)` toggles the container classes; states seen in the code
are `disabled`, `downloading`, `idle`, `listening`, `conversation`,
`processing`, `speaking` and `error`.

| Gesture | Trigger | Action |
|---|---|---|
| Short tap | `pointerup` before `LONG_PRESS_MS`, no double | `openTextInput()` — type to the assistant |
| Long-press | held `LONG_PRESS_MS` (400 ms) | `startListening(getWakeWord() ? 'wake' : 'single')` — releasing does **not** close the mic |
| Double tap | two taps within `DOUBLE_TAP_MS` (500 ms) | `tapOrb`: listen once (`startListening('single')`); tap while listening cancels |
| Double tap while answering | `isTurnActive()` | `stopActiveTurn('tap')`, then listen for the rephrase |
| Talk over the reply | barge-in monitor | `stopActiveTurn('barge-in')` then listen |

The Touch Bar's "Ask" button calls the same `tapOrb` path (`initTouchBar({ talk:
tapOrb })`). `voice:level` events feed `setMicLevel` (clamped to `--voice-level`
/ `--voice-pan`).

## Wake word

Long-press **arms** wake recognition; the listener stays armed until the phrase
is heard or the user taps to cancel. There is deliberately no "waiting for the
wake word" timeout — being timed out mid-thought is the complaint this avoids.

An armed listener runs on **Vosk** and stays entirely local: audio is decoded in
WASM, nothing is uploaded, and no sidecar is involved until the phrase has been
heard. (Before the hand-off, the only traffic an armed listener can produce is
the one-time model fetch.) The request that follows is then transcribed by the
engine chosen in Settings — see
[the wake phase always listens with Vosk](#the-wake-phase-always-listens-with-vosk).

Matching (`web/js/voice.js`):

- The assistant name comes from `getAiName()` (`ai.name`, default `Peak`).
- Text is normalised (lowercase, punctuation to spaces), then the name is
  matched with a per-token Levenshtein tolerance of 1 (2 for tokens ≥ 5 chars) —
  the small models regularly render an unusual name as a near-homophone
  ("Peak'd" → "Beak").
- The name may be the whole utterance, or follow "hey", or follow a clipped
  one/two-letter "hey".
- `extractAfterWake()` returns the request after the phrase, `''` when the
  phrase was heard alone, or `null` when absent.
- A wake session that has fallen back to Whisper (no Vosk model on this machine)
  biases the decoder with `prompt=Hey <name>` so the unusual name survives the
  tiny model. Vosk needs no such help.

After the wake phrase alone, `awaitingCommand` is set,
`armWakeCommandTimer()` gives the command `WAKE_COMMAND_TIMEOUT_MS` (8 s) before
it cancels with *"Didn't catch that"*, and the session swaps to the configured
engine. Partials are **never dispatched** — they are being revised as the
decoder hears more ("pi" → "pi tempo" → "pi tempo so …"), so a partial only
records that a request has started (`wakeRemainderSeen`) and the *final* is what
reaches the agent. A `cleanTranscript()` pass runs first so an invented partial
cannot arm the window.

## Conversation window

Waking the assistant means talking to it: once the wake phrase has been heard
(with `voice.conversation`, **on by default**), a turn does not end with its
answer. When the reply's playback ends, `openConversationWindow()` re-opens
listening on the configured engine, so the next request needs no wake phrase.
Turns inside the window dispatch `mode: "continuous"`, which the core records
with the turn.

The window ends on gestures, not on a clock unless the user picks one:

- **A double tap** means "one thing only": the conversation ends and the double
  tap opens a single request (`leaveConversation()` before
  `startListening('single')`).
- **A tap** switches to typing, which ends the conversation too.
- An utterance that is **only** a stop phrase ("basta", "stop") ends it. The list
  is whole-utterance matched, so "stop the music" still reaches the agent
  (`isStopPhrase()` in `voice/wake.js`).
- `voice.conversation_timeout` is **0 by default ("until I stop it")**.
  A non-zero value arms `armConversationTimer()`, which **restarts on every burst
  of speech** so thinking out loud does not end it; on expiry
  `closeConversation()` returns to the armed wake listener — the same state a
  long-press leaves — or closes the microphone when the wake word is off.
- The orb's `conversation` state is held for as long as the window is armed.

### The microphone rule

The recogniser is open **only while the assistant is silent**. `playback.js`
dispatches `voice:speaking { speaking, text }` around real playback;
`recognition.js` closes any live session on `speaking: true` and re-opens the
window only after `speaking: false`. That is what makes the assistant unable to
transcribe — and answer — itself: not echo cancellation, but the absence of a
listening path.

The one ear open during a reply is the **barge-in monitor** (`playback.js`): a
second microphone stream that never transcribes and stops the reply the moment
the user talks over it (`voice:barge-in` → `handleBargeIn()`). Its threshold is
**measured per reply**: during the first `ECHO_SAMPLE_MS` (800 ms) of playback
the monitor samples the assistant's own residual (what echo cancellation left of
it in the microphone), takes the 40th percentile as the floor, and refuses to
fire below `max(BARGE_IN_RMS, floor × 2.5)` (capped at `BARGE_IN_MAX`).
Headphones therefore interrupt on a quiet voice; a loud laptop speaker raises the
bar by itself, which is what stops the assistant from cutting itself off.

### Interrupt → correct

Stopping an answer makes the next request a **correction** of it.
`markCorrectionPending()` — called by `voice:barge-in` and by the tap/long-press
paths in `app.js` — sets a flag that the next dispatched turn consumes, and that
turn carries `correction: true` to `/api/agent`. The core adds
`correction_block()` to the turn's prompt, so the message is applied to the
request being answered instead of reading as a new question. The interrupted turn
is already saved with the invisible `system` note that says the user stopped it.

### Echo filter

`isSelfEcho(text, spoken)` in `transcriptGuard.js` is the text-level backstop for
the moment the window opens onto the tail of the assistant's audio. A transcript
is dropped when it is substantially the last reply: a verbatim (word-aligned)
slice of ≥ 3 words, or ≥ 80 % of its words present in order with at least 5
words. Short answers ("sì", "no", "ok") are never dropped — those words
legitimately appear in the question the assistant just asked. The comparison runs
for `ECHO_FILTER_WINDOW_MS` (2.5 s) after playback ends, and a dropped transcript
keeps the window listening instead of creating a turn.

## Local VAD and endpointing

Whisper gives no end-of-speech signal, so the browser runs its own gate and
endpointer in the audio callback (`voice.js`). The same triangular-window speech
level also drives the glow and the pan.

| Constant | Value | Meaning |
|---|---|---|
| `WHISPER_SPEECH_RMS` | 0.012 | frame RMS above which the frame counts as speech |
| `WHISPER_PREROLL_MS` | 400 | run-up kept in a ring buffer so the first syllable is not clipped |
| `WHISPER_HANGOVER_MS` | 700 | tail kept after speech stops so trailing words are not cut |
| `WHISPER_ENDPOINT_SILENCE_MS` | 900 | trailing silence that ends an utterance |
| `WHISPER_MAX_QUEUED_SECONDS` | 20 | audio is dropped rather than queued forever if the network lags |
| `WHISPER_CHUNK_MS` | 500 | declared decode-cadence hint (the queue is drained whole) |
| `getSilenceTimeout()` | 3000–30000 ms (default 8000) | listen-once wait before *"Didn't catch that"* |

While idle the audio callback costs one RMS loop per ~256 ms ScriptProcessor
frame; audio is only enqueued while speech is active (plus preroll/hangover).
In wake mode, a pause after "Hey <name>" is treated as the user getting ready,
not the end: `trackEndpointing` flips to `awaitingCommand` and re-arms the
command timer.

## Barge-in

Speaking over the assistant stops it and starts a new request. The microphone
stays open while the assistant thinks and speaks, so
`agent.js::sendToAgent()` calls `startBargeInMonitor()` for the duration of the
turn and `stopBargeInMonitor()` in its `finally`.

| Constant | Value | Meaning |
|---|---|---|
| `BARGE_IN_RMS` | 0.03 | well above the transcription threshold so the assistant's own voice (imperfectly AEC-cancelled) does not cut it off |
| `BARGE_IN_HOLD_MS` | 350 | sustained speech required before firing |
| `BARGE_IN_SETTLE_MS` | 800 | ignore the first moments after the monitor opens |
| `BARGE_IN_TTS_GUARD_MS` | 700 | guard after playback starts, giving the echo canceller time to converge |

When a barge-in fires, `voice.js` dispatches `voice:barge-in`; `app.js`
(`handleBargeIn`) calls `stopActiveTurn('barge-in')` and opens a fresh
`startListening('single')`. `stopActiveTurn` (`agent.js`) also calls
`stopSpeaking()` and `stopBargeInMonitor()`, aborts the in-flight request and
POSTs `/api/agent/stop`, so the core records the interruption. `stopSpeaking()`
bumps `speakToken` so a reply still being fetched is dropped, and resolves the
in-flight `speak()` promise so the awaiting turn never hangs.

## Low-power AI swap

**Power Saver** can swap the speech engines for the lightest ones. This is a
purely client-side decision:

- `web/js/powerShared.js` defines three modes (`performance`, `balanced`,
  `saver`), a 20 % low-battery threshold, and `effectivePowerMode()`.
- `web/js/preferences.js` stores `power.mode`, `power.auto_saver` and
  `power.low_power_ai` (per-user), and exposes `isLowPowerAiActive()`,
  `getEffectiveSttEngine()` and `getEffectiveTtsEngine()`.
- `getEffectiveSttEngine()` returns `vosk` and `getEffectiveTtsEngine()`
  returns `supertonic` while low-power AI is active, **without overwriting the
  user's Settings → Voice choice** — leaving the saver restores it.
- Every change dispatches `power:changed`; `voice.js` listens and calls
  `maybeReconfigureForPower()`. STT is re-initialised only when idle (a live
  session defers it via `pendingPowerRefresh`); TTS is read per reply, so the
  next reply already uses Supertonic.

The swap never touches the stored Voice settings and requires no reload. See
`web/js/powerShared.js`, `web/js/preferences.js` and `web/js/voice.js`.

---

## Recovery & degradation

| Failure | Behaviour |
|---|---|
| faster-whisper sidecar down | `GET /api/voice/status` reports `unavailable`; the browser falls back to Vosk. |
| Supertonic down | TTS fails; STT still works. |
| Qwen3-TTS not started | `tts_engine` stays Supertonic. |
| Mic permission denied | The voice bar reports it; typed input still works. |
| Network lag during STT | Audio is dropped past `WHISPER_MAX_QUEUED_SECONDS` rather than queued forever. |
| Low-power mode | Engines swap without touching settings (above). |

## Source map

| Path | Role |
|---|---|
| [`src/api/voice.rs`](../../src/api/voice.rs) | TTS/STT routes, status, downloads, languages. |
| [`src/services/whisper.rs`](../../src/services/whisper.rs) | faster-whisper client + model inventory. |
| [`src/services/qwen_tts.rs`](../../src/services/qwen_tts.rs) | Qwen3-TTS client + `pcm_to_wav`. |
| [`crates/shiny-plugin-sdk/src/services.rs`](../../crates/shiny-plugin-sdk/src/services.rs) | `SupertonicClient`. |
| [`voice/lang_map.json`](../../voice/lang_map.json) | Language → Supertonic voice / Vosk model. |
| [`voice/whisper_server.py`](../../voice/whisper_server.py) | LocalAgreement-2 streaming server. |
| [`voice/start_whisper.sh`](../../voice/start_whisper.sh), [`start_supertonic.sh`](../../voice/start_supertonic.sh), [`start_qwen_tts.sh`](../../voice/start_qwen_tts.sh) | Launchers. |
| [`voice/detect_accel.py`](../../voice/detect_accel.py) | Vulkan → CUDA → CPU. |
| [`web/js/voice.js`](../../web/js/voice.js), [`transcriptGuard.js`](../../web/js/transcriptGuard.js), [`textInput.js`](../../web/js/textInput.js), [`app.js`](../../web/js/app.js), [`voiceShared.js`](../../web/js/voiceShared.js) | Voice bar, STT upload, barge-in, power swap, per-engine TTS voice sets. |

Related: [sidecars](../deployment/sidecars.md) · [API voice](../api/voice.md) ·
[low-power AI](../host/battery-power.md#power-management-application-profile).

## Internal module layout and cancellation

`web/js/voice.js` retains the public API and forwards to `voice/recognition.js`.
Capture conversion and permission-grant ownership live in `voice/capture.js`,
pure wake-name matching in `voice/wake.js`, and playback, analysis and barge-in
monitoring in `voice/playback.js`. Existing recognition engines, tuning values,
voice events and preference keys are preserved.

Cancelled synthesis requests are aborted, playback object URLs are released on
all completion paths, and old microphone permission grants cannot overwrite or
stop a newer session. Node lifecycle tests exercise these races without opening a
real microphone.
