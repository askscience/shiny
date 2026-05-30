import { apiFetch, getVoiceLang, setVoiceLang } from './api.js';
import { changeLanguage } from './voice.js';
import { applyAccent, getStoredAccent, DEFAULT_ACCENT } from './accent.js';
import { getAiName, setAiName, setAccent } from './preferences.js';

const panel = document.getElementById('settings-panel');
const select = document.getElementById('lang-select');
const openBtn = document.getElementById('settings-btn');
const closeBtn = document.getElementById('settings-close');
const accentPicker = document.getElementById('accent-picker');
const aiNameInput = document.getElementById('ai-name-input');
const aiNameHint = document.getElementById('ai-name-hint');

function updateAiNameHint() {
  if (aiNameHint) aiNameHint.textContent = getAiName();
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

  openBtn?.addEventListener('click', () => {
    if (accentPicker) accentPicker.value = getStoredAccent();
    if (aiNameInput) aiNameInput.value = getAiName();
    updateAiNameHint();
    panel?.classList.remove('hidden');
  });

  closeBtn?.addEventListener('click', async () => {
    const lang = select.value;
    if (lang !== getVoiceLang()) {
      setVoiceLang(lang);
      await changeLanguage(lang);
    }
    setAiName(aiNameInput?.value || '');
    updateAiNameHint();
    panel?.classList.add('hidden');
  });

  panel?.addEventListener('click', (e) => {
    if (e.target === panel) panel.classList.add('hidden');
  });
}
