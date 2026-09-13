import { apiFetch, setAuth, getToken, clearAuth, logoutSession, validateSession, getTraveler } from './api.js';
import {
  getKnownUsers,
  saveKnownUser,
  renderAvatarEl,
  readAvatarFile,
} from './userProfiles.js';
import { resetUserSession } from './session.js';

const overlay = document.getElementById('login-overlay');
const appEl = document.getElementById('app');
const errorEl = document.getElementById('login-error');

const stepPick = document.getElementById('login-step-pick');
const stepPassword = document.getElementById('login-step-password');
const stepRegister = document.getElementById('login-step-register');
const profilePicker = document.getElementById('profile-picker');

const passwordInput = document.getElementById('login-password');
const usernameInput = document.getElementById('login-username');
const loginBtn = document.getElementById('login-btn');
const loginBackPick = document.getElementById('login-back-pick');
const registerToLogin = document.getElementById('register-to-login');
const loginToRegister = document.getElementById('login-to-register');
const selectedProfileEl = document.querySelector('#login-step-password .profile-selected');

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
  if (users.length) {
    showStep('pick');
  } else {
    // No profile is known on this device — which is the normal state for a new
    // browser, a private window, or cleared storage. The saved-profile picker
    // would be empty, so go straight to the manual sign-in form: an existing
    // user must be able to type their username, not be forced into "create
    // account" and told their own username is taken.
    showManualLoginStep();
  }
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
  selectedProfileEl?.classList.remove('hidden');
  renderAvatarEl(selectedAvatarEl, user);
  if (selectedNameEl) selectedNameEl.textContent = user.name || user.username;
  // A picked profile already names the user: hide the free-text field so there
  // is exactly one source of truth for the username.
  usernameInput?.classList.add('hidden');
  if (usernameInput) usernameInput.value = '';
  // Back to the picker is meaningful here; "create a profile" is not.
  loginBackPick?.classList.remove('hidden');
  loginToRegister?.classList.add('hidden');
  if (passwordInput) {
    passwordInput.value = '';
    passwordInput.focus();
  }
  showStep('password');
}

/**
 * Sign in by typing a username, for when no profile is saved on this device.
 * The picker step is skipped because it would be empty.
 */
function showManualLoginStep() {
  selectedUser = null;
  // A typed username has no avatar to show; an empty placeholder tile would
  // just add a meaningless "?" above the form.
  selectedProfileEl?.classList.add('hidden');
  // The username field must be *shown*, not merely enabled: `loginBtn` reads
  // it, and a hidden field that is still read would look like a broken login.
  usernameInput?.classList.remove('hidden');
  if (usernameInput) usernameInput.value = '';
  if (passwordInput) passwordInput.value = '';
  // Nothing to go back to, so the Back button would be a dead end; offer the
  // route that helps instead.
  loginBackPick?.classList.add('hidden');
  loginToRegister?.classList.remove('hidden');
  showStep('password');
  (usernameInput || passwordInput)?.focus();
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
  // With no saved profiles there is nothing to go back *to*; the button is
  // hidden in that case (see showManualLoginStep), so this only runs when a
  // picker exists.
  showStep('pick');
});

registerToLogin?.addEventListener('click', () => {
  // "Already have a profile? Sign in" — the escape hatch from the register
  // screen, which is all a first-time visitor on this device used to see.
  if (getKnownUsers().length) showStep('pick');
  else showManualLoginStep();
});

loginToRegister?.addEventListener('click', () => showRegisterStep());

registerBackBtn?.addEventListener('click', () => {
  registerAvatarData = null;
  if (getKnownUsers().length) showStep('pick');
  else showManualLoginStep();
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
  hideError();

  // Either a picked profile (picker step) or a typed username (manual step).
  const username = selectedUser?.username || usernameInput?.value.trim() || '';
  if (!username) {
    showError('Enter your username');
    usernameInput?.focus();
    return;
  }
  if (!passwordInput?.value) {
    showError('Enter your password');
    passwordInput?.focus();
    return;
  }

  try {
    // `authRedirect: false` — a 401 here means "bad credentials", not an
    // expired session. Without this, apiFetch's global 401 handler fires the
    // misleading "Session expired — sign in again" toast on every wrong password.
    const data = await apiFetch('/api/auth/login', {
      method: 'POST',
      authRedirect: false,
      body: JSON.stringify({
        username,
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

usernameInput?.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') passwordInput?.focus();
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
      authRedirect: false,
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

export async function logout() {
  // Invalidate the session server-side + clear the HttpOnly cookie first.
  await logoutSession();
  resetUserSession();
  appEl?.classList.add('hidden');
  showLogin();
}

export async function requireAuth() {
  // validateSession() also checks the `shiny_token` session cookie, so don't
  // bail out early just because localStorage has no token.
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
  resetUserSession();
  clearAuth();
  showLogin();
  window.dispatchEvent(new CustomEvent('app:toast', {
    detail: { message: 'Session expired — sign in again', type: 'error' },
  }));
});

export { clearAuth, getToken };
