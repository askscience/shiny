import { apiFetch } from '../api.js';
import { toast, button, select } from '../../ui/index.js';
import { t } from '../i18n.js';
import { getSongQuality, setSongQuality } from '../preferences.js';
import { el, field } from './controls.js';

/** Build the music section with resources owned by its current mount. */
export function createMusicSection({ on, cleanups, resources }) {
  const SONG_QUALITY_BLURB = {
    q4: 'settings.music.quality.q4.blurb',
    q8: 'settings.music.quality.q8.blurb',
  };

  const SONG_QUALITY_OPTIONS = [
    { value: 'q4', labelKey: 'settings.music.quality.q4' },
    { value: 'q8', labelKey: 'settings.music.quality.q8' },
  ];


  function buildMusic() {
    const quality = select({
      options: SONG_QUALITY_OPTIONS.map((o) => ({ value: o.value, label: t(o.labelKey) })),
      onChange: (value) => {
        setSongQuality(value);
        updateMusicUI();
      },
    });

    const hint = el('p', 'settings-hint');
    const download = button({ label: t('common.download', null, 'Download'), variant: 'quiet', size: 'sm' });
    download.classList.add('hidden');
    const row = el('div', 'whisper-model-row');
    row.append(hint, download);

    let status = null;
    let statusLoaded = false;
    let poll = null;

    const infoFor = (key) => status?.inventory?.qualities?.[key] || null;
    const downloadFor = (key) => status?.downloads?.[key] || null;
    const baseFor = () => status?.downloads?.base || null;
    const describe = (key) => {
      const info = infoFor(key);
      const blurb = SONG_QUALITY_BLURB[key];
      return info ? `${info.quant} · ${info.size} · ${blurb ? t(blurb) : ''}` : '';
    };

    function updateMusicUI() {
      const current = getSongQuality();
      quality.select.value = current;
      const info = infoFor(current);
      const base = baseFor();

      if (!status) {
        hint.textContent = statusLoaded
          ? t('settings.music.checkFailed', null, 'Could not check the song models.')
          : t('settings.music.checking', null, 'Checking the song models…');
        download.classList.add('hidden');
        return;
      }
      if (!status.inventory?.engine) {
        hint.textContent = t('settings.music.unavailable', null, 'Local song generation is unavailable: the ace-server engine was not found (set YUE2_ENGINE or run scripts/install-yue2-engine.sh).');
        download.classList.add('hidden');
        return;
      }
      if (base?.status === 'downloading') {
        const pct = base.total ? Math.round((base.bytes / base.total) * 100) : 0;
        hint.textContent = t('common.downloadingShared', { percent: pct }, `Downloading the shared model files… ${pct}%`);
        download.classList.add('hidden');
        return;
      }
      const dl = downloadFor(current);
      if (dl?.status === 'downloading') {
        const pct = dl.total ? Math.round((dl.bytes / dl.total) * 100) : 0;
        hint.textContent = t('common.downloadingModel', { model: current, percent: pct }, `Downloading the ${current} model… ${pct}%`);
        download.classList.add('hidden');
        return;
      }
      if (dl?.status === 'error') {
        hint.textContent = dl.error || t('common.downloadFailed', null, 'Download failed.');
        download.classList.remove('hidden');
        download.disabled = false;
        return;
      }
      if (info?.ready) {
        hint.textContent = t('settings.music.ready', { details: describe(current) }, `Ready — ${describe(current)}. Songs render locally and take several minutes.`);
        download.classList.add('hidden');
        return;
      }
      const missing = [];
      for (const shared of status.inventory?.shared || []) {
        if (!shared.present) missing.push(shared.label);
      }
      if (!info?.present) missing.push(describe(current) || current);
      hint.textContent = t('common.notDownloaded', { missing: missing.join(', ') }, `Not downloaded yet — ${missing.join(', ')}. The assistant asks before fetching.`);
      download.classList.remove('hidden');
      download.disabled = false;
    }

    async function refreshMusicStatus() {
      try {
        const res = await apiFetch('/api/songgen/status');
        status = res?.data || null;
      } catch (_) {
        status = null;
      }
      statusLoaded = true;
      updateMusicUI();
      return status;
    }

    function pollDownload() {
      clearInterval(poll);
      poll = resources.interval(async () => {
        await refreshMusicStatus();
        const current = getSongQuality();
        const state = downloadFor(current)?.status;
        if (infoFor(current)?.ready || state === 'error') {
          clearInterval(poll);
          poll = null;
          if (infoFor(current)?.ready) toast(t('settings.music.modelReady', null, 'Song model ready'), { type: 'info' });
        }
      }, 1500);
    }

    on(download, 'click', async () => {
      const current = getSongQuality();
      download.disabled = true;
      try {
        await apiFetch('/api/songgen/download', {
          method: 'POST',
          body: JSON.stringify({ quality: current }),
        });
        toast(t('settings.music.downloadingBackground', null, 'Downloading the song models in the background…'), { type: 'info' });
        pollDownload();
      } catch (e) {
        toast(e.message || t('common.downloadStartFailed', null, 'Could not start the download'), { type: 'error' });
        download.disabled = false;
      }
    });

    refreshMusicStatus();
    cleanups.push(() => { clearInterval(poll); poll = null; });

    const qualityField = field(t('common.qualityModel', null, 'Quality model'), quality);
    qualityField.appendChild(el('p', 'settings-hint', t('settings.music.hint', null, 'YuE2 turns style + lyrics into a 48 kHz stereo song on this machine.')));
    return [qualityField, row];
  }


  return buildMusic();
}
