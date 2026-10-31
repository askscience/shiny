import { apiFetch, getVoiceLang } from '../api.js';
import { getTtsVoice, getTtsSpeed, getEffectiveTtsEngine } from '../preferences.js';
import { ENGINE_DEFAULT_VOICE } from '../voiceShared.js';
import { t } from '../i18n.js';

/** Playback owns its audio URLs, analyser and barge-in microphone. */
export function createVoicePlayback({ onIdle = () => {}, request = apiFetch } = {}) {
  const BARGE_IN_RMS = 0.03;
  const BARGE_IN_HOLD_MS = 350;
  const BARGE_IN_SETTLE_MS = 800;
  /**
   * Window after a reply starts playing in which the assistant's own residual
   * (what echo cancellation left of it in the microphone) is *measured* instead
   * of acted on — see `settleEchoThreshold()`. Outside it, a voice above the
   * threshold interrupts.
   */
  const ECHO_SAMPLE_MS = BARGE_IN_SETTLE_MS;
  /** Frames kept for that measurement (~20 ms each on a 48 kHz context). */
  const ECHO_SAMPLE_MAX = 48;
  /**
   * Which sample counts as the echo floor. The 40th percentile tracks a loud
   * speaker but cannot be dragged up by a user who starts talking immediately,
   * which a mean or a maximum would.
   */
  const ECHO_PERCENTILE = 0.4;
  /** How far above that floor a voice has to be to count as interrupting. */
  const ECHO_MARGIN = 2.5;
  /** Ceiling for the raised threshold, so a loud room stays interruptible. */
  const BARGE_IN_MAX = 0.08;
  let currentAudio = null;
  let stopPlayback = null;
  let speakToken = 0;
  let barge = { stream: null, ctx: null, proc: null, source: null, since: 0, startedAt: 0, fired: true };
  /** Level a voice must exceed to interrupt; re-measured for every reply. */
  let bargeThreshold = BARGE_IN_RMS;
  /** When the current reply started playing (0 when nothing is playing). */
  let playbackStartedAt = 0;
  /** Residual levels sampled during the first ECHO_SAMPLE_MS of the reply. */
  let echoSamples = [];
  /** The last reply that was actually spoken — the echo filter's reference. */
  let lastSpoken = '';
  let pendingSpeech = null;

  /**
   * Tell the app whether a reply is audible, and what it is saying. While it
   * speaks, `audio` is the playing element, so a listener can follow its
   * progress (the spoken-reply caption times its words against it).
   */
  function announceSpeaking(speaking) {
    window.dispatchEvent(new CustomEvent('voice:speaking', {
      detail: { speaking, text: lastSpoken, audio: speaking ? currentAudio : null },
    }));
  }

  async function speak(text, lang) {
    stopSpeaking();
    // A reply can legitimately have nothing left to say aloud: an image or song
    // link is stripped before it gets here, and a picture-only reply strips to
    // an empty string. The synthesizer rejects an empty input, so sending it
    // used to end every such turn in "Voice playback unavailable" — an error
    // toast for a reply that was only ever a picture. Nothing was audible, so
    // the reply is announced as already finished: a conversation window still
    // has to re-open on it.
    if (!text || !text.trim()) {
      announceSpeaking(false);
      return;
    }
    const token = ++speakToken;
    const controller = new AbortController();
    pendingSpeech = controller;
    document.body.classList.add('orb-speaking');
    const engine = getEffectiveTtsEngine();
    try {
      const blob = await request('/api/tts', {
        method: 'POST',
        body: JSON.stringify({ text, lang: lang || getVoiceLang(), engine,
          voice: getTtsVoice(engine) || ENGINE_DEFAULT_VOICE[engine], speed: getTtsSpeed() }),
        responseType: 'blob', signal: controller.signal,
      });
      if (token !== speakToken) return;
      const url = URL.createObjectURL(blob);
      let audio;
      try { audio = new Audio(url); }
      catch (error) { URL.revokeObjectURL(url); throw error; }
      currentAudio = audio;
      const stopPulse = attachTtsPulse(audio);
      await new Promise((resolve, reject) => {
        let settled = false;
        const finish = (error) => {
          if (settled) return;
          settled = true;
          stopPlayback = null;
          if (currentAudio === audio) currentAudio = null;
          audio.onended = null;
          audio.onerror = null;
          stopPulse();
          URL.revokeObjectURL(url);
          playbackStartedAt = 0;
          announceSpeaking(false);
          if (error) reject(error); else resolve();
        };
        stopPlayback = () => finish();
        audio.onended = () => finish();
        audio.onerror = () => finish(new Error('playback failed'));
        // The reply is audible from here on, and the barge-in monitor spends
        // the first ECHO_SAMPLE_MS measuring it instead of watching for a
        // voice, so its threshold can sit above the assistant's own residual.
        lastSpoken = text;
        playbackStartedAt = performance.now();
        bargeThreshold = BARGE_IN_RMS;
        echoSamples = [];
        announceSpeaking(true);
        Promise.resolve().then(() => audio.play()).catch(finish);
      });
    } catch (error) {
      if (token !== speakToken || controller.signal.aborted) return;
      console.warn('TTS failed:', error);
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: t('voice.playbackUnavailable', null, 'Voice playback unavailable'), type: 'error' },
      }));
      // Nothing reached the speakers, but nothing is speaking either: the
      // window may re-open on this reply just like on a spoken one.
      announceSpeaking(false);
    } finally {
      if (pendingSpeech === controller) pendingSpeech = null;
      // A cancelled reply must never clear the newer reply's speaking state.
      if (token === speakToken) {
        document.body.classList.remove('orb-speaking');
        onIdle();
      }
    }
  }

  /**
   * Cut the assistant off mid-sentence.
   *
   * Resolves the in-flight `speak()` promise so the turn that is awaiting
   * playback finishes immediately instead of hanging until the audio ends.
   * Returns true when something was actually playing.
   */
  function stopSpeaking() {
    const wasPlaying = !!stopPlayback;
    const stop = stopPlayback;
    stopPlayback = null;
    speakToken += 1; // a reply still being fetched must not start playing
    pendingSpeech?.abort();
    pendingSpeech = null;
    if (currentAudio) {
      try { currentAudio.pause(); } catch (_) {}
      currentAudio = null;
    }
    if (stop) {
      try { stop(); } catch (_) {}
    }
    document.body.classList.remove('orb-speaking');
    onIdle();
    return wasPlaying;
  }

  /* ── Barge-in (talking over the assistant) ──────────────────── */

  /**
   * Watch the microphone while the assistant is thinking or speaking.
   *
   * The mic has to stay open for the user to be able to interrupt, which is a
   * deliberate trade: `echoCancellation` plus a threshold above the speech
   * threshold keep the assistant's own voice from cutting itself off — and that
   * threshold is **measured per reply** rather than fixed, see
   * [`settleEchoThreshold`].
   */
  let bargeGeneration = 0;
  let bargeStarting = false;

  /**
   * Set this reply's interruption threshold above the assistant's own residual.
   *
   * Measured, not guessed: with headphones the floor is near silence and the
   * threshold stays at `BARGE_IN_RMS`, so even a quiet voice interrupts; a
   * laptop speaker leaks enough of the reply to raise the bar above it, which is
   * what keeps the assistant from stopping itself. Only the opening
   * `ECHO_SAMPLE_MS` of playback is sampled, and the 40th percentile means a
   * user who starts talking inside that window cannot push the bar above their
   * own voice.
   */
  function settleEchoThreshold() {
    const ordered = echoSamples.slice().sort((a, b) => a - b);
    echoSamples = [];
    const floor = ordered[Math.floor(ordered.length * ECHO_PERCENTILE)] || 0;
    bargeThreshold = Math.min(BARGE_IN_MAX, Math.max(BARGE_IN_RMS, floor * ECHO_MARGIN));
  }

  async function startBargeInMonitor() {
    if (barge.stream || bargeStarting) return;
    bargeStarting = true;
    const generation = ++bargeGeneration;
    let stream = null;
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        video: false,
        audio: { echoCancellation: true, noiseSuppression: true, channelCount: { ideal: 1 } },
      });
      if (generation !== bargeGeneration) {
        stream.getTracks().forEach((track) => track.stop());
        return;
      }
      const Ctx = window.AudioContext || window.webkitAudioContext;
      const ctx = new Ctx();
      const source = ctx.createMediaStreamSource(stream);
      const proc = ctx.createScriptProcessor(1024, 1, 1);
      barge = {
        stream, ctx, proc, source,
        since: 0,
        startedAt: performance.now(),
        fired: false,
      };
      proc.onaudioprocess = (e) => {
        if (barge.fired || !barge.stream) return;
        const now = performance.now();
        const data = e.inputBuffer.getChannelData(0);
        let sum = 0;
        for (let i = 0; i < data.length; i++) sum += data[i] * data[i];
        const rms = Math.sqrt(sum / data.length);

        // The opening moments of a reply are the assistant's own voice coming
        // back through echo cancellation: measure it instead of acting on it,
        // so the threshold can sit above it (settleEchoThreshold).
        if (playbackStartedAt && now - playbackStartedAt < ECHO_SAMPLE_MS) {
          echoSamples.push(rms);
          if (echoSamples.length > ECHO_SAMPLE_MAX) echoSamples.shift();
          return;
        }
        if (echoSamples.length) settleEchoThreshold();

        // The start of a turn is the user getting ready, not an interruption.
        if (now - barge.startedAt < BARGE_IN_SETTLE_MS) return;
        if (rms > bargeThreshold) {
          if (!barge.since) barge.since = now;
          if (now - barge.since >= BARGE_IN_HOLD_MS) {
            barge.fired = true;
            window.dispatchEvent(new CustomEvent('voice:barge-in'));
          }
        } else {
          barge.since = 0;
        }
      };
      source.connect(proc);
      // A ScriptProcessor is only pulled while connected to a destination; it
      // writes nothing, so this stays silent.
      proc.connect(ctx.destination);
    } catch (_) {
      if (stream) stream.getTracks().forEach((t) => t.stop());
      if (generation === bargeGeneration) stopBargeInMonitor();
    } finally {
      if (generation === bargeGeneration) bargeStarting = false;
    }
  }

  function stopBargeInMonitor() {
    bargeGeneration += 1;
    bargeStarting = false;
    const { stream, ctx, proc, source } = barge;
    barge = { stream: null, ctx: null, proc: null, source: null, since: 0, startedAt: 0, fired: true };
    bargeThreshold = BARGE_IN_RMS;
    echoSamples = [];
    try { source?.disconnect(); } catch (_) {}
    try { proc?.disconnect(); } catch (_) {}
    try { ctx?.close(); } catch (_) {}
    stream?.getTracks().forEach((t) => t.stop());
  }

  function isBargeInMonitoring() {
    return !!barge.stream;
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
   * Drive `voice:level` from an <audio> element's own output, so the glow keeps
   * reacting while the assistant talks. Returns a stop() that releases the
   * analyser and lets the glow settle. Falls back to a no-op when WebAudio can't
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
      // Let the glow fall back to rest instead of freezing on the last frame.
      window.dispatchEvent(new CustomEvent('voice:level', {
        detail: { level: 0, pan: 0, source: 'tts' },
      }));
    };
  }

  return { speak, stopSpeaking, startBargeInMonitor, stopBargeInMonitor, isBargeInMonitoring };
}
