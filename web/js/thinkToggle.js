/**
 * thinkToggle.js — the brain button: Ollama's `think` switch for the chat.
 *
 * Brain = the model reasons and its trace folds into the answer bubble; the
 * slashed brain = thinking off (faster answers). The button only claims the
 * capability the resolved provider/model really has (GET /api/ai/thinking).
 * While that answer is unknown — or when the probe fails — the toggle stays
 * usable, because the core re-checks the same capability before every turn
 * and downgrades to the model default rather than failing.
 *
 * The voice-bar right-click menu renders the same entry from the cached
 * capability here, so both surfaces always show the same state.
 */

import { setIcon } from '../ui/index.js';
import { apiFetch } from './api.js';
import { getAiProvider, getOllamaModel, getThink, setThink } from './preferences.js';

const btn = document.getElementById('compose-think-btn');
const glyph = btn?.querySelector('[data-icon]') || null;

/** Last capability answer; `null` = not resolved yet. */
let support = null;

/** Cached, synchronous — the context menu reads it while building. */
export function getThinkingSupport() {
  return support;
}

/** False only when the probe positively established the model cannot think. */
export function canThink() {
  return !support || support.supported;
}

function unsupportedReason() {
  if (support?.provider === 'openai') return 'Thinking needs an Ollama model';
  if (support?.model) return `${support.model} cannot think`;
  return 'This model cannot think';
}

function syncSurface() {
  if (!btn) return;
  const on = getThink();
  const capable = canThink();
  btn.setAttribute('aria-pressed', on ? 'true' : 'false');
  btn.disabled = !capable;
  btn.title = capable
    ? (on ? 'Thinking on — click to turn it off' : 'Thinking off — click to turn it on')
    : unsupportedReason();
  btn.setAttribute('aria-label', btn.title);
  if (glyph) {
    void setIcon(glyph, on && capable ? 'ui/brain' : 'ui/brain-off', { size: 17 });
  }
}

/** Was the capability resolved for a different model/provider than the
 *  current choice? (Then it must be probed again.) */
function probeIsStale() {
  if (!support) return true;
  if (support.provider !== getAiProvider()) return true;
  const model = getOllamaModel();
  return !!model && model !== support.model;
}

async function refreshSupport() {
  try {
    const res = await apiFetch('/api/ai/thinking', { authRedirect: false });
    const data = res?.data || {};
    support = {
      supported: !!data.supported,
      provider: data.provider || 'ollama',
      model: data.model || '',
    };
  } catch (_) {
    // Unknown is not "cannot think": leave the toggle usable and let the
    // core's own check do the enforcing.
    support = null;
  }
  syncSurface();
}

export function initThinkToggle() {
  if (!btn) return;

  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    if (!canThink()) return;
    setThink(!getThink());
  });

  // Fired by setThink (state) and by setOllamaModel/setAiProvider (capability).
  window.addEventListener('ai:settings', () => {
    syncSurface();
    if (probeIsStale()) void refreshSupport();
  });
  // Another window (settings) changed the assistant — keep in step.
  window.addEventListener('storage', (e) => {
    if (e.key && e.key.startsWith('ai.')) {
      syncSurface();
      if (probeIsStale()) void refreshSupport();
    }
  });

  syncSurface();
  void refreshSupport();
}
