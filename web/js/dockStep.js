/**
 * Step summary line below the artifact dock while AI work is in progress.
 *
 * The dock is ALSO the little status bubble that hangs above the voice bar, but
 * two other rules can take it away from the line: the markup ships with
 * `hidden`, and artifacts.js hides the dock whenever the window has no saved
 * cards. Nothing re-showed it, so the bubble lived in the code without ever
 * being painted. Owning the dock's visibility here keeps the line and its
 * bubble in step — as long as the line is up, the bubble is up.
 *
 * The voice bar follows the same steps on its own: `agent:step` carries the
 * running tool's action, and the bar's dot grid (voiceDots.js) draws it.
 */

const stepEl = document.getElementById('artifact-dock-step');
const dockEl = document.getElementById('artifact-dock');

function bubbleStillNeeded() {
  const icons = document.getElementById('artifact-dock-icons');
  if (icons && icons.childElementCount > 0) return true;   // saved cards
  return document.body.classList.contains('compose-active'); // live composer
}

/**
 * Show a status line. `action` is the tool the line belongs to when it is a
 * tool's own step (agent.js passes the SSE `action`), so the voice bar knows
 * which tool is running.
 */
export function setDockStep(message, { action = '' } = {}) {
  if (!stepEl) return;
  const text = (message || '').trim();
  if (!text) return;
  stepEl.textContent = text;
  stepEl.classList.remove('hidden');
  // Paint the bubble the line lives in, whatever artifacts.js last decided.
  dockEl?.classList.remove('hidden');
  window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: text, action: String(action || '') } }));
}

export function clearDockStep() {
  if (!stepEl) return;
  stepEl.textContent = '';
  stepEl.classList.add('hidden');
  // With no line left, the bubble is only worth keeping for saved cards.
  if (!bubbleStillNeeded()) dockEl?.classList.add('hidden');
  window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: null } }));
}
