import { apiFetch, getVoiceLang, setVoiceLang, getTraveler } from './api.js';
import { logout } from './auth.js';
import { changeLanguage } from './voice.js';
import { applyAccent, getStoredAccent } from './accent.js';
import { getAiName, setAiName, setAccent, getOllamaModel, setOllamaModel } from './preferences.js';
import { saveKnownUser, renderAvatarEl, readAvatarFile } from './userProfiles.js';

const panel = document.getElementById('settings-panel');
const select = document.getElementById('lang-select');
const openBtn = document.getElementById('settings-btn');
const closeBtn = document.getElementById('settings-close');
const logoutBtn = document.getElementById('settings-logout');
const accentPicker = document.getElementById('accent-picker');
const aiNameInput = document.getElementById('ai-name-input');
const aiNameHint = document.getElementById('ai-name-hint');
const profileNameInput = document.getElementById('profile-name-input');
const profileAvatarInput = document.getElementById('profile-avatar-input');
const profileAvatarPreview = document.getElementById('profile-avatar-preview');
const ollamaModelSelect = document.getElementById('ollama-model-select');
const ollamaModelHint = document.getElementById('ollama-model-hint');

let pendingAvatar = undefined;
let serverDefaultModel = '';

function updateAiNameHint() {
  if (aiNameHint) aiNameHint.textContent = getAiName();
}

function loadProfileFields() {
  const traveler = getTraveler();
  if (profileNameInput) profileNameInput.value = traveler?.name || '';
  pendingAvatar = undefined;
  renderAvatarEl(profileAvatarPreview, {
    name: traveler?.name || traveler?.username || '',
    avatar: traveler?.avatar || null,
  });
}

function populateOllamaModelSelect(models, selected) {
  if (!ollamaModelSelect) return;
  ollamaModelSelect.innerHTML = '';

  const defaultOpt = document.createElement('option');
  defaultOpt.value = '';
  defaultOpt.textContent = serverDefaultModel
    ? `Default (${serverDefaultModel})`
    : 'Default (server)';
  ollamaModelSelect.appendChild(defaultOpt);

  models.forEach((name) => {
    const opt = document.createElement('option');
    opt.value = name;
    opt.textContent = name;
    ollamaModelSelect.appendChild(opt);
  });

  const stored = selected ?? getOllamaModel();
  if (stored && [...ollamaModelSelect.options].some((o) => o.value === stored)) {
    ollamaModelSelect.value = stored;
  } else {
    ollamaModelSelect.value = '';
  }
}

async function loadOllamaModels() {
  if (!ollamaModelSelect) return;
  try {
    const res = await apiFetch('/api/ollama/models');
    const { models = [], default: defaultModel, available } = res.data || {};
    serverDefaultModel = defaultModel || '';
    populateOllamaModelSelect(models);

    if (ollamaModelHint) {
      if (!available) {
        ollamaModelHint.textContent = 'Ollama is offline — using the server default model.';
      } else if (!models.length) {
        ollamaModelHint.textContent = 'No models found in Ollama. Pull one with: ollama pull llama3.2';
      } else {
        ollamaModelHint.textContent = 'Models from your local Ollama server.';
      }
    }
  } catch (e) {
    populateOllamaModelSelect([]);
    if (ollamaModelHint) {
      if (e.status === 404) {
        ollamaModelHint.textContent = 'Model list unavailable — restart the server, then refresh the page.';
      } else {
        ollamaModelHint.textContent = 'Could not load models — using the server default.';
      }
    }
  }
}

export async function initSettings() {
  try {
    const res = await apiFetch('/api/voice/languages');
    select.innerHTML = '';
    res.data.forEach((lang) => {
      const opt = document.createElement('option');
      opt.value = lang.code;
      opt.textContent = `${lang.code.toUpperCase()}${lang.vosk_available ? '' : ' (TTS only)'}`;
      select.appendChild(opt);
    });
    select.value = getVoiceLang();
  } catch (_) {
    select.innerHTML = '<option value="en">EN</option>';
  }

  if (accentPicker) {
    accentPicker.value = getStoredAccent();
    accentPicker.addEventListener('input', (e) => {
      const hex = e.target.value;
      setAccent(hex);
      applyAccent(hex);
    });
  }

  if (aiNameInput) {
    aiNameInput.value = getAiName();
    updateAiNameHint();
    aiNameInput.addEventListener('input', () => {
      setAiName(aiNameInput.value);
      updateAiNameHint();
    });
  }

  ollamaModelSelect?.addEventListener('change', () => {
    setOllamaModel(ollamaModelSelect.value);
  });

  await loadOllamaModels();

  profileAvatarInput?.addEventListener('change', async () => {
    const file = profileAvatarInput.files?.[0];
    if (!file) return;
    try {
      pendingAvatar = await readAvatarFile(file);
      renderAvatarEl(profileAvatarPreview, {
        name: profileNameInput?.value || getTraveler()?.name || '',
        avatar: pendingAvatar,
      });
    } catch (e) {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: e.message, type: 'error' },
      }));
      profileAvatarInput.value = '';
    }
  });

  openBtn?.addEventListener('click', () => {
    if (accentPicker) accentPicker.value = getStoredAccent();
    if (aiNameInput) aiNameInput.value = getAiName();
    updateAiNameHint();
    loadProfileFields();
    void loadOllamaModels();
    panel?.classList.remove('hidden');
  });

  logoutBtn?.addEventListener('click', () => {
    panel?.classList.add('hidden');
    logout();
  });

  closeBtn?.addEventListener('click', async () => {
    const lang = select.value;
    if (lang !== getVoiceLang()) {
      setVoiceLang(lang);
      await changeLanguage(lang);
    }
    setAiName(aiNameInput?.value || '');
    updateAiNameHint();
    setOllamaModel(ollamaModelSelect?.value || '');

    const name = profileNameInput?.value.trim();
    const traveler = getTraveler();
    const body = {};
    if (name && name !== traveler?.name) body.name = name;
    if (pendingAvatar !== undefined) body.avatar = pendingAvatar;

    if (Object.keys(body).length) {
      try {
        const res = await apiFetch('/api/travelers/me', {
          method: 'PUT',
          body: JSON.stringify(body),
        });
        if (res?.data) {
          localStorage.setItem('traveler', JSON.stringify(res.data));
          saveKnownUser(res.data);
        }
      } catch (e) {
        window.dispatchEvent(new CustomEvent('app:toast', {
          detail: { message: e.message || 'Could not save profile', type: 'error' },
        }));
      }
    }

    pendingAvatar = undefined;
    panel?.classList.add('hidden');
  });

  panel?.addEventListener('click', (e) => {
    if (e.target === panel) panel.classList.add('hidden');
  });
}

export function refreshSettingsUI() {
  if (accentPicker) accentPicker.value = getStoredAccent();
  if (aiNameInput) aiNameInput.value = getAiName();
  updateAiNameHint();
  loadProfileFields();
  if (select) select.value = getVoiceLang();
  void loadOllamaModels();
}
