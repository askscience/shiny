#!/usr/bin/env python3
"""Faster-Whisper streaming STT sidecar for PEAK'D!.

Why a sidecar?
--------------
Vosk runs inside the browser as WASM, but faster-whisper is CTranslate2 —
native code that cannot be shipped to the browser. So the core server proxies
microphone audio here and this process does the recognition on the machine
that already runs the app.

Streaming
---------
Whisper is a batch model: it has no incremental decoder. The recognised way to
make it feel live is the *LocalAgreement-2* policy (Macháček et al., 2023):
keep decoding the uncommitted tail of the utterance, and promote the longest
common prefix of two consecutive hypotheses to "committed" text. Committed
words never change again, so the partial transcript grows steadily instead of
flickering. When the caller flushes, the whole utterance is decoded once more
in one pass, which is more accurate than stitching the committed pieces.

Cost stays bounded because the decode window is trimmed to the first
uncommitted word after every commit — we only ever re-decode what Whisper is
still unsure about, not the whole utterance.

Wire format
-----------
Raw 16 kHz mono PCM16LE in the request body (no container, no base64). One
session per browser utterance, keyed by an opaque id the client generates.

Endpoints
---------
    GET  /health                     readiness + model inventory
    POST /stt/chunk?session=&lang=&model=&final=
    POST /stt/close?session=
"""

from __future__ import annotations

import os
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

# Anaconda ships its own libiomp5 and so does CTranslate2; importing both in one
# process aborts with "OMP: Error #15" unless we allow the duplicate. Must be
# set before ctranslate2 is imported.
os.environ.setdefault("KMP_DUPLICATE_LIB_OK", "TRUE")
os.environ.setdefault("OMP_NUM_THREADS", str(max(1, (os.cpu_count() or 4) // 2)))
os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")

import numpy as np  # noqa: E402
from fastapi import Body, FastAPI, HTTPException, Query  # noqa: E402
from fastapi.responses import JSONResponse  # noqa: E402

SAMPLE_RATE = 16_000

# ── Model inventory ──────────────────────────────────────────────────────────
# Directory names match what voice/download_whisper.py writes, so the tiny
# model bundled with the app and the optional small download are found as-is.
MODELS: dict[str, dict[str, str]] = {
    "tiny": {
        "dir": "faster-whisper-tiny",
        "repo": "Systran/faster-whisper-tiny",
        "label": "Tiny",
        "size": "~75 MB",
    },
    "small": {
        "dir": "faster-whisper-small",
        "repo": "Systran/faster-whisper-small",
        "label": "Small",
        "size": "~480 MB",
    },
}
DEFAULT_MODEL = os.environ.get("WHISPER_MODEL", "tiny").strip().lower() or "tiny"
MODELS_DIR = Path(os.environ.get("WHISPER_MODELS_DIR", "data/whisper-models"))
DEVICE = os.environ.get("WHISPER_DEVICE", "cpu").strip() or "cpu"
COMPUTE_TYPE = os.environ.get("WHISPER_COMPUTE_TYPE", "int8").strip() or "int8"

# Partial decoding cadence: never re-decode until at least this much new audio
# arrived. 500 ms keeps the transcript visibly live without burning the CPU on
# every 128 ms audio callback.
MIN_DECODE_SECONDS = float(os.environ.get("WHISPER_MIN_CHUNK_MS", "500")) / 1000.0
# Decoding half a second of room tone makes Whisper hallucinate ("Thank you
# very much"), so wait for a lead-in before the first partial.
MIN_START_SECONDS = float(os.environ.get("WHISPER_MIN_START_MS", "1000")) / 1000.0
# Never let a long uncommitted tail turn the re-decode loop quadratic: past a
# couple of seconds the required interval grows with the window.
MAX_DECODE_INTERVAL_SECONDS = 2.0
DECODE_INTERVAL_RATIO = 0.3
# A single Whisper window is 30 s; longer uncommitted stretches are trimmed.
MAX_WINDOW_SECONDS = 28.0
# Keep the whole utterance this long so the final pass can re-decode it.
MAX_UTTERANCE_SECONDS = float(os.environ.get("WHISPER_MAX_UTTERANCE_SECONDS", "120"))
# Drop idle sessions so a closed tab cannot pin memory forever.
SESSION_TTL_SECONDS = float(os.environ.get("WHISPER_SESSION_TTL_SECONDS", "120"))

app = FastAPI(title="PEAK'D! faster-whisper STT", version="1.0.0")


# ── Model manager ────────────────────────────────────────────────────────────

_models: dict[str, Any] = {}
_model_lock = threading.Lock()
# CTranslate2 models are not safe to call concurrently on the same instance.
_decode_lock = threading.Lock()


def model_path(key: str) -> Path:
    return MODELS_DIR / MODELS[key]["dir"]


def model_available(key: str) -> bool:
    path = model_path(key)
    return (path / "model.bin").is_file() and (path / "config.json").is_file()


def inventory() -> dict[str, Any]:
    return {
        key: {
            "label": spec["label"],
            "size": spec["size"],
            "present": model_available(key),
            "loaded": key in _models,
        }
        for key, spec in MODELS.items()
    }


def resolve_model_key(requested: str | None) -> str:
    key = (requested or DEFAULT_MODEL).strip().lower()
    if key not in MODELS:
        key = DEFAULT_MODEL if DEFAULT_MODEL in MODELS else "tiny"
    if model_available(key):
        return key
    # Fall back to anything already on disk (tiny is the bundled default).
    for candidate in ("tiny", "small"):
        if model_available(candidate):
            return candidate
    raise HTTPException(
        status_code=503,
        detail=(
            f"No faster-whisper model found in {MODELS_DIR}. "
            "Expected faster-whisper-tiny (bundled) — run voice/download_whisper.py tiny."
        ),
    )


def get_model(key: str):
    with _model_lock:
        cached = _models.get(key)
        if cached is not None:
            return cached
        from faster_whisper import WhisperModel  # imported lazily: slow + native

        model = WhisperModel(
            str(model_path(key)),
            device=DEVICE,
            compute_type=COMPUTE_TYPE,
            download_root=str(MODELS_DIR),
        )
        _models[key] = model
        return model


# ── Streaming sessions ───────────────────────────────────────────────────────


@dataclass
class Session:
    lang: str | None
    model: str
    # Bias text (Whisper's `initial_prompt`). The browser sends the wake phrase
    # so an unusual assistant name survives the tiny model — "Peak'd" otherwise
    # comes back as "Beak".
    prompt: str | None = None
    # Audio from the first uncommitted word onwards (the decode window).
    buf: np.ndarray = field(default_factory=lambda: np.zeros(0, dtype=np.float32))
    # Absolute offset of buf[0] inside the utterance, in seconds.
    buf_start: float = 0.0
    # Words promoted by LocalAgreement; never change again.
    committed: list[str] = field(default_factory=list)
    # Last hypothesis over `buf`: [(word, start, end)] relative to buf[0].
    hyp: list[tuple[str, float, float]] = field(default_factory=list)
    # Whole utterance, kept for the accurate final re-decode.
    utterance: list[np.ndarray] = field(default_factory=list)
    utterance_samples: int = 0
    # Samples appended since the last decode attempt.
    pending_samples: int = 0
    partial: str = ""
    updated: float = field(default_factory=time.time)


_sessions: dict[str, Session] = {}
_sessions_lock = threading.Lock()


def _normalize(word: str) -> str:
    return "".join(ch for ch in word.lower() if ch.isalnum())


def _common_prefix_len(a: list[str], b: list[str]) -> int:
    n = 0
    for x, y in zip(a, b):
        if _normalize(x) != _normalize(y):
            break
        n += 1
    return n


def _join(words: list[str]) -> str:
    text = " ".join(w.strip() for w in words if w.strip())
    return text.strip()


def _decode(model, audio: np.ndarray, lang: str | None, *, accurate: bool, prompt: str | None = None):
    """Run one Whisper pass and return (words, text).

    `words` is a list of (word, start, end) in seconds relative to `audio`.
    Partials run greedy (beam 1) and without VAD so nothing the user said is
    dropped between passes; the final pass uses beam search + VAD for accuracy.
    """
    segments, _info = model.transcribe(
        audio,
        language=lang,
        beam_size=5 if accurate else 1,
        vad_filter=accurate,
        word_timestamps=True,
        condition_on_previous_text=False,
        without_timestamps=False,
        initial_prompt=(prompt or None),
    )
    words: list[tuple[str, float, float]] = []
    text_parts: list[str] = []
    for seg in segments:
        text_parts.append(seg.text)
        for w in seg.words or []:
            words.append((w.word, float(w.start), float(w.end)))
    return words, "".join(text_parts).strip()


def _expire_sessions() -> None:
    now = time.time()
    with _sessions_lock:
        stale = [k for k, s in _sessions.items() if now - s.updated > SESSION_TTL_SECONDS]
        for k in stale:
            _sessions.pop(k, None)


def _run_partial(session: Session) -> str:
    """Advance one decoding step and return the current partial transcript."""
    model = get_model(session.model)
    with _decode_lock:
        words, _text = _decode(
            model, session.buf, session.lang, accurate=False, prompt=session.prompt
        )

    if session.hyp:
        hyp_words = [w for w, _, _ in session.hyp]
        new_words = [w for w, _, _ in words]
        agreed = _common_prefix_len(hyp_words, new_words)
    else:
        agreed = 0

    if agreed:
        committed_words = words[:agreed]
        session.committed.extend(w for w, _, _ in committed_words)
        # Trim the decode window to the first uncommitted word. Everything
        # before it is settled, so re-decoding it would only cost time.
        drop = int(round(committed_words[-1][2] * SAMPLE_RATE))
        drop = max(0, min(drop, len(session.buf)))
        if drop:
            session.buf = session.buf[drop:]
            session.buf_start += drop / SAMPLE_RATE
        shift = drop / SAMPLE_RATE
        session.hyp = [
            (w, s - shift, e - shift) for (w, s, e) in words[agreed:] if e > shift
        ]
    else:
        session.hyp = words

    head = _join(session.committed)
    tail = _join([w for w, _, _ in session.hyp])
    session.partial = (head + " " + tail).strip() if head else tail
    return session.partial


# ── Endpoints ────────────────────────────────────────────────────────────────


@app.get("/health")
def health() -> dict[str, Any]:
    _expire_sessions()
    return {
        "status": "ok",
        "device": DEVICE,
        "compute_type": COMPUTE_TYPE,
        "default_model": DEFAULT_MODEL,
        "models_dir": str(MODELS_DIR),
        "models": inventory(),
        "sessions": len(_sessions),
    }


@app.post("/stt/chunk")
def stt_chunk(
    session: str = Query(..., min_length=1),
    lang: str | None = Query(None),
    model: str | None = Query(None),
    prompt: str | None = Query(None, max_length=200),
    final: bool = Query(False),
    audio: bytes = Body(default=b"", media_type="application/octet-stream"),
) -> JSONResponse:
    _expire_sessions()
    key = resolve_model_key(model)
    language = (lang or "").strip().lower() or None
    bias = (prompt or "").strip() or None

    samples = np.frombuffer(audio, dtype="<i2").astype(np.float32) / 32768.0
    if samples.size == 0 and not final:
        return JSONResponse({"text": "", "partial": not final, "final": False})

    with _sessions_lock:
        sess = _sessions.get(session)
        if sess is None or sess.model != key or sess.lang != language or sess.prompt != bias:
            sess = Session(lang=language, model=key, prompt=bias)
            _sessions[session] = sess
        sess.updated = time.time()

    if samples.size:
        sess.buf = np.concatenate([sess.buf, samples])
        sess.pending_samples += samples.size
        sess.utterance.append(samples)
        sess.utterance_samples += samples.size

    # Keep memory bounded on marathon utterances.
    max_utt = int(MAX_UTTERANCE_SECONDS * SAMPLE_RATE)
    if sess.utterance_samples > max_utt:
        dropped = sess.utterance_samples - max_utt
        while dropped > 0 and sess.utterance:
            head = sess.utterance[0]
            if head.size <= dropped:
                dropped -= head.size
                sess.utterance.pop(0)
            else:
                sess.utterance[0] = head[dropped:]
                dropped = 0
        sess.utterance_samples = sum(a.size for a in sess.utterance)

    if final:
        # One accurate pass over the whole utterance beats stitching the
        # streaming pieces together.
        audio_all = (
            np.concatenate(sess.utterance)
            if sess.utterance
            else np.zeros(0, dtype=np.float32)
        )
        if audio_all.size:
            model_obj = get_model(sess.model)
            with _decode_lock:
                _words, text = _decode(
                    model_obj, audio_all, sess.lang, accurate=True, prompt=sess.prompt
                )
        else:
            text = _join(sess.committed)
        # Fall back to the streamed text when VAD eats a very short utterance.
        if not text.strip():
            text = (sess.partial or _join(sess.committed)).strip()
        with _sessions_lock:
            _sessions.pop(session, None)
        return JSONResponse({"text": text, "partial": False, "final": True})

    # Decode only when enough new audio accumulated; otherwise repeat the last
    # partial so a fast poller still gets an answer. The required interval
    # grows with the uncommitted window so a model that keeps changing its mind
    # cannot make this quadratic.
    window_seconds = len(sess.buf) / SAMPLE_RATE
    if window_seconds < MIN_START_SECONDS:
        return JSONResponse({"text": sess.partial, "partial": True, "final": False})
    required = max(
        MIN_DECODE_SECONDS,
        min(MAX_DECODE_INTERVAL_SECONDS, window_seconds * DECODE_INTERVAL_RATIO),
    )
    if sess.pending_samples < int(required * SAMPLE_RATE):
        return JSONResponse({"text": sess.partial, "partial": True, "final": False})

    if len(sess.buf) > int(MAX_WINDOW_SECONDS * SAMPLE_RATE):
        # Whisper sees 30 s at a time. Beyond that, drop the oldest audio and
        # restart the hypothesis rather than silently truncating the window.
        keep = int(MAX_WINDOW_SECONDS * SAMPLE_RATE)
        dropped = len(sess.buf) - keep
        sess.buf = sess.buf[dropped:]
        sess.buf_start += dropped / SAMPLE_RATE
        sess.hyp = []

    sess.pending_samples = 0
    text = _run_partial(sess)
    return JSONResponse({"text": text, "partial": True, "final": False})


@app.post("/stt/close")
def stt_close(session: str = Query(..., min_length=1)) -> JSONResponse:
    with _sessions_lock:
        sess = _sessions.pop(session, None)
    if sess is None:
        return JSONResponse({"text": "", "partial": False, "final": True})
    return JSONResponse(
        {"text": (sess.partial or _join(sess.committed)).strip(), "partial": False, "final": True}
    )


def main() -> None:
    import uvicorn

    host = os.environ.get("WHISPER_HOST", "127.0.0.1")
    port = int(os.environ.get("WHISPER_PORT", "7789"))
    # Preload so the first spoken word is not delayed by a cold model load.
    if model_available(DEFAULT_MODEL):
        try:
            get_model(DEFAULT_MODEL)
        except Exception as exc:  # pragma: no cover - startup diagnostics only
            print(f"[whisper] preload failed: {exc}", flush=True)
    uvicorn.run(app, host=host, port=port, log_level=os.environ.get("WHISPER_LOG", "warning"))


if __name__ == "__main__":
    main()
