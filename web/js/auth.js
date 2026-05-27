import { apiFetch, setAuth, getToken, clearAuth, validateSession } from './api.js';

const overlay = document.getElementById('login-overlay');
const appEl = document.getElementById('app');
const emailInput = document.getElementById('login-email');
const passwordInput = document.getElementById('login-password');
const nameInput = document.getElementById('register-name');
const loginBtn = document.getElementById('login-btn');
const registerBtn = document.getElementById('register-btn');
const errorEl = document.getElementById('login-error');

export function showLogin() {
  overlay.classList.remove('hidden');
  appEl?.classList.add('hidden');
}

export function hideLogin() {
  overlay.classList.add('hidden');
}

function showError(msg) {
  errorEl.textContent = msg;
  errorEl.classList.remove('hidden');
}

function onAuthSuccess() {
  hideLogin();
  appEl?.classList.remove('hidden');
  window.dispatchEvent(new CustomEvent('auth:success'));
}

loginBtn.addEventListener('click', async () => {
  errorEl.classList.add('hidden');
  try {
    const data = await apiFetch('/api/auth/login', {
      method: 'POST',
      body: JSON.stringify({
        email: emailInput.value.trim(),
        password: passwordInput.value,
      }),
    });
    setAuth(data.token, data.traveler);
    onAuthSuccess();
  } catch (e) {
    showError(e.message);
  }
});

registerBtn.addEventListener('click', async () => {
  errorEl.classList.add('hidden');
  nameInput.classList.remove('hidden');
  const name = nameInput.value.trim();
  if (!name) {
    nameInput.classList.remove('hidden');
    nameInput.focus();
    showError('Enter your name to register');
    return;
  }
  try {
    const data = await apiFetch('/api/auth/register', {
      method: 'POST',
      body: JSON.stringify({
        name,
        email: emailInput.value.trim(),
        password: passwordInput.value,
      }),
    });
    setAuth(data.token, data.traveler);
    onAuthSuccess();
  } catch (e) {
    showError(e.message);
  }
});

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
