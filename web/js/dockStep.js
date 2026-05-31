/** Step summary line below the artifact dock while AI work is in progress. */

const stepEl = document.getElementById('artifact-dock-step');

export function setDockStep(message) {
  if (!stepEl) return;
  const text = (message || '').trim();
  if (!text) return;
  stepEl.textContent = text;
  stepEl.classList.remove('hidden');
  window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: text } }));
}

export function clearDockStep() {
  if (!stepEl) return;
  stepEl.textContent = '';
  stepEl.classList.add('hidden');
  window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: null } }));
}
