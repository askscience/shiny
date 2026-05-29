/**
 * Start navigation via REST (bypasses agent tool when needed).
 */

import { apiFetch } from './api.js';

export async function fetchNavigationSession({ destination, to_lat, to_lon, from_lat, from_lon, profile }) {
  const q = new URLSearchParams();
  if (destination) q.set('destination', destination);
  if (to_lat != null) q.set('to_lat', String(to_lat));
  if (to_lon != null) q.set('to_lon', String(to_lon));
  if (from_lat != null) q.set('from_lat', String(from_lat));
  if (from_lon != null) q.set('from_lon', String(from_lon));
  if (profile) q.set('profile', profile);
  const res = await apiFetch(`/api/navigate/start?${q}`);
  return res.data;
}

const NAV_INTENT = /\b(navig|direction|take me|drive me|go to|portami|guidami|indicazioni|verso)\b/i;

export function looksLikeNavigationRequest(message) {
  return NAV_INTENT.test(message || '');
}

export function extractDestinationFromMessage(message) {
  if (!message) return '';
  return message
    .replace(
      /^(please\s+)?(navigate( me)? to|take me to|drive( me)? to|go to|directions to|portami a|guidami (a|verso)|naviga (verso|a)|indicazioni per)\s+/i,
      '',
    )
    .replace(/[.!?]+$/, '')
    .trim();
}

export function agentFailedNavigation(res) {
  if (res?.navigation) return false;
  const tried = res?.actions_taken?.some((a) => a.action === 'navigate_to');
  const reply = res?.reply || '';
  const toolError = /unknown action|non .*riconosciut|errore tecnico/i.test(reply);
  return tried || toolError;
}
