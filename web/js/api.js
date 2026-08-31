const API_BASE = '';
const SESSION_COOKIE = 'shiny_token';

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
  // Drop the session cookie too, so an explicit logout doesn't get
  // auto-restored by the cookie on the next page load.
  document.cookie = `${SESSION_COOKIE}=; Path=/; Max-Age=0; SameSite=Lax`;
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

async function handleUnauthorized() {
  // A 401 can be a stale localStorage bearer token while the `shiny_token`
  // session cookie is still valid. Re-check against the cookie first so a
  // transient 401 never logs the user out.
  try {
    const res = await fetch('/api/travelers/me', { headers: {} });
    if (res.ok) {
      const data = await res.json().catch(() => null);
      if (data?.data) {
        localStorage.setItem('traveler', JSON.stringify(data.data));
      }
      return; // still authenticated via the session cookie
    }
  } catch (_) {
    /* network error — fall through to logout */
  }
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

  const text = await res.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = null;
  }

  if (res.status === 401) {
    if (options.authRedirect !== false) handleUnauthorized();
    throw new ApiError(data?.error || 'Unauthorized', 401);
  }
  if (!res.ok) {
    throw new ApiError(data?.error || text || res.statusText, res.status);
  }
  return data;
}

export async function validateSession() {
  // Don't short-circuit on a missing localStorage token: the `shiny_token`
  // session cookie (sent automatically by the browser) may still be valid.
  try {
    const res = await apiFetch('/api/travelers/me', { authRedirect: false });
    if (res?.data) {
      localStorage.setItem('traveler', JSON.stringify(res.data));
    }
    return true;
  } catch (e) {
    // Report the 401 but DON'T wipe the session here — a transient failure
    // must not log the user out. The caller (requireAuth) decides whether to
    // clear auth and show the login screen.
    if (e.status === 401) return false;
    // Transient / server errors must not log the user out.
    return true;
  }
}

export function getVoiceLang() {
  return localStorage.getItem('voice.lang') ||
    (navigator.language || 'en-US').split('-')[0];
}

export function setVoiceLang(lang) {
  localStorage.setItem('voice.lang', lang);
}
