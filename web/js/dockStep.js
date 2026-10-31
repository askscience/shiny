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
 * The voice bar's own glyph is a second home for the same step (Settings →
 * Appearance → "Tool status", rendered by voiceGlyph.js): the line as text, the
 * running tool's icon worn per tool with a short fade — or nothing at all, when the
 * line then stays in the bubble above.
 */

import { getImmersive } from './preferences.js';
import { paintGlyph, paintStateLabel } from './voiceGlyph.js';

const stepEl = document.getElementById('artifact-dock-step');
const dockEl = document.getElementById('artifact-dock');
/** The bar's glyph. A mirror of the dock's line, not a live region of its own —
 *  the dock keeps that job. */
const glyphEl = document.getElementById('voice-step');

/** The line and the tool it belongs to, so a mode switch can repaint. */
let lastText = '';
let lastAction = '';

function bubbleStillNeeded() {
  const icons = document.getElementById('artifact-dock-icons');
  if (icons && icons.childElementCount > 0) return true;   // saved cards
  return document.body.classList.contains('compose-active'); // live composer
}

/** The state mode: the assistant's state, decoded letter by letter on change. */
function paintBarState() {
  glyphEl.classList.remove('hidden');
  painted = null;
  paintStateLabel(glyphEl, document.documentElement.dataset.voice || 'idle');
}

/** What the glyph was last painted with, so unrelated repaints do nothing. */
let painted = null;

/**
 * Paint the bar's glyph from the current mode and the last step. The icon mode
 * always carries a mark — the app's own while nothing runs, the tool's while it
 * does — so it is only hidden in the modes that can be empty.
 */
function paintBarGlyph() {
  if (!glyphEl) return;
  const mode = getImmersive().orb_status;
  if (mode === 'state') { paintBarState(); return; }
  if (mode === 'off' || (mode === 'text' && !lastText)) {
    if (painted !== null) {
      glyphEl.classList.add('hidden');
      painted = null;
    }
    return;
  }
  glyphEl.classList.remove('hidden');
  if (painted === `${mode}:${lastAction}:${mode === 'text' ? lastText : ''}`) return;
  painted = `${mode}:${lastAction}:${mode === 'text' ? lastText : ''}`;
  paintGlyph(glyphEl, { mode, text: lastText, action: lastAction });
}

/**
 * Show a status line. `action` is the tool the line belongs to when it is a
 * tool's own step (agent.js passes the SSE `action`), so the icon mode knows
 * which mark to wear.
 */
export function setDockStep(message, { action = '' } = {}) {
  if (!stepEl) return;
  const text = (message || '').trim();
  if (!text) return;
  lastText = text;
  lastAction = String(action || '');
  stepEl.textContent = text;
  stepEl.classList.remove('hidden');
  paintBarGlyph();
  // Paint the bubble the line lives in, whatever artifacts.js last decided.
  dockEl?.classList.remove('hidden');
  window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: text } }));
}

export function clearDockStep() {
  if (!stepEl) return;
  lastText = '';
  lastAction = '';
  stepEl.textContent = '';
  stepEl.classList.add('hidden');
  // The icon mode keeps its resting mark: the line goes, the app's mark stays.
  paintBarGlyph();
  // With no line left, the bubble is only worth keeping for saved cards.
  if (!bubbleStillNeeded()) dockEl?.classList.add('hidden');
  window.dispatchEvent(new CustomEvent('agent:step', { detail: { message: null } }));
}

/* A mode switch (off ↔ text ↔ icons) repaints the glyph in place — the line
   itself is already the last step's. */
window.addEventListener('desktop:immersive', paintBarGlyph);
/* The state label follows the assistant (sphere.js) and the language. */
window.addEventListener('voice:state', () => { if (getImmersive().orb_status === 'state') paintBarGlyph(); });
window.addEventListener('locale:change', () => { if (getImmersive().orb_status === 'state') paintBarGlyph(); });

/* Boot: the icon mode carries its resting mark from the first frame, even
   though this module can load after the settings were published (modules load
   in graph order, not in the order the app boots). */
paintBarGlyph();
