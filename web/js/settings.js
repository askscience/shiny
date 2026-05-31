import { apiFetch, getVoiceLang, setVoiceLang, getTraveler } from './api.js';
import { logout } from './auth.js';
import { changeLanguage } from './voice.js';
import { applyAccent, getStoredAccent } from './accent.js';
import { getAiName, setAiName, setAccent } from './preferences.js';
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

let pendingAvatar = undefined;

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
}
