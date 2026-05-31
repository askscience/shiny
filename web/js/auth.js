import { apiFetch, setAuth, getToken, clearAuth, validateSession, getTraveler } from './api.js';
import {
  getKnownUsers,
  saveKnownUser,
  renderAvatarEl,
  readAvatarFile,
} from './userProfiles.js';

const overlay = document.getElementById('login-overlay');
const appEl = document.getElementById('app');
const errorEl = document.getElementById('login-error');

const stepPick = document.getElementById('login-step-pick');
const stepPassword = document.getElementById('login-step-password');
const stepRegister = document.getElementById('login-step-register');
const profilePicker = document.getElementById('profile-picker');

const passwordInput = document.getElementById('login-password');
const loginBtn = document.getElementById('login-btn');
const loginBackPick = document.getElementById('login-back-pick');

const registerBackBtn = document.getElementById('login-back-from-register');
const registerBtn = document.getElementById('register-btn');
const registerUsernameInput = document.getElementById('register-username');
const registerPasswordInput = document.getElementById('register-password');
const registerAvatarInput = document.getElementById('register-avatar');
const registerAvatarPreview = document.getElementById('register-avatar-preview');

const selectedAvatarEl = document.getElementById('login-selected-avatar');
const selectedNameEl = document.getElementById('login-selected-name');

let selectedUser = null;
let registerAvatarData = null;

function showError(msg) {
  errorEl.textContent = msg;
  errorEl.classList.remove('hidden');
}

function hideError() {
  errorEl.classList.add('hidden');
}

function showStep(step) {
  stepPick?.classList.toggle('hidden', step !== 'pick');
  stepPassword?.classList.toggle('hidden', step !== 'password');
  stepRegister?.classList.toggle('hidden', step !== 'register');
  hideError();
}

export function showLogin() {
  overlay.classList.remove('hidden');
  appEl?.classList.add('hidden');
  selectedUser = null;
  registerAvatarData = null;
  renderProfilePicker();
  const users = getKnownUsers();
  showStep(users.length ? 'pick' : 'register');
}

export function hideLogin() {
  overlay.classList.add('hidden');
}

function onAuthSuccess(traveler) {
  saveKnownUser(traveler);
  hideLogin();
  appEl?.classList.remove('hidden');
  window.dispatchEvent(new CustomEvent('auth:success'));
}

function renderProfilePicker() {
  if (!profilePicker) return;
  profilePicker.innerHTML = '';

  getKnownUsers().forEach((user) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'profile-tile';
    btn.setAttribute('aria-label', `Sign in as ${user.name}`);

    const avatar = document.createElement('div');
    avatar.className = 'profile-avatar';
    renderAvatarEl(avatar, user);

    const label = document.createElement('span');
    label.className = 'profile-tile-name';
    label.textContent = user.name || user.username;

    btn.append(avatar, label);
    btn.addEventListener('click', () => selectUser(user));
    profilePicker.appendChild(btn);
  });

  const addBtn = document.createElement('button');
  addBtn.type = 'button';
  addBtn.className = 'profile-tile profile-tile-add';
  addBtn.setAttribute('aria-label', 'Add new user');

  const addAvatar = document.createElement('div');
  addAvatar.className = 'profile-avatar profile-avatar-add';
  addAvatar.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg>';

  const addLabel = document.createElement('span');
  addLabel.className = 'profile-tile-name';
  addLabel.textContent = 'Add user';

  addBtn.append(addAvatar, addLabel);
  addBtn.addEventListener('click', () => showRegisterStep());
  profilePicker.appendChild(addBtn);
}

function selectUser(user) {
  selectedUser = user;
  renderAvatarEl(selectedAvatarEl, user);
  if (selectedNameEl) selectedNameEl.textContent = user.name || user.username;
  if (passwordInput) {
    passwordInput.value = '';
    passwordInput.focus();
  }
  showStep('password');
}

function showRegisterStep() {
  selectedUser = null;
  registerAvatarData = null;
  if (registerUsernameInput) registerUsernameInput.value = '';
  if (registerPasswordInput) registerPasswordInput.value = '';
  if (registerAvatarInput) registerAvatarInput.value = '';
  renderAvatarEl(registerAvatarPreview, { name: '', avatar: null });
  registerAvatarPreview?.classList.add('profile-avatar--placeholder');
  showStep('register');
  registerUsernameInput?.focus();
}

loginBackPick?.addEventListener('click', () => {
  selectedUser = null;
  showStep('pick');
});

registerBackBtn?.addEventListener('click', () => {
  registerAvatarData = null;
  showStep(getKnownUsers().length ? 'pick' : 'register');
});

registerAvatarInput?.addEventListener('change', async () => {
  const file = registerAvatarInput.files?.[0];
  if (!file) return;
  try {
    registerAvatarData = await readAvatarFile(file);
    registerAvatarPreview?.classList.remove('profile-avatar--placeholder');
    renderAvatarEl(registerAvatarPreview, { name: registerUsernameInput?.value || '', avatar: registerAvatarData });
  } catch (e) {
    showError(e.message);
    registerAvatarInput.value = '';
  }
});

loginBtn?.addEventListener('click', async () => {
  if (!selectedUser) return;
  hideError();
  try {
    const data = await apiFetch('/api/auth/login', {
      method: 'POST',
      body: JSON.stringify({
        username: selectedUser.username,
        password: passwordInput.value,
      }),
    });
    setAuth(data.token, data.traveler);
    onAuthSuccess(data.traveler);
  } catch (e) {
    showError(e.message);
  }
});

passwordInput?.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') loginBtn?.click();
});

registerBtn?.addEventListener('click', async () => {
  hideError();
  const username = registerUsernameInput?.value.trim() || '';
  const password = registerPasswordInput?.value || '';

  if (!username) {
    showError('Choose a username');
    registerUsernameInput?.focus();
    return;
  }
  if (!password) {
    showError('Choose a password');
    registerPasswordInput?.focus();
    return;
  }

  try {
    const data = await apiFetch('/api/auth/register', {
      method: 'POST',
      body: JSON.stringify({
        username,
        password,
        avatar: registerAvatarData,
      }),
    });
    setAuth(data.token, data.traveler);
    onAuthSuccess(data.traveler);
  } catch (e) {
    showError(e.message);
  }
});

export function logout() {
  clearAuth();
  appEl?.classList.add('hidden');
  showLogin();
}

export async function requireAuth() {
  if (!getToken()) {
    showLogin();
    return false;
  }
  const valid = await validateSession();
  if (!valid) {
    clearAuth();
    showLogin();
    return false;
  }
  const traveler = getTraveler();
  if (traveler) saveKnownUser(traveler);
  return true;
}

window.addEventListener('auth:expired', () => {
  clearAuth();
  showLogin();
  window.dispatchEvent(new CustomEvent('app:toast', {
    detail: { message: 'Session expired — sign in again', type: 'error' },
  }));
});

export { clearAuth, getToken };
