const API_BASE = '';

export function getToken() {
  return localStorage.getItem('token');
}

export function setAuth(token, traveler) {
  localStorage.setItem('token', token);
  if (traveler) {
    localStorage.setItem('traveler', JSON.stringify(traveler));
  }
}

export function clearAuth() {
  localStorage.removeItem('token');
  localStorage.removeItem('traveler');
}

export function getTraveler() {
  try {
    return JSON.parse(localStorage.getItem('traveler') || 'null');
  } catch {
    return null;
  }
}

export class ApiError extends Error {
  constructor(message, status) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

function handleUnauthorized() {
  clearAuth();
  window.dispatchEvent(new CustomEvent('auth:expired'));
}

export async function apiFetch(path, options = {}) {
  const headers = { ...(options.headers || {}) };
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;

  if (options.body && typeof options.body === 'string' && !headers['Content-Type']) {
    headers['Content-Type'] = 'application/json';
  }

  const res = await fetch(`${API_BASE}${path}`, { ...options, headers });

  if (options.responseType === 'blob') {
    if (res.status === 401) {
      if (options.authRedirect !== false) handleUnauthorized();
      throw new ApiError('Session expired — please sign in again', 401);
    }
    if (!res.ok) {
      const err = await res.text();
      throw new ApiError(err || res.statusText, res.status);
    }
    return res.blob();
  }

  const data = await res.json().catch(() => null);
  if (res.status === 401) {
    if (options.authRedirect !== false) handleUnauthorized();
    throw new ApiError(data?.error || 'Unauthorized', 401);
  }
  if (!res.ok) {
    throw new ApiError(data?.error || res.statusText, res.status);
  }
  return data;
}

export async function validateSession() {
  if (!getToken()) return false;
  try {
    await apiFetch('/api/travelers/me', { authRedirect: false });
    return true;
  } catch (e) {
    if (e.status === 401) {
      clearAuth();
      return false;
    }
    return !!getToken();
  }
}

export function getVoiceLang() {
  return localStorage.getItem('voice.lang') ||
    (navigator.language || 'en-US').split('-')[0];
}

export function setVoiceLang(lang) {
  localStorage.setItem('voice.lang', lang);
}
