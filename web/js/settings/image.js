import { apiFetch } from '../api.js';
import { toast, button, select } from '../../ui/index.js';
import { t } from '../i18n.js';
import { getImageModel, setImageModel, getImageQuality, setImageQuality } from '../preferences.js';
import { el, field } from './controls.js';

/** Build the image section with resources owned by its current mount. */
export function createImageSection({ on, cleanups, resources }) {
  const IMAGE_QUALITY_BLURB = {
    fast: 'settings.images.quality.fast.blurb',
    balanced: 'settings.images.quality.balanced.blurb',
    high: 'settings.images.quality.high.blurb',
    max: 'settings.images.quality.max.blurb',
  };

  const IMAGE_MODELS = [
    {
      value: 'z-image',
      labelKey: 'settings.images.model.zImage',
      hintKey: 'settings.images.model.zImage.hint',
    },
    {
      value: 'qwen-image-2.1',
      labelKey: 'settings.images.model.qwenImage',
      hintKey: 'settings.images.model.qwenImage.hint',
    },
  ];

  const IMAGE_QUALITY_OPTIONS = [
    { value: 'fast', labelKey: 'settings.images.quality.fast' },
    { value: 'balanced', labelKey: 'settings.images.quality.balanced' },
    { value: 'high', labelKey: 'settings.images.quality.high' },
    { value: 'max', labelKey: 'settings.images.quality.max' },
  ];

  function buildImage() {
    const model = select({
      options: IMAGE_MODELS.map((m) => ({ value: m.value, label: t(m.labelKey) })),
      onChange: (value) => {
        setImageModel(value);
        updateImageUI();
      },
    });
    model.select.value = getImageModel();

    const quality = select({
      options: IMAGE_QUALITY_OPTIONS.map((o) => ({ value: o.value, label: t(o.labelKey) })),
      onChange: (value) => {
        setImageQuality(value);
        updateImageUI();
      },
    });

    const modelHint = el('p', 'settings-hint');
    const hint = el('p', 'settings-hint');
    const download = button({ label: t('common.download', null, 'Download'), variant: 'quiet', size: 'sm' });
    download.classList.add('hidden');
    const row = el('div', 'whisper-model-row');
    row.append(hint, download);

    let status = null;
    let statusLoaded = false;
    let poll = null;

    const modelDef = (id) => IMAGE_MODELS.find((m) => m.value === id) || IMAGE_MODELS[0];
    const familyInfo = (id) => status?.inventory?.families?.[id] || null;
    const infoFor = (id, key) => familyInfo(id)?.qualities?.[key] || null;
    const downloadFor = (id, key) => status?.downloads?.[id]?.[key] || null;
    const baseFor = (id) => status?.downloads?.[id]?.base || null;
    const describe = (id, key) => {
      const info = infoFor(id, key);
      const blurb = IMAGE_QUALITY_BLURB[key];
      return info ? `${info.quant} · ${info.size} · ${blurb ? t(blurb) : ''}` : '';
    };

    function updateImageUI() {
      const currentModel = getImageModel();
      const current = getImageQuality();
      model.select.value = currentModel;
      quality.select.value = current;
      modelHint.textContent = t(modelDef(currentModel).hintKey);
      const info = infoFor(currentModel, current);
      const base = baseFor(currentModel);

      if (!status) {
        hint.textContent = statusLoaded
          ? t('settings.images.checkFailed', null, 'Could not check the image models.')
          : t('settings.images.checking', null, 'Checking the image models…');
        download.classList.add('hidden');
        return;
      }
      if (!status.inventory?.sd_cli) {
        hint.textContent = t('settings.images.unavailable', null, 'Local image generation is unavailable: the sd-cli binary was not found (set Z_IMAGE_SD_CLI).');
        download.classList.add('hidden');
        return;
      }
      if (base?.status === 'downloading') {
        const pct = base.total ? Math.round((base.bytes / base.total) * 100) : 0;
        hint.textContent = t('common.downloadingShared', { percent: pct }, `Downloading the shared model files… ${pct}%`);
        download.classList.add('hidden');
        return;
      }
      const dl = downloadFor(currentModel, current);
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
        hint.textContent = t('settings.images.ready', { details: describe(currentModel, current) }, `Ready — ${describe(currentModel, current)}. Images run locally on this machine; larger sizes take minutes.`);
        download.classList.add('hidden');
        return;
      }
      const missing = [];
      for (const shared of familyInfo(currentModel)?.shared || []) {
        if (!shared.present) missing.push(shared.label);
      }
      if (!info?.present) missing.push(describe(currentModel, current) || current);
      hint.textContent = t('common.notDownloaded', { missing: missing.join(', ') }, `Not downloaded yet — ${missing.join(', ')}. The assistant asks before fetching.`);
      download.classList.remove('hidden');
      download.disabled = false;
    }

    async function refreshImageStatus() {
      try {
        const res = await apiFetch('/api/imagegen/status');
        status = res?.data || null;
      } catch (_) {
        status = null;
      }
      statusLoaded = true;
      updateImageUI();
      return status;
    }

    function pollDownload() {
      clearInterval(poll);
      poll = resources.interval(async () => {
        await refreshImageStatus();
        const currentModel = getImageModel();
        const current = getImageQuality();
        const state = downloadFor(currentModel, current)?.status;
        if (infoFor(currentModel, current)?.ready || state === 'error') {
          clearInterval(poll);
          poll = null;
          if (infoFor(currentModel, current)?.ready) toast(t('settings.images.modelReady', null, 'Image model ready'), { type: 'info' });
        }
      }, 1500);
    }

    on(download, 'click', async () => {
      const currentModel = getImageModel();
      const current = getImageQuality();
      download.disabled = true;
      try {
        await apiFetch('/api/imagegen/download', {
          method: 'POST',
          body: JSON.stringify({ model: currentModel, quality: current }),
        });
        toast(t('settings.images.downloadingBackground', null, 'Downloading image models in the background…'), { type: 'info' });
        pollDownload();
      } catch (e) {
        toast(e.message || t('common.downloadStartFailed', null, 'Could not start the download'), { type: 'error' });
        download.disabled = false;
      }
    });

    refreshImageStatus();
    cleanups.push(() => { clearInterval(poll); poll = null; });

    const modelField = field(t('settings.images.model', null, 'Model'), model);
    modelField.appendChild(modelHint);
    return [
      modelField,
      field(t('common.qualityModel', null, 'Quality model'), quality),
      row,
    ];
  }

  /* ── Music (local song generation) ──────────────────────────── */

  return buildImage();
}
