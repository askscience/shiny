# API — voice

Source: [`src/api/voice.rs`](../../src/api/voice.rs). Services:
[`whisper.rs`](../../src/services/whisper.rs),
[`qwen_tts.rs`](../../src/services/qwen_tts.rs), and the SDK's
`SupertonicClient`.

See also [voice](../core/voice.md) and [sidecars](../deployment/sidecars.md).

---

## `GET /api/voice/status?lang=<code>`

Authenticated. Reports readiness and model inventory for both speech stacks.

```jsonc
{
  "success": true,
  "vosk": "ready",              // "ready" | "missing"
  "supertonic": "ready",        // "ready" | "unavailable"
  "stt_lang": "en",
  "supertonic_lang": "en",
  "whisper": "ready",           // "ready" | "unavailable"
  "default_engine": "whisper",
  "whisper_default_model": "tiny",
  "whisper_models": { … },      // per-model presence/loaded state
  "whisper_downloads": { … },   // download progress
  "tts_engine": "supertonic",
  "qwen": "unavailable",
  "qwen_models": { … },
  "qwen_downloads": { … },
  "qwen_speakers": [ … ]
}
```

`stt_lang`/`supertonic_lang` are resolved from `voice/lang_map.json`: a language
without a native Vosk model falls back to English for STT while keeping its own
Supertonic voice.

---

## `POST /api/voice/download`

Authenticated. Downloads a **Vosk** model (runs `voice/download_vosk.py`).

```json
{ "lang": "fr" }
```

Response: `{ success: true, data: { "status": "ready" } }` (or an error string).

---

## `POST /api/voice/whisper/download`

Authenticated. Starts a **background** faster-whisper model download (`tiny` is
bundled; `small` is optional). Returns immediately; poll `/api/voice/status`
for progress.

```json
{ "model": "small" }
```

---

## `POST /api/voice/qwen/download`

Authenticated. Starts a background **Qwen3-TTS** model download (e.g.
`1.7b-customvoice`). Returns immediately; poll `/api/voice/status`.

```json
{ "model": "1.7b-customvoice" }
```

---

## `POST /api/voice/stt/chunk`

Authenticated. Streams one microphone chunk (or the final flush) to the
faster-whisper sidecar. The **request body is raw audio**: 16 kHz mono PCM16LE.
Query parameters:

| Param | Meaning |
|---|---|
| `session` | Client-chosen session id. |
| `lang` | ISO-639-1 language (skips detection). |
| `model` | `tiny` (default) or `small`. |
| `prompt` | Decoder bias (`initial_prompt`) — e.g. the wake phrase. |
| `final` | `true` on the last chunk: re-decode the whole utterance. |

Response is the sidecar's JSON (`{ text, final, … }`). The sidecar implements a
LocalAgreement-2 policy, so intermediate calls return a growing partial
transcript and the `final` call returns the accurate result.

---

## `POST /api/voice/stt/close?session=<id>`

Authenticated. Drops an STT session (mic cancelled). Best-effort: a dead
connection returns an empty result rather than an error, because the sidecar
expires idle sessions anyway.

---

## `GET /api/voice/languages`

**Public** (no auth). The supported languages from `voice/lang_map.json`:

```jsonc
{
  "success": true,
  "data": [
    { "code": "en", "supertonic": "en", "vosk_available": true, "vosk_stt_lang": "en" }
  ]
}
```

`vosk_available` is true when a native Vosk model ships (`vosk_zip`);
`vosk_stt_lang` is the model actually used (the language itself, or `en` as the
fallback).

---

## `POST /api/tts`

Authenticated. Synthesizes speech and returns `audio/wav`.

```jsonc
{
  "text": "Hello there",
  "lang": "en",                 // default "en"
  "voice": "M1",                // engine-specific voice/speaker
  "speed": 1.0,
  "engine": "qwen"              // "qwen" for Qwen3-TTS; omit/other ⇒ Supertonic
}
```

- **Supertonic** (default) proxies to the Supertonic sidecar and returns its WAV.
- **Qwen3-TTS** (`engine: "qwen"`) streams raw 24 kHz PCM from the sidecar and
  wraps it in a RIFF/WAV container server-side (`pcm_to_wav`), because the
  buffered WAV path could hang the GPU on a long utterance.

---

## Model files

- `GET /api/voice/models/vosk/*` — **public**, serves downloaded Vosk model
  archives (`VOSK_MODELS_DIR`) to the browser, which caches them.
- faster-whisper models live in `WHISPER_MODELS_DIR` and are used server-side
  only.
