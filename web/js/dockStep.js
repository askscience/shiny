/**
 * Step summary line below the artifact dock while AI work is in progress.
 *
 * The dock is ALSO the little status bubble that hangs under the orb, but two
 * other rules can take it away from the line: the markup ships with `hidden`,
 * and artifacts.js hides the dock whenever the window has no saved cards.
 * Nothing re-showed it, so the bubble lived in the code without ever being
 * painted. Owning the dock's visibility here keeps the line and its bubble in
 * step — as long as the line is up, the bubble is up.
 *
 * Being a child of #chrome-bottom, the bubble needs no hiding of its own: it
 * rides the orb's reveal, so with autohide on the two slide away together.
 */

const stepEl = document.getElementById('artifact-dock-step');
const dockEl = document.getElementById('artifact-dock');

function bubbleStillNeeded() {
  const icons = document.getElementById('artifact-dock-icons');
  if (icons && icons.childElementCount > 0) return true;   // saved cards
  return document.body.classList.contains('compose-active'); // live composer
}

export function setDockStep(message) {
  if (!stepEl) return;
  const text = (message || '').trim();
  if (!text) return;
  stepEl.textContent = text;
  stepEl.classList.remove('hidden');
  // Paint the bubble the line lives in, whatever artifacts.js last decided.
  dockEl?.classList.remove('hidden');
  window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: text } }));
}

export function clearDockStep() {
  if (!stepEl) return;
  stepEl.textContent = '';
  stepEl.classList.add('hidden');
  // With no line left, the bubble is only worth keeping for saved cards.
  if (!bubbleStillNeeded()) dockEl?.classList.add('hidden');
  window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: null } }));
}
