import { apiFetch } from '../api.js';
import { input, select, revealRegion, disposeMotion } from '../../ui/index.js';
import { t, tp } from '../i18n.js';
import { getAiName, setAiName, getAiProvider, setAiProvider, getOllamaModel, setOllamaModel, getOpenAiBaseUrl, setOpenAiBaseUrl, getOpenAiApiKey, setOpenAiApiKey, getOpenAiModel, setOpenAiModel, flushPreferencesNow } from '../preferences.js';
import { el, field } from './controls.js';

/** Build the assistant section with resources owned by its current mount. */
export function createAssistantSection({ on, cleanups, resources }) {
  function buildAssistant() {
    const nameInput = input({
      value: getAiName(),
      maxlength: 32,
      autocomplete: 'off',
    });
    const nameHint = el('p', 'settings-hint');
    const syncNameHint = () => {
      nameHint.textContent = t('settings.assistant.nameHint', { name: getAiName() }, `Say “Hey, ${getAiName()}” to activate voice on long press.`);
    };
    syncNameHint();
    on(nameInput, 'input', () => {
      setAiName(nameInput.value);
      syncNameHint();
    });

    const provider = select({
      options: [
        { value: 'ollama', label: t('settings.assistant.providerOllama', null, 'Ollama (local)') },
        { value: 'openai', label: t('settings.assistant.providerOpenai', null, 'OpenAI-compatible endpoint') },
      ],
      onChange: async (value) => {
        setAiProvider(value);
        syncProviderUI();
        await flushPreferencesNow();
        void loadAiModels();
      },
    });

    const model = select({ onChange: (value) => { setOllamaModel(value); void flushPreferencesNow(); } });
    const modelHint = el('p', 'settings-hint', t('settings.assistant.modelsHint', null, 'Models from your local Ollama server.'));

    const baseUrl = input({ value: getOpenAiBaseUrl(), placeholder: 'https://api.openai.com/v1', autocomplete: 'off' });
    const apiKey = input({ type: 'password', value: getOpenAiApiKey(), placeholder: 'sk-…', autocomplete: 'off' });
    const openaiModel = input({ value: getOpenAiModel(), placeholder: 'gpt-4o-mini', autocomplete: 'off' });
    const openaiHint = el('p', 'settings-hint', t('settings.assistant.openaiModelHint', null, 'The exact model name your endpoint expects.'));
    on(baseUrl, 'input', () => setOpenAiBaseUrl(baseUrl.value));
    on(apiKey, 'input', () => setOpenAiApiKey(apiKey.value));
    on(openaiModel, 'input', () => setOpenAiModel(openaiModel.value));

    const ollamaFields = el('div');
    ollamaFields.append(field(t('settings.assistant.aiModel', null, 'AI model'), model, { id: model.select.id }), modelHint);

    const openaiFields = el('div', 'hidden');
    cleanups.push(() => { disposeMotion(ollamaFields); disposeMotion(openaiFields); });
    openaiFields.append(
      field(t('settings.assistant.baseUrl', null, 'Endpoint base URL'), baseUrl),
      field(t('settings.assistant.apiKey', null, 'API key'), apiKey),
      field(t('settings.assistant.model', null, 'Model'), openaiModel),
      openaiHint,
    );

    function syncProviderUI() {
      const isOpenAi = getAiProvider() === 'openai';
      provider.select.value = getAiProvider();
      revealRegion(ollamaFields, !isOpenAi);
      revealRegion(openaiFields, isOpenAi);
    }
    syncProviderUI();

    let serverDefaultModel = '';
    let modelRequest = 0;

    async function loadAiModels() {
      const seq = ++modelRequest;
      try {
        const res = await apiFetch('/api/ai/models');
        if (seq !== modelRequest) return;
        const { provider: who, models = [], default: fallback, available } = res.data || {};
        serverDefaultModel = fallback || '';

        model.setOptions([
          { value: '', label: serverDefaultModel ? t('settings.assistant.defaultModel', { model: serverDefaultModel }, `Default (${serverDefaultModel})`) : t('common.defaultServer', null, 'Default (server)') },
          ...models.map((n) => ({ value: n, label: n })),
        ]);
        const stored = getOllamaModel();
        model.select.value = stored && models.includes(stored) ? stored : '';

        if (who === 'openai') {
          modelHint.textContent = '';
          openaiHint.textContent = available
            ? (models.length
              ? tp('settings.assistant.modelsFound', models.length, null, `${models.length} model(s) found at the endpoint.`)
              : t('settings.assistant.endpointReachable', null, 'Endpoint reachable.'))
            : t('settings.assistant.endpointUnreachable', null, 'Could not reach the endpoint yet — check the base URL and API key.');
        } else {
          openaiHint.textContent = t('settings.assistant.openaiModelHint', null, 'The exact model name your endpoint expects.');
          modelHint.textContent = !available
            ? t('settings.assistant.ollamaOffline', null, 'Ollama is offline — using the server default model.')
            : (!models.length
              ? t('settings.assistant.noModels', null, 'No models found in Ollama. Pull one with: ollama pull llama3.2')
              : t('settings.assistant.modelsHint', null, 'Models from your local Ollama server.'));
        }
      } catch (e) {
        if (seq !== modelRequest) return;
        model.setOptions([{ value: '', label: t('common.defaultServer', null, 'Default (server)') }]);
        modelHint.textContent = e.status === 404
          ? t('settings.assistant.modelsUnavailable', null, 'Model list unavailable — restart the server, then reopen Settings.')
          : t('settings.assistant.modelsLoadFailed', null, 'Could not load models — using the server default.');
      }
    }
    void loadAiModels();

    const providerDetails = el('div');
    providerDetails.append(ollamaFields, openaiFields);

    return [
      field(t('settings.assistant.name', null, 'Assistant name'), nameInput),
      nameHint,
      field(t('settings.assistant.provider', null, 'AI provider'), provider, {
        details: providerDetails,
        hint: t('settings.assistant.providerHint', null, 'Ollama runs entirely on this device. An OpenAI-compatible endpoint works with OpenAI, OpenRouter, LM Studio, vLLM and other servers.'),
      }),
    ];
  }

  /* ── Voice ──────────────────────────────────────────────────── */


  return buildAssistant();
}
