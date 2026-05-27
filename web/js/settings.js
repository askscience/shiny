import { apiFetch, getVoiceLang, setVoiceLang } from './api.js';
import { changeLanguage } from './voice.js';

const panel = document.getElementById('settings-panel');
const select = document.getElementById('lang-select');
const openBtn = document.getElementById('settings-btn');
const closeBtn = document.getElementById('settings-close');

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

  openBtn?.addEventListener('click', () => {
    panel?.classList.remove('hidden');
  });
  closeBtn?.addEventListener('click', async () => {
    const lang = select.value;
    if (lang !== getVoiceLang()) {
      setVoiceLang(lang);
      await changeLanguage(lang);
    }
    panel?.classList.add('hidden');
  });
  panel?.addEventListener('click', (e) => {
    if (e.target === panel) panel.classList.add('hidden');
  });
}
