/**
 * Image plugin REST client.
 *
 * Thin wrappers over the plugin's own routes. All pixel work happens on the
 * server; this module only moves JSON, raw RGBA and the looks strip around.
 */

import { apiFetch, getToken } from '../../js/api.js';

/** GET/POST/… that unwraps the `{success,data}` envelope. */
async function api(path, options = {}) {
  const res = await apiFetch(path, options);
  return res?.data ?? null;
}

const enc = encodeURIComponent;

/* ── Documents ──────────────────────────────────────────────────── */

export function listImages() {
  return api('/api/images');
}

export function fetchImageMeta(id) {
  return api(`/api/images/${enc(id)}`);
}

export function deleteImage(id) {
  return api(`/api/images/${enc(id)}`, { method: 'DELETE' });
}

export async function createImage(file) {
  const q = file?.name ? `?name=${encodeURIComponent(file.name)}` : '';
  const res = await apiFetch(`/api/images${q}`, {
    method: 'POST',
    body: file,
    headers: { 'Content-Type': file?.type || 'application/octet-stream' },
  });
  return res?.data ?? null;
}

/* ── Pixels ─────────────────────────────────────────────────────── */

function authHeaders(extra = {}) {
  const token = getToken();
  return token ? { ...extra, Authorization: `Bearer ${token}` } : extra;
}

async function errorText(res) {
  const text = await res.text();
  try {
    const j = JSON.parse(text);
    return j.error || j.message || text;
  } catch (_) {
    return text || res.statusText;
  }
}

/** The current pixels as raw RGBA. */
export async function fetchRender(id) {
  const res = await fetch(`/api/images/${enc(id)}/render?raw=true`, { headers: authHeaders() });
  if (!res.ok) throw new Error(await errorText(res));
  const w = Number(res.headers.get('x-image-width') || 0);
  const h = Number(res.headers.get('x-image-height') || 0);
  return { w, h, buf: await res.arrayBuffer() };
}

/**
 * Apply a request server-side and stream the resulting RGBA back.
 * `body` is `{ operations: [...] }` (destructive batch) or `{ settings: {…} }`
 * (partial dial update); `commit:false` previews without writing. The returned
 * `look` / `adjust` are the document's live state after the run.
 */
export async function rawApply(id, body, { commit = true } = {}) {
  const res = await fetch(
    `/api/images/${enc(id)}/apply?raw=true&commit=${commit ? 'true' : 'false'}`,
    {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify(body),
    },
  );
  if (!res.ok) throw new Error(await errorText(res));
  const w = Number(res.headers.get('x-image-width') || 0);
  const h = Number(res.headers.get('x-image-height') || 0);
  const look = res.headers.get('x-image-look') || null;
  let adjust = {};
  try {
    adjust = JSON.parse(res.headers.get('x-image-adjust') || '{}') || {};
  } catch (_) {
    adjust = {};
  }
  return { w, h, buf: await res.arrayBuffer(), look, adjust };
}

/**
 * The look-preview strip: one PNG holding every look applied to a small crop
 * of this image, laid out left to right.
 */
export async function fetchLooks(id) {
  const res = await fetch(`/api/images/${enc(id)}/looks`, { headers: authHeaders() });
  if (!res.ok) throw new Error(await errorText(res));
  return res.blob();
}

/** Raw bytes of a home file (files plugin), for opening images from it. */
export function blobFromHome(path) {
  return apiFetch(`/api/files/raw?path=${enc(path)}`, { responseType: 'blob' });
}

/* ── Export ─────────────────────────────────────────────────────── */

export async function fetchPng(id) {
  const res = await apiFetch(`/api/images/${enc(id)}/data`, { responseType: 'blob' });
  return res;
}
