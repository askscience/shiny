import { getLocale, t } from './i18n.js';

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
  // Best-effort clear of a legacy non-HttpOnly cookie. The current cookie is
  // HttpOnly, so JS can't remove it — server logout (logoutSession) is what
  // actually clears it via `Set-Cookie`.
  document.cookie = `${SESSION_COOKIE}=; Path=/; Max-Age=0; SameSite=Lax`;
}

/**
 * Server-side logout: invalidates the stored session token and clears the
 * HttpOnly `shiny_token` cookie (JavaScript cannot clear an HttpOnly cookie
 * directly). Then wipes the client-side copy. Safe to call even when the
 * token is already invalid.
 */
export async function logoutSession() {
  try {
    await apiFetch('/api/auth/logout', { method: 'POST', authRedirect: false });
  } catch (_) {
    /* already-invalid token — clearing locally is still correct */
  }
  clearAuth();
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

let sessionCheck = null;

/** Coalesce simultaneous 401s; only a confirmed cookie rejection expires auth. */
async function handleUnauthorized() {
  if (sessionCheck) return sessionCheck;
  const token = getToken();
  const traveler = localStorage.getItem('traveler');
  sessionCheck = (async () => {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 5000);
    try {
      const res = await fetch('/api/travelers/me', { headers: {}, signal: controller.signal });
      // A different login completed while this request was in flight.
      if (getToken() !== token || localStorage.getItem('traveler') !== traveler) return false;
      if (res.ok) {
        const data = await res.json();
        if (!data?.data || getToken() !== token || localStorage.getItem('traveler') !== traveler) return false;
        localStorage.removeItem('token'); // discard the stale bearer; use the valid cookie
        localStorage.setItem('traveler', JSON.stringify(data.data));
        return true;
      }
      if (res.status === 401) {
        clearAuth();
        window.dispatchEvent(new CustomEvent('auth:expired'));
      }
    } catch (_) {
      // Connectivity and malformed responses cannot prove session expiry.
    } finally {
      clearTimeout(timer);
    }
    return false;
  })();
  try { return await sessionCheck; }
  finally { sessionCheck = null; }
}

/** Shared authentication/recovery for JSON, binary and streaming requests.
 * A rejected request is retried once only after cookie authentication succeeds.
 * Network failures are never retried automatically (a mutation may have run).
 */
export async function apiRequest(path, options = {}) {
  const { authRedirect = true, responseType, ...requestOptions } = options;
  const headers = new Headers(requestOptions.headers);
  const token = getToken();
  if (token && !headers.has('Authorization')) headers.set('Authorization', `Bearer ${token}`);
  headers.set('Accept-Language', getLocale());
  if (typeof options.body === 'string' && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json');
  }
  let res = await fetch(`${API_BASE}${path}`, { ...requestOptions, headers });
  if (res.status === 401 && authRedirect && await handleUnauthorized()) {
    // Release the first response body before issuing a replacement request.
    await res.body?.cancel();
    headers.delete('Authorization');
    res = await fetch(`${API_BASE}${path}`, { ...requestOptions, headers });
  }
  return res;
}

export async function apiFetch(path, options = {}) {
  const res = await apiRequest(path, options);

  if (options.responseType === 'blob') {
    if (res.status === 401) {
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
    throw new ApiError(data?.error || 'Unauthorized', 401);
  }
  if (!res.ok) {
    throw new ApiError(data?.error || text || res.statusText, res.status);
  }
  if (text && data === null && text.trim() !== 'null') {
    throw new ApiError(t('api.invalidResponse', null, 'The server returned an invalid response. Try again.'), 502);
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

/** Voice/assistant language is scoped per traveler, so every user keeps their own. */
function voiceLangKey() {
  const id = getTraveler()?.id;
  return id ? `voice.lang.${id}` : 'voice.lang';
}

/** Explicit per-user language choice, or `null` when following the interface. */
export function getVoiceLangExplicit() {
  return localStorage.getItem(voiceLangKey()) || null;
}

/**
 * Resolved language for speech recognition, spoken replies and the language the
 * assistant writes in: the explicit per-user choice, else the interface
 * language. The interface language follows the system language until the user
 * picks one (Settings → Appearance → Interface language), so this is the system
 * language out of the box — and choosing an interface language switches the
 * voice with it, until a separate voice language is picked under Voice.
 */
export function getVoiceLang() {
  return getVoiceLangExplicit() || getLocale().split('-')[0];
}

/** Persist an explicit language. Pass empty/null to clear it (back to auto). */
export function setVoiceLang(lang) {
  if (lang) localStorage.setItem(voiceLangKey(), lang);
  else localStorage.removeItem(voiceLangKey());
}

/** Follow the interface language again (drop the explicit override). */
export function clearVoiceLang() {
  localStorage.removeItem(voiceLangKey());
}
