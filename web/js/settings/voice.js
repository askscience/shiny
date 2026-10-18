import { apiFetch, getVoiceLang, getVoiceLangExplicit, setVoiceLang, clearVoiceLang } from '../api.js';
import { toast, button, select, slider, toggleRow, revealRegion, disposeMotion } from '../../ui/index.js';
import { t } from '../i18n.js';
import { getTtsVoice, setTtsVoice, getTtsSpeed, setTtsSpeed, getSilenceTimeout, setSilenceTimeout, getWakeWord, setWakeWord, getConversation, setConversation, getConversationTimeout, setConversationTimeout, getSttEngine, setSttEngine, getWhisperModel, setWhisperModel, getTtsEngine, setTtsEngine, getQwenModel, setQwenModel } from '../preferences.js';
import { voicesForEngine } from '../voiceShared.js';
import { el, field, sliderField, heading } from './controls.js';

/** Build the voice section with resources owned by its current mount. */
export function createVoiceSection({ on, cleanups, resources }) {
  const WHISPER_MODEL_LABEL = {
    tiny: 'settings.voice.whisper.model.tiny',
    small: 'settings.voice.whisper.model.small',
  };
  const QWEN_MODEL_LABEL = {
    '0.6b-customvoice': 'settings.voice.qwen.model.06b',
    '1.7b-customvoice': 'settings.voice.qwen.model.17b',
  };
  function buildVoice() {
    // Empty value = follow the interface language (the default). Picking a code
    // stores an explicit voice language that then stays put across interface
    // language changes.
    const lang = select({
      onChange: (value) => (value ? setVoiceLang(value) : clearVoiceLang()),
    });
    const langHint = el('p', 'settings-hint', t('settings.voice.language.hint'));

    const engine = select({
      options: [
        { value: 'whisper', label: t('settings.voice.sttEngine.whisper', null, 'Faster Whisper (recommended)') },
        { value: 'vosk', label: t('settings.voice.sttEngine.vosk', null, 'Vosk (in-browser)') },
      ],
      onChange: (value) => {
        setSttEngine(value);
        updateWhisperUI();
        if (value === 'whisper' && whisperStatus?.whisper !== 'ready') {
          toast(t('settings.voice.whisperNotRunning', null, 'Faster Whisper is not running — voice will use Vosk'), { type: 'info' });
        }
      },
    });
    const engineHint = el('p', 'settings-hint', t('settings.voice.sttEngineHint', null, 'Faster Whisper runs on the machine hosting PEAK\u2019D! and streams words while you speak — noticeably more accurate than Vosk. Vosk runs entirely in this browser.'));

    const whisperModel = select({
      options: [
        { value: 'tiny', label: t('settings.voice.whisper.option.tiny', null, 'Tiny — included (~75 MB)') },
        { value: 'small', label: t('settings.voice.whisper.option.small', null, 'Small — download (~480 MB)') },
      ],
      onChange: (value) => {
        setWhisperModel(value);
        updateWhisperUI();
      },
    });
    const whisperHint = el('p', 'settings-hint');
    const whisperDownload = button({ label: t('common.download', null, 'Download'), variant: 'quiet', size: 'sm' });
    whisperDownload.classList.add('hidden');
    const whisperRow = el('div', 'whisper-model-row');
    whisperRow.append(whisperHint, whisperDownload);
    const whisperFields = el('div');
    cleanups.push(() => disposeMotion(whisperFields));
    whisperFields.append(field(t('settings.voice.whisperModel', null, 'Whisper model'), whisperModel), whisperRow);

    let whisperStatus = null;
    let whisperStatusLoaded = false;
    let whisperPoll = null;

    function updateWhisperUI() {
      const current = getWhisperModel();
      whisperModel.select.value = current;
      revealRegion(whisperFields, getSttEngine() === 'whisper');

      const info = whisperStatus?.whisper_models?.[current];
      const download = whisperStatus?.whisper_downloads?.[current];
      const label = t(WHISPER_MODEL_LABEL[current], null, current);

      if (whisperStatus && whisperStatus.whisper !== 'ready') {
        whisperHint.textContent = t('settings.voice.whisperMissing', null, 'Faster Whisper is not running on this machine — voice falls back to Vosk until it is started (./voice/start_whisper.sh).');
        whisperDownload.classList.add('hidden');
        return;
      }
      if (!whisperStatus) {
        whisperHint.textContent = whisperStatusLoaded
          ? t('settings.voice.speechCheckFailed', null, 'Could not check the speech service.')
          : t('settings.voice.speechChecking', null, 'Checking the speech service…');
        whisperDownload.classList.add('hidden');
        return;
      }
      if (info?.present) {
        const note = current === 'tiny' ? t('settings.voice.defaultNote', null, ' (default)') : '';
        whisperHint.textContent = info.loaded
          ? t('settings.voice.whisper.readyLoaded', { model: label, note }, `${label} model ready and loaded${note}.`)
          : t('settings.voice.whisper.ready', { model: label, note }, `${label} model ready${note}.`);
        whisperDownload.classList.add('hidden');
        return;
      }
      if (download?.status === 'downloading') {
        const pct = download.total ? Math.round((download.bytes / download.total) * 100) : 0;
        whisperHint.textContent = t('common.downloadingModel', { model: label.toLowerCase(), percent: pct }, `Downloading the ${label.toLowerCase()} model… ${pct}%`);
        whisperDownload.classList.add('hidden');
        return;
      }
      if (download?.status === 'error') {
        whisperHint.textContent = download.error || t('common.downloadFailed', null, 'Download failed.');
        whisperDownload.classList.remove('hidden');
        whisperDownload.disabled = false;
        return;
      }
      whisperHint.textContent = t('settings.voice.whisper.notDownloaded', { model: label }, `${label} model is not downloaded yet — voice uses Tiny until you download it.`);
      whisperDownload.classList.remove('hidden');
      whisperDownload.disabled = false;
    }

    function pollWhisperDownload() {
      clearInterval(whisperPoll);
      whisperPoll = resources.interval(async () => {
        const status = await refreshVoiceStatus();
        const current = getWhisperModel();
        const state = status?.whisper_downloads?.[current]?.status;
        const present = status?.whisper_models?.[current]?.present === true;
        if (present || state === 'error') {
          clearInterval(whisperPoll);
          whisperPoll = null;
          if (present) {
            const label = t(WHISPER_MODEL_LABEL[current], null, current);
            toast(t('settings.voice.whisper.readyToast', { model: label }, `${label} model ready`), { type: 'info' });
          }
        }
      }, 1500);
      cleanups.push(() => { clearInterval(whisperPoll); whisperPoll = null; });
    }

    on(whisperDownload, 'click', async () => {
      const current = getWhisperModel();
      whisperDownload.disabled = true;
      try {
        await apiFetch('/api/voice/whisper/download', {
          method: 'POST',
          body: JSON.stringify({ model: current }),
        });
        toast(t('settings.voice.whisper.downloadingBackground', { model: current }, `Downloading the ${current} model in the background…`), { type: 'info' });
        pollWhisperDownload();
      } catch (e) {
        toast(e.message || t('common.downloadStartFailed', null, 'Could not start the download'), { type: 'error' });
        whisperDownload.disabled = false;
      }
    });

    const ttsVoice = select({
      options: [
        { value: '', label: t('common.defaultServer', null, 'Default (server)') },
        ...voicesForEngine(getTtsEngine()).map((v) => ({ value: v, label: v })),
      ],
      onChange: (value) => {
        setTtsVoice(value, getTtsEngine());
        ttsVoiceValue = value;
      },
    });

    // ── Speech output engine ────────────────────────────────────
    const ttsEngine = select({
      options: [
        { value: 'supertonic', label: t('settings.voice.ttsEngine.supertonic', null, 'Supertonic (default)') },
        { value: 'qwen', label: t('settings.voice.ttsEngine.qwen', null, 'Qwen3-TTS (experimental)') },
      ],
      onChange: (value) => {
        setTtsEngine(value);
        updateTtsUI();
        if (value === 'qwen' && qwenStatus && qwenStatus.qwen !== 'ready') {
          toast(t('settings.voice.qwenNotRunning', null, 'Qwen3-TTS is not running — Supertonic will keep speaking'), { type: 'info' });
        }
      },
    });

    const qwenModel = select({
      options: [
        { value: '0.6b-customvoice', label: t('settings.voice.qwen.option.06b', null, '0.6B CustomVoice — recommended (~605 MB)') },
        { value: '1.7b-customvoice', label: t('settings.voice.qwen.option.17b', null, '1.7B CustomVoice — better (~1.2 GB)') },
      ],
      onChange: (value) => {
        setQwenModel(value);
        updateTtsUI();
      },
    });
    const qwenHint = el('p', 'settings-hint');
    const qwenDownload = button({ label: t('common.download', null, 'Download'), variant: 'quiet', size: 'sm' });
    qwenDownload.classList.add('hidden');
    // Reload the sidecar after any pause (and for troubleshooting): kills the
    // process holding the port and re-runs the idempotent launcher.
    const qwenReload = button({ label: t('settings.voice.reload', null, 'Reload'), variant: 'quiet', size: 'sm' });
    on(qwenReload, 'click', async () => {
      qwenReload.disabled = true;
      try {
        await apiFetch('/api/voice/qwen/restart', { method: 'POST' });
        toast(t('settings.voice.qwenRestarting', null, 'Restarting Qwen3-TTS…'), { type: 'info' });
      } catch (e) {
        toast(e.message || t('settings.voice.qwenRestartFailed', null, 'Could not restart Qwen3-TTS'), { type: 'error' });
      } finally {
        setTimeout(() => { qwenReload.disabled = false; }, 1500);
      }
    });
    const qwenRow = el('div', 'whisper-model-row');
    qwenRow.append(qwenHint, qwenDownload, qwenReload);
    const qwenFields = el('div');
    cleanups.push(() => disposeMotion(qwenFields));
    qwenFields.append(field(t('settings.voice.qwenModel', null, 'Qwen model'), qwenModel), qwenRow);

    let qwenStatus = null;
    let qwenPoll = null;

    /**
     * The voice picker is engine-specific: Supertonic's M* / F* voices and the
     * Qwen3-TTS CustomVoice speakers share no names, so the options and the
     * chosen value are re-resolved whenever the engine changes. The value comes
     * from getTtsVoice(engine), which only returns a voice the engine knows —
     * each engine's pick is stored under its own preference key, so switching
     * engines back and forth keeps both choices.
     */
    function updateTtsUI() {
      const engine = getTtsEngine();
      ttsEngine.select.value = engine;
      revealRegion(qwenFields, engine === 'qwen');

      ttsVoice.setOptions([
        { value: '', label: t('common.defaultServer', null, 'Default (server)') },
        ...voicesForEngine(engine).map((v) => ({ value: v, label: v })),
      ]);
      const current = getTtsVoice(engine);
      ttsVoice.select.value = current;
      ttsVoiceValue = current;
    }
    let ttsVoiceValue = getTtsVoice(getTtsEngine());

    function updateQwenUI() {
      const current = getQwenModel();
      qwenModel.select.value = current;
      const info = qwenStatus?.qwen_models?.[current];
      const download = qwenStatus?.qwen_downloads?.[current];
      const label = t(QWEN_MODEL_LABEL[current], null, current);

      if (!qwenStatus) {
        qwenHint.textContent = t('settings.voice.speechChecking', null, 'Checking the speech service…');
        qwenDownload.classList.add('hidden');
        return;
      }
      if (info?.present) {
        const note = current === '0.6b-customvoice' ? t('settings.voice.defaultNote', null, ' (default)') : '';
        qwenHint.textContent = qwenStatus.qwen === 'ready'
          ? t('settings.voice.qwen.ready', { model: label, note }, `${label} model ready${note}.`)
          : t('settings.voice.qwen.readyStopped', { model: label, note }, `${label} model ready${note} — start the engine to use it.`);
        qwenDownload.classList.add('hidden');
        return;
      }
      if (download?.status === 'downloading') {
        const pct = download.total ? Math.round((download.bytes / download.total) * 100) : 0;
        qwenHint.textContent = t('common.downloadingModel', { model: label, percent: pct }, `Downloading the ${label} model… ${pct}%`);
        qwenDownload.classList.add('hidden');
        return;
      }
      if (download?.status === 'error') {
        qwenHint.textContent = download.error || t('common.downloadFailed', null, 'Download failed.');
        qwenDownload.classList.remove('hidden');
        qwenDownload.disabled = false;
        return;
      }
      qwenHint.textContent = t('settings.voice.qwen.notDownloaded', { model: label }, `${label} model is not downloaded yet — download it to use Qwen3-TTS.`);
      qwenDownload.classList.remove('hidden');
      qwenDownload.disabled = false;
    }

    async function refreshVoiceStatus() {
      try {
        const status = await apiFetch(`/api/voice/status?lang=${encodeURIComponent(getVoiceLang())}`);
        whisperStatus = status;
        qwenStatus = status;
      } catch (_) {
        whisperStatus = null;
        qwenStatus = null;
      }
      whisperStatusLoaded = true;
      updateWhisperUI();
      updateTtsUI();
      updateQwenUI();
      return qwenStatus;
    }

    function pollQwenDownload() {
      clearInterval(qwenPoll);
      qwenPoll = resources.interval(async () => {
        const status = await refreshVoiceStatus();
        const current = getQwenModel();
        const state = status?.qwen_downloads?.[current]?.status;
        const present = status?.qwen_models?.[current]?.present === true;
        if (present || state === 'error') {
          clearInterval(qwenPoll);
          qwenPoll = null;
          if (present) {
            const label = t(QWEN_MODEL_LABEL[current], null, current);
            toast(t('settings.voice.qwen.readyToast', { model: label }, `${label} model ready`), { type: 'info' });
          }
        }
      }, 1500);
      cleanups.push(() => { clearInterval(qwenPoll); qwenPoll = null; });
    }

    on(qwenDownload, 'click', async () => {
      const current = getQwenModel();
      qwenDownload.disabled = true;
      try {
        await apiFetch('/api/voice/qwen/download', {
          method: 'POST',
          body: JSON.stringify({ model: current }),
        });
        const label = t(QWEN_MODEL_LABEL[current], null, current);
        toast(t('settings.voice.qwen.downloadingBackground', { model: label }, `Downloading ${label} in the background…`), { type: 'info' });
        pollQwenDownload();
      } catch (e) {
        toast(e.message || t('common.downloadStartFailed', null, 'Could not start the download'), { type: 'error' });
        qwenDownload.disabled = false;
      }
    });

    const speedControl = slider({
      min: 0.7, max: 2, step: 0.1, value: getTtsSpeed(),
      onInput: (value) => {
        speed.value.textContent = `${value.toFixed(1)}×`;
        setTtsSpeed(value);
      },
    });
    const speed = sliderField(t('settings.voice.speechSpeed', null, 'Speech speed'), `${getTtsSpeed().toFixed(1)}×`, speedControl);

    const silence = select({
      options: [
        { value: '4000', label: t('settings.voice.seconds', { count: 4 }, '4 seconds') },
        { value: '6000', label: t('settings.voice.seconds', { count: 6 }, '6 seconds') },
        { value: '8000', label: t('settings.voice.seconds', { count: 8 }, '8 seconds') },
        { value: '12000', label: t('settings.voice.seconds', { count: 12 }, '12 seconds') },
        { value: '20000', label: t('settings.voice.seconds', { count: 20 }, '20 seconds') },
      ],
      onChange: (value) => setSilenceTimeout(Number(value)),
    });
    const silenceMs = getSilenceTimeout();
    if ([...silence.select.options].some((o) => Number(o.value) === silenceMs)) {
      silence.select.value = String(silenceMs);
    }

    const wake = toggleRow({
      label: t('settings.voice.wakeWord', null, 'Wake word'),
      hint: t('settings.voice.wakeWordHint', null, 'Long-press listens for “Hey…” before a command; when off it just listens.'),
      checked: getWakeWord(),
      onChange: (checked) => setWakeWord(checked),
    });

    // The conversation is on by default: waking the assistant means talking to
    // it. Its length is a choice between a bound and "until I stop it".
    const conversationTimeout = select({
      options: [
        { value: '0', label: t('settings.voice.conversationUntilStopped', null, 'Until I stop it') },
        ...[8, 15, 20, 30, 45, 60].map((count) => ({
          value: String(count * 1000),
          label: t('settings.voice.seconds', { count }, `${count} seconds`),
        })),
      ],
      onChange: (value) => setConversationTimeout(Number(value)),
    });
    const conversationTimeoutMs = getConversationTimeout();
    if ([...conversationTimeout.select.options].some((o) => Number(o.value) === conversationTimeoutMs)) {
      conversationTimeout.select.value = String(conversationTimeoutMs);
    }
    const conversation = toggleRow({
      label: t('settings.voice.conversation', null, 'Keep listening after a reply'),
      hint: t('settings.voice.conversationHint', null,
        'Once “Hey …” wakes it, the assistant stays in conversation: after each reply it keeps listening, '
        + 'so a follow-up needs no phrase. A tap asks one thing only, a double tap switches to typing, '
        + 'and “basta” ends it. The assistant never listens while it is speaking.'),
      checked: getConversation(),
      onChange: (checked) => setConversation(checked),
    });

    if (ttsVoiceValue) ttsVoice.select.value = ttsVoiceValue;

    void (async () => {
      try {
        const res = await apiFetch('/api/voice/languages');
        lang.setOptions([
          { value: '', label: t('settings.voice.language.auto') },
          ...res.data.map((l) => ({
            value: l.code,
            label: `${l.code.toUpperCase()}${l.vosk_available ? '' : t('settings.voice.ttsOnly', null, ' (TTS only)')}`,
          })),
        ]);
        // The stored value is the *override*; the empty option means "follow the
        // interface language", which getVoiceLang() resolves at use time.
        const explicit = getVoiceLangExplicit();
        const codes = res.data.map((l) => l.code);
        if (explicit && codes.includes(explicit)) lang.select.value = explicit;
        else lang.select.value = codes.includes(getVoiceLang()) ? '' : 'en';
      } catch (_) {
        lang.setOptions([{ value: '', label: t('settings.voice.language.auto') }, { value: 'en', label: 'EN' }]);
        lang.select.value = '';
      }
    })();

    updateWhisperUI();
    void refreshVoiceStatus();

    return [
      field(t('settings.voice.language'), lang),
      langHint,
      heading(t('settings.voice.sttHeading', null, 'Speech recognition')),
      field(t('settings.voice.sttEngine', null, 'Engine'), engine, { hint: engineHint.textContent, details: whisperFields }),
      heading(t('settings.voice.ttsHeading', null, 'Speech output')),
      field(t('settings.voice.ttsEngine', null, 'Engine'), ttsEngine, {
        details: qwenFields,
        hint: t('settings.voice.ttsEngineHint', null, 'Supertonic is bundled and always available. Qwen3-TTS sounds more natural, '
          + 'runs on your GPU through Vulkan when one is available, and needs a one-time '
          + 'build plus a model download.'),
      }),
      field(t('settings.voice.voice', null, 'Voice'), ttsVoice, {
        hint: t('settings.voice.voiceHint', null, 'The voice used for spoken replies.'),
      }),
      speed.wrap,
      field(t('settings.voice.stopListening', null, 'Stop listening after'), silence, {
        hint: t('settings.voice.stopListeningHint', null, 'How long to wait for speech before canceling a tap-to-talk input.'),
      }),
      wake,
      conversation,
      field(t('settings.voice.conversationWindow', null, 'Conversation window'), conversationTimeout, {
        hint: t('settings.voice.conversationWindowHint', null, 'How long the microphone stays open after each answer.'),
      }),
    ];
  }

  /* ── Power ──────────────────────────────────────────────────── */

  /* ── Images (local image generation) ────────────────────────── */


  return buildVoice();
}
