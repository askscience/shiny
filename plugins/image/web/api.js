/**
 * Image plugin REST client.
 *
 * Thin wrappers over the plugin's own routes. All pixel work happens on the
 * server; this module only moves JSON, raw RGBA and selection masks around.
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

export function fetchImage(id) {
  return api(`/api/images/${enc(id)}`);
}

export function renameImage(id, title) {
  return api(`/api/images/${enc(id)}`, { method: 'PUT', body: JSON.stringify({ title }) });
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

/** The flattened composite as raw RGBA. */
export async function fetchRender(id) {
  const res = await fetch(`/api/images/${enc(id)}/render?raw=true`, { headers: authHeaders() });
  if (!res.ok) throw new Error(await errorText(res));
  const w = Number(res.headers.get('x-image-width') || 0);
  const h = Number(res.headers.get('x-image-height') || 0);
  return { w, h, buf: await res.arrayBuffer() };
}

/**
 * Apply operations to one layer server-side and stream the composited RGBA
 * back. `commit:false` previews without writing to the database.
 */
export async function rawApply(id, operations, { commit = true, layerId = null } = {}) {
  const payload = { operations };
  if (layerId) payload.layer_id = layerId;
  const res = await fetch(
    `/api/images/${enc(id)}/apply?raw=true&commit=${commit ? 'true' : 'false'}`,
    {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify(payload),
    },
  );
  if (!res.ok) throw new Error(await errorText(res));
  const w = Number(res.headers.get('x-image-width') || 0);
  const h = Number(res.headers.get('x-image-height') || 0);
  return { w, h, buf: await res.arrayBuffer() };
}

/** Apply without streaming pixels back (used when the window is not showing). */
export function applyOps(id, operations, layerId = null) {
  const payload = { operations };
  if (layerId) payload.layer_id = layerId;
  return api(`/api/images/${enc(id)}/apply`, { method: 'POST', body: JSON.stringify(payload) });
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

/* ── Layers ─────────────────────────────────────────────────────── */

export function listLayers(id) {
  return api(`/api/images/${enc(id)}/layers`);
}

export function createLayer(id, body = {}) {
  return api(`/api/images/${enc(id)}/layers`, { method: 'POST', body: JSON.stringify(body) });
}

export function updateLayer(id, layerId, patch) {
  return api(`/api/images/${enc(id)}/layers/${enc(layerId)}`, {
    method: 'PUT',
    body: JSON.stringify(patch),
  });
}

export function deleteLayer(id, layerId) {
  return api(`/api/images/${enc(id)}/layers/${enc(layerId)}`, { method: 'DELETE' });
}

export function duplicateLayer(id, layerId) {
  return api(`/api/images/${enc(id)}/layers/${enc(layerId)}/duplicate`, { method: 'POST' });
}

export function mergeLayer(id, layerId) {
  return api(`/api/images/${enc(id)}/layers/${enc(layerId)}/merge`, { method: 'POST' });
}

export function reorderLayers(id, ids) {
  return api(`/api/images/${enc(id)}/layers/reorder`, {
    method: 'POST',
    body: JSON.stringify({ ids }),
  });
}

export function flattenImage(id) {
  return api(`/api/images/${enc(id)}/flatten`, { method: 'POST' });
}

/** Import a file into a new layer (sent as raw bytes, not multipart). */
export async function uploadLayer(id, file, { name, groupId } = {}) {
  const params = new URLSearchParams();
  const label = name || file?.name;
  if (label) params.set('name', label);
  if (groupId) params.set('group_id', groupId);
  const q = params.toString() ? `?${params.toString()}` : '';
  const res = await apiFetch(`/api/images/${enc(id)}/layers${q}`, {
    method: 'POST',
    body: file,
    headers: { 'Content-Type': file?.type || 'application/octet-stream' },
  });
  return res?.data ?? null;
}

/** Replace a layer's pixels from a file (raw bytes). */
export async function replaceLayerImage(id, layerId, file) {
  const res = await apiFetch(`/api/images/${enc(id)}/layers/${enc(layerId)}/image`, {
    method: 'POST',
    body: file,
    headers: { 'Content-Type': file?.type || 'application/octet-stream' },
  });
  return res?.data ?? null;
}

export function layerThumbUrl(id, layerId) {
  return `/api/images/${enc(id)}/layers/${enc(layerId)}/thumb`;
}

/** Raw bytes of a home file (files plugin), for opening images from it. */
export function blobFromHome(path) {
  return apiFetch(`/api/files/raw?path=${enc(path)}`, { responseType: 'blob' });
}

/** Layer thumbnail as a Blob (authenticated). */
export function fetchLayerThumb(id, layerId) {
  return apiFetch(layerThumbUrl(id, layerId), { responseType: 'blob' });
}

/* ── Selection ──────────────────────────────────────────────────── */

export function getSelection(id) {
  return api(`/api/images/${enc(id)}/selection`);
}

/** `mask` is a Uint8Array of width×height coverage bytes. */
export function setSelection(id, width, height, mask) {
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < mask.length; i += chunk) {
    binary += String.fromCharCode.apply(null, mask.subarray(i, i + chunk));
  }
  const data = btoa(binary);
  return api(`/api/images/${enc(id)}/selection`, {
    method: 'PUT',
    body: JSON.stringify({ width, height, data }),
  });
}

export function clearSelection(id) {
  return api(`/api/images/${enc(id)}/selection`, { method: 'DELETE' });
}

/* ── Document geometry ──────────────────────────────────────────── */

export function cropDocument(id, { x, y, width, height }) {
  return api(`/api/images/${enc(id)}/crop`, {
    method: 'POST',
    body: JSON.stringify({ x, y, width, height }),
  });
}

export function resizeDocument(id, width, height) {
  return api(`/api/images/${enc(id)}/resize`, {
    method: 'POST',
    body: JSON.stringify({ width, height }),
  });
}

export function rotateDocument(id, angle) {
  return api(`/api/images/${enc(id)}/rotate`, {
    method: 'POST',
    body: JSON.stringify({ angle }),
  });
}

export function flipDocument(id, axis) {
  return api(`/api/images/${enc(id)}/flip`, {
    method: 'POST',
    body: JSON.stringify({ axis }),
  });
}

/* ── Export ─────────────────────────────────────────────────────── */

export async function fetchPng(id) {
  const res = await apiFetch(`/api/images/${enc(id)}/data`, { responseType: 'blob' });
  return res;
}
