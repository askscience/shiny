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
import re  # noqa: E402
import unicodedata  # noqa: E402
from fastapi import Body, FastAPI, HTTPException, Query  # noqa: E402
from fastapi.responses import JSONResponse  # noqa: E402

SAMPLE_RATE = 16_000

# ── Hallucination filter ─────────────────────────────────────────────────────
# Decoding silence (and near-silence) makes Whisper invent text, and because it
# was trained on mountains of subtitled video the invented text is usually the
# boilerplate from a subtitle track: credits, "thanks for watching", "like and
# subscribe", broadcaster sign-offs. It is not Italian-specific — the same
# failure is reported for English, German, French, Spanish, Portuguese, Dutch,
# Russian, Ukrainian, Czech, Romanian, Turkish, Arabic, Chinese, Welsh,
# Norwegian and Danish, which is why the detector below is built from
# LANGUAGE-INDEPENDENT signals (URLs, domains, credits-by phrasing, subscribe
# boilerplate) rather than a list of sentences. The reported offender,
# "Sottotitoli e revisione a cura di QTSS", appears hundreds of times in the
# same transcript when it triggers.
#
# Two independent guards, because either one alone has a failure mode:
#   1. segment confidence — the model itself says "this window is not speech"
#      AND "I am not confident"; quiet-but-real speech survives.
#   2. text patterns — the transcript is boilerplate, or the phrase is embedded
#      in otherwise real speech (so the credits are stripped, not the turn).
# Texts from failed turns are also de-duplicated inside one transcript.

# Credit/boilerplate phrasing. Each entry is a regex over `_normalize`d text
# (lowercase, accents folded, punctuation dropped, whitespace collapsed).
_HALLUCINATION_PHRASES = [
    # "Sottotitoli e revisione a cura di QTSS", "Sottotitoli a cura di …",
    # "Sottotitoli creati dalla comunità Amara.org", "Revisione a cura di …"
    r"\bsottotitol\w*(\s+\w+){0,4}\s+a\s+cura\s+di\b",
    r"\brevisione\s+a\s+cura\s+di\b",
    r"\bsottotitol\w*\s+(creat|realizz|offert|fornit)\w*\b",
    # "subtitles by", "subs by", "captions by", "translated by",
    # "transcription by" (English / Italian "tradotto da" / "tradotto da")
    r"\bsub(s|titles?|titled)\s+(by|from)(?=\s|$)",
    r"\bcaptions?\s+by(?=\s|$)",
    r"\btranslat(ed|ion|or|ions)\s+(by|from)(?=\s|$)",
    r"\btradott\w*\s+da\b",
    r"\btraduc\w*\s+por\b",
    r"\btraduction\s+(par|de)\b",
    r"ubersetzung\s+(von|durch)\b",
    r"\buntertitel\w*\s+(von|f.r|durch)\b",
    r"\bsubtitul\w*\s+por\b",
    r"\blegendas?\s+por\b",
    r"\btranslated\s+by\s+the\s+\w+\s+community\b",
    # "Translated by Amara.org Community", "❤️ Translated by …"
    r"\bamara\s*(org|com)?\b",
    r"\bqtss\b",
    # YouTube-style calls to action
    r"\blike\s+and\s+subscribe\b",
    r"\bdon t\s+forget\s+to\s+(like|subscribe)\b",
    r"\bsubscribe\s+to\s+(the|my|our)\s+channel\b",
    r"\bthanks?\s+for\s+watching\b",
    r"\bthank\s+you\s+for\s+watching\b",
    r"\bthanks?\s+for\s+viewing\b",
    r"\bgrazie\s+per\s+(la\s+)?visione\b",
    r"\bdanke\s+f.rs?\s+zuschauen\b",
    r"\bmerci\s+d\s+avoir\s+regard\b",
    r"\bmerci\s+d\s+avoir\s+regarde\b",
    r"\bgracias\s+por\s+(ver|su\s+visita)\b",
    r"\bobrigad\w*\s+por\s+(assistir|ver)\b",
    r"\bпродолжение\s+следует\b",
    r"\bдякую\s+за\s+перегляд\b",
    r"\bспасибо\s+за\s+просмотр\b",
    r"\bsubtitles?\s+created\s+by\b",
    r"\bsubtitles?\s+provided\s+by\b",
    r"\bthe\s+end\b",
    r"\bsilence\b",
    r"\bblank\s+audio\b",
    # Broadcast subtitle credits, e.g. "Untertitelung des ZDF für funk, 2017".
    r"\buntertitelung\b",
    r"\bsubtitling\s+(of|by|for)\b",
    # Public-domain / archive.org boilerplate.
    r"\bpublic\s+domain\b",
    # Arabic: the silence hallucination reported for ar is a translator credit.
    r"\bترجمة\b",
    r"\bنانسي\s+قنقر\b",
    r"\binfo\s+un\s+libro\s+pubblico\b",
]

_HALLUCINATION_RE = re.compile("|".join(_HALLUCINATION_PHRASES))

# Domains that only ever appear in a subtitle track or a watermark.
_HALLUCINATION_HOSTS = re.compile(
    r"\b(?:www\.)?[\w-]+\.(?:org|com|net|info|co\.uk|it|de|fr|es|ru|cz|pl|nl|se|dk|no|fi|tr|gr|pt|br|ar|cn|jp|kr)\b"
)

# A whole segment that is only this, in a window long enough to have held real
# speech, is silence being decorated. Whole-utterance "grazie" / "ok" survives,
# because a real one is short.
_WINDOW_ONLY_PHRASES = {
    "grazie", "grazie mille", "ok", "okay", "si", "no", "ciao", "pronto",
    "thank you", "thanks", "thank you very much", "you", "bye", "hello", "hey",
    "yeah", "yep", "yes", "right", "sure", "please", "grazie a tutti",
    "danke", "merci", "gracias", "obrigado", "obrigada", "da", "net", "ja",
    "sí", "bueno", "vale", "a", "e", "o", "hmm", "mm", "eh", "ehh", "ah",
    "oh", "uh", "um", "mmm",
}
# Below this, an utterance is too short to be judged by its text alone.
_WINDOW_ONLY_MIN_SECONDS = 1.5
# A URL takes a moment to dictate ("apri il sito example punto com"); hearing
# one from a sliver of audio is the decoder inventing it.
_DICTATED_URL_MIN_SECONDS = 0.8

# faster-whisper's own defaults; a segment is only dropped when BOTH signals
# agree that this is not speech, so quiet-but-real speech is never thrown away.
_NO_SPEECH_PROB = 0.6
_AVG_LOGPROB = -1.0

_PUNCT_RE = re.compile(r"[^\w\s]", re.UNICODE)
_SPACE_RE = re.compile(r"\s+")
_URL_RE = re.compile(
    r"(?:https?://|www\.)[^\s]+|\b[\w-]+\.(?:com|org|net|info|co|io|tv|me|uk|it|de|fr|es|ru|cz|pl|nl|se|dk|no|fi|tr|gr|pt|br|ar|cn|jp|kr)\b(?:/\S*)?",
    re.IGNORECASE,
)
# Boilerplate runs glued together by these are markup noise, not speech.
_MARKUP_RE = re.compile(r"[♪♫#*_~|<>\[\]{}]+")
_HASHTAG_RE = re.compile(r"(?:^|\s)[#@][\w.-]+")
# Leftovers after a credit sentence was removed; none of these is an utterance.
_CREDIT_TAILS = {
    "a cura di", "by", "di", "da", "van", "par", "por", "von", "the end",
    "subtitles", "sottotitoli", "revisione", "translated", "tradotto",
}
_DUP_MIN_WORDS = 4   # a 4+ word phrase repeated identically is boilerplate
_DUP_MIN_RUNS = 2


def _normalize(text: str) -> str:
    """Case-fold, drop accents/punctuation/emoji, collapse whitespace.

    Folding accents matters: Whisper writes "comunità" and "für", and a
    matcher that only knew the plain ASCII spelling would miss the credits in
    exactly the languages that trigger them.
    """
    folded = unicodedata.normalize("NFKD", text or "")
    folded = "".join(ch for ch in folded if not unicodedata.combining(ch))
    folded = folded.lower().replace("'", " ").replace("’", " ")
    folded = _PUNCT_RE.sub(" ", folded)
    return _SPACE_RE.sub(" ", folded).strip()


def _words_of(text: str) -> list[str]:
    return [w for w in _normalize(text).split(" ") if w]


def _without_urls(text: str) -> str:
    return _URL_RE.sub(" ", text or "")


def _boilerplate_phrase(text: str) -> bool:
    """True when the text carries a subtitle/credit/CTA fingerprint.

    This is the language-independent half of the filter: URLs, domains, markup
    and hashtags only appear in a transcript because the decoder invented them
    (real dictation is not dictated character by character), and the phrase
    list covers the credit/outro wording reported across languages.
    """
    raw = text or ""
    norm = _normalize(raw)
    if not _normalize(_without_urls(raw)) and (raw.strip() or _URL_RE.search(raw)):
        # Nothing but punctuation/symbols, or nothing but a URL.
        return True
    if _MARKUP_RE.search(raw) or _HASHTAG_RE.search(raw):
        return True
    if _HALLUCINATION_RE.search(norm):
        return True
    # "info un libro pubblico su www.mesmerism.info" keeps the giveaway words
    # once the URL has been normalized away.
    if "pubblico" in norm and "libro" in norm:
        return True
    return False


def _segment_is_hallucination(seg, window_seconds: float) -> bool:
    """True when this decoded segment is invented rather than speech.

    A segment is dropped when nothing survives cleaning it (pure credits, a
    bare URL, a decoder loop), when the model says "not speech" AND is unsure
    (the window where Whisper invents text), or when a multi-second span
    produced nothing but a filler word.
    """
    text = (getattr(seg, "text", "") or "").strip()
    if not text:
        return True
    stripped = clean_transcript(text)
    if not stripped:
        return True
    # Pure credits / a bare URL shorter than it takes to dictate one.
    if _boilerplate_phrase(text) or (
        window_seconds < _DICTATED_URL_MIN_SECONDS and not _normalize(_without_urls(text))
    ):
        return True
    no_speech = float(getattr(seg, "no_speech_prob", 0.0) or 0.0)
    logprob = float(getattr(seg, "avg_logprob", 0.0) or 0.0)
    if no_speech >= _NO_SPEECH_PROB and logprob <= _AVG_LOGPROB:
        return True
    if window_seconds >= _WINDOW_ONLY_MIN_SECONDS and stripped.lower() in _WINDOW_ONLY_PHRASES:
        return True
    return False


def keep_segments(segments: list[tuple[object, float]]):
    """Split one decode into (kept segments, words, cleaned text).

    `segments` is [(segment, span in seconds)] for the whole decode, so the
    confidence rules can see how much audio each segment actually covers.
    A segment can be half real speech and half credit, so the text is cleaned
    per segment too and only what survives is joined.
    """
    kept: list[object] = []
    words: list[tuple[str, float, float]] = []
    texts: list[str] = []
    for seg, span in segments:
        if _segment_is_hallucination(seg, span):
            continue
        text = clean_transcript(getattr(seg, "text", "") or "")
        if not text:
            continue
        kept.append(seg)
        texts.append(text)
        for w in getattr(seg, "words", None) or []:
            words.append((w.word, float(w.start), float(w.end)))
    # Segments carry their own leading/trailing space; dropping one in the
    # middle must not glue its neighbours together.
    joined = _SPACE_RE.sub(" ", " ".join(texts)).strip()
    return kept, words, clean_transcript(joined)


def _is_duplicated_boilerplate(text: str) -> bool:
    """True when a ≥4-word phrase fills the text back-to-back.

    This is the shape the reported bug takes: "Sottotitoli e revisione a cura
    di QTSS" hundreds of times over in one transcript. Two things keep real
    speech safe: the phrase must be at least four words, and the text has to be
    nothing but that repeat (a trailing partial repeat is still a loop — that
    is a window cut mid-repetition).
    """
    words = _words_of(text)
    if len(words) < _DUP_MIN_WORDS * _DUP_MIN_RUNS:
        return False
    for size in range(_DUP_MIN_WORDS, len(words) // _DUP_MIN_RUNS + 1):
        period = words[:size]
        reps = 0
        i = 0
        while words[i:i + size] == period:
            reps += 1
            i += size
        if reps < _DUP_MIN_RUNS:
            continue
        # A trailing partial repetition must still follow the period.
        if all(w == period[j] for j, w in enumerate(words[i:])):
            return True
    return False


def _symbols_only(text: str) -> bool:
    """True for a run with no letters or digits at all ("♪♪♪", "..." )."""
    return not any(ch.isalnum() for ch in text or "")


def _strip_credits(text: str) -> str:
    """Remove credit/boilerplate sentences, keeping whatever real speech is
    left in the same turn."""
    if not text or _symbols_only(text):
        return ""
    kept = []
    for sentence in _re_split_sentences(text):
        if _symbols_only(sentence) or _boilerplate_phrase(sentence):
            continue
        # One sentence can hold both real speech and a credit ("accendi la luce
        # Sottotitoli a cura di QTSS"): cut at the credit, keep what precedes.
        match = _HALLUCINATION_RE.search(_normalize(sentence))
        if match:
            prefix = _drop_from_phrase(sentence, match)
            if prefix and _normalize(prefix) not in _WINDOW_ONLY_PHRASES:
                kept.append(prefix)
            continue
        # A URL is the one token real dictation may contain ("open
        # example.com") — strip the URL, keep the sentence.
        sentence = _URL_RE.sub(" ", sentence)
        sentence = _SPACE_RE.sub(" ", sentence).strip(" \t\n\r-–—:;,.")
        if sentence and _normalize(sentence) not in _WINDOW_ONLY_PHRASES:
            kept.append(sentence)
    out = " ".join(kept).strip()
    # A leftover fragment that is only the tail of a credit ("a cura di", "by")
    # is not a sentence either.
    if _normalize(out) in _CREDIT_TAILS:
        return ""
    if _is_duplicated_boilerplate(out):
        return ""
    return out


def _drop_from_phrase(sentence: str, match) -> str:
    """The original sentence up to where the normalized match began.

    Normalizing drops punctuation and folds accents, so its offsets do not map
    back one-to-one; walking the real characters and counting the characters
    that survive normalization finds the cut point exactly.
    """
    norm_prefix = _normalize(sentence)[:match.start()]
    if not norm_prefix:
        return ""
    used = 0
    for i in range(len(sentence)):
        if used >= len(norm_prefix):
            return sentence[:i].strip()
        if _normalize(sentence[i:i + 1]):
            used += 1
    return sentence.strip()


def _re_split_sentences(text: str) -> list[str]:
    return [s for s in re.split(r"(?<=[.!?…])\s+|\n+", text or "") if s.strip()]


def clean_transcript(text: str) -> str:
    """Public entry point: text -> the part a human actually said.

    Used by every decode path so hallucinations can never reach the UI, the
    committed streaming prefix, or the agent.
    """
    if not text or not text.strip():
        return ""
    if _symbols_only(text) or _symbols_only(_without_urls(text)):
        return ""
    if _is_duplicated_boilerplate(text):
        return ""
    if _boilerplate_phrase(text):
        return _strip_credits(text)
    return text.strip()


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


def _fold_word(word: str) -> str:
    """Word identity for LocalAgreement: case- and punctuation-insensitive."""
    return "".join(ch for ch in word.lower() if ch.isalnum())


def _common_prefix_len(a: list[str], b: list[str]) -> int:
    n = 0
    for x, y in zip(a, b):
        if _fold_word(x) != _fold_word(y):
            break
        n += 1
    return n


def _join(words: list[str]) -> str:
    text = " ".join(w.strip() for w in words if w.strip())
    return text.strip()


def _decode(model, audio: np.ndarray, lang: str | None, *, accurate: bool, prompt: str | None = None):
    """Run one Whisper pass and return (words, text), hallucinations removed.

    `words` is a list of (word, start, end) in seconds relative to `audio`.
    Partials run greedy (beam 1) and without VAD so nothing the user said is
    dropped between passes; the final pass uses beam search + VAD for accuracy.

    Both passes drop segments the model invented (silence → subtitle credits,
    "thanks for watching", and friends) and strip credit clauses out of
    otherwise real speech, so an invented sentence can never be committed to
    the streaming prefix or handed to the agent.
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
    # The confidence gate needs to know how much audio this decode covered
    # before any segments are consumed.
    raw = list(segments)
    total = float(getattr(_info, "duration", 0.0) or 0.0)
    if total <= 0:
        total = len(audio) / SAMPLE_RATE if audio is not None else 0.0
    spans = [
        max(0.0, float(getattr(seg, "end", 0.0) or 0.0) - float(getattr(seg, "start", 0.0) or 0.0))
        for seg in raw
    ]
    unknown = sum(1 for span in spans if span <= 0)
    known = sum(spans)
    for i, span in enumerate(spans):
        if span <= 0 and unknown:
            spans[i] = max(0.0, (total - known) / unknown)
    _kept, words, text = keep_segments(list(zip(raw, spans)))
    return words, text


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

    head = clean_transcript(_join(session.committed))
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
            text = clean_transcript(_join(sess.committed))
        # Fall back to the streamed text when VAD eats a very short utterance —
        # but never fall back to a hallucination the filter just removed.
        if not text.strip():
            text = clean_transcript(sess.partial or _join(sess.committed))
        text = clean_transcript(text)
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
        {
            "text": clean_transcript(sess.partial or _join(sess.committed)),
            "partial": False,
            "final": True,
        }
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
