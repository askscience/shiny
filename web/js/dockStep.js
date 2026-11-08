/**
 * The steps of AI work in progress ("Thinking…", "Searching the web…").
 *
 * They used to be written as a small text bubble above the voice bar
 * (`#artifact-dock-step`). The voice bar's dot grid (voiceDots.js) now draws
 * each step and the text chat shows it in its own pending bubble
 * (textInput.js), so the bubble is no longer painted: this module only
 * broadcasts `agent:step`, with the running tool's `action` so the dots know
 * which tool is at work. The element stays in the markup, hidden, for the
 * saved-cards dock it lives in.
 */

/**
 * Announce a step. `action` is the tool the step belongs to when it is a
 * tool's own step (agent.js passes the SSE `action`).
 */
export function setDockStep(message, { action = '' } = {}) {
  const text = (message || '').trim();
  if (!text) return;
  window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: text, action: String(action || '') } }));
}

/** The work is over: the dots and the chat bubble drop their step. */
export function clearDockStep() {
  window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: null } }));
}
