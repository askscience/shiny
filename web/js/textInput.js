import { setSphereState } from './sphere.js';
import { currentConversationId, loadConversationMessages, stopActiveTurn } from './agent.js';

const compose = document.getElementById('compose-mode');
const field = document.getElementById('text-input-field');
const conversationEl = document.getElementById('compose-conversation');
const stopBtn = document.getElementById('compose-stop');
// The composer is the one and only text input, pinned inside the bottom of the
// conversation. It is not moved around: field, send and stop live here always.
const composer = document.getElementById('compose-composer');
const sendBtn = document.getElementById('compose-send');

let onSubmitCallback = null;
let isOpen = false;
let isSending = false;
let ignoreOutsideClick = false;
let assistantBubble = null;

function isInsideComposeInput(target) {
  if (!target) return false;
  if (target.closest('#text-input-field, #compose-composer')) return true;
  if (!document.body.classList.contains('compose-awaiting') && target.closest('#artifact-dock')) {
    return true;
  }
  return false;
}

function isInsideComposeContent(target) {
  return isInsideComposeInput(target) || !!target?.closest('.compose-conversation');
}

function setAwaiting(on) {
  document.body.classList.toggle('compose-awaiting', on);
}

/** The stop control only exists while there is an answer to stop. */
function setStopVisible(on) {
  stopBtn?.classList.toggle('visible', on);
  stopBtn?.setAttribute('aria-hidden', on ? 'false' : 'true');
  // The send button yields to stop while an answer runs.
  sendBtn?.classList.toggle('hidden', on);
}

function finishSending() {
  field.disabled = false;
  autoResizeField();
  isSending = false;
  setStopVisible(false);
  showComposeInput();
  setSphereState('idle');
  field.focus();
}

function showComposeInput() {
  setAwaiting(false);
  requestAnimationFrame(() => field?.focus());
}

function hideComposeInput() {
  field?.blur();
  setAwaiting(true);
}

/* ── Bubble conversation (text mode only) ─────────────────── */

function scrollConversation() {
  if (conversationEl) conversationEl.scrollTop = conversationEl.scrollHeight;
}

function escapeText(text) {
  const div = document.createElement('div');
  div.textContent = text || '';
  return div.innerHTML.replace(/\n/g, '<br>');
}

function renderMarkdown(text) {
  const m = window.marked;
  if (m) {
    try {
      if (typeof m.parse === 'function') return m.parse(text, { gfm: true, breaks: true });
      if (typeof m === 'function') return m(text);
    } catch (_) { /* fall through to escaped text */ }
  }
  return escapeText(text);
}

function renderBubble(role, text) {
  if (!conversationEl) return null;
  const bubble = document.createElement('div');
  bubble.className = `compose-bubble compose-bubble--${role}`;
  const bubbleText = document.createElement('div');
  bubbleText.className = 'compose-bubble-text';
  if (role === 'assistant') {
    bubbleText.innerHTML = renderMarkdown(text || '');
  } else {
    bubbleText.textContent = text || '';
  }
  bubble.appendChild(bubbleText);
  // Bubbles go above the composer so the input stays last.
  conversationEl.insertBefore(bubble, composer || null);
  scrollConversation();
  return bubble;
}

function clearConversation() {
  conversationEl?.querySelectorAll(':scope > .compose-bubble').forEach((b) => b.remove());
  assistantBubble = null;
}

function startAssistantBubble() {
  assistantBubble = renderBubble('assistant', '…');
  assistantBubble?.classList.add('is-thinking');
}

function streamAssistantBubble(text) {
  if (!assistantBubble) startAssistantBubble();
  if (!text) return; // keep the "…" indicator until real text arrives
  assistantBubble?.classList.remove('is-thinking');
  const t = assistantBubble?.querySelector('.compose-bubble-text');
  if (t) t.innerHTML = renderMarkdown(text);
  scrollConversation();
}

/** Load the current conversation thread (incl. audio turns, which are saved
 *  as text server-side) so text mode resumes where the user left off. */
async function loadConversation() {
  const id = currentConversationId();
  if (!id) return;
  const entries = await loadConversationMessages(id).catch(() => []);
  for (const m of entries) {
    renderBubble(m.role === 'user' ? 'user' : 'assistant', m.content);
  }
  scrollConversation();
}

export function initTextInput(onSubmit) {
  onSubmitCallback = onSubmit;

  field?.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      submitTextInput();
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      // Escape during an answer stops it; the panel stays open so the user can
      // immediately type something else.
      if (isSending) stopActiveTurn('escape');
      else closeTextInput();
    }
  });

  field?.addEventListener('input', () => {
    autoResizeField();
    if (isOpen && !isSending) setSphereState('idle');
  });

  field?.addEventListener('focus', () => {
    if (isOpen && !isSending) setSphereState('idle');
  });

  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape' || !isOpen) return;
    if (isSending) stopActiveTurn('escape');
    else closeTextInput();
  });

  stopBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    stopActiveTurn('button');
  });

  // The send button submits the same way Enter does.
  sendBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    void submitTextInput();
  });

  document.addEventListener('pointerdown', (e) => {
    if (!isOpen || ignoreOutsideClick || isSending) return;
    if (isInsideComposeContent(e.target)) return;
    closeTextInput();
  });
}

export async function openTextInput() {
  if (!compose || isOpen) return;
  isOpen = true;
  document.body.classList.add('compose-active');
  compose.classList.remove('hidden');
  compose.setAttribute('aria-hidden', 'false');
  clearConversation();
  field.value = '';
  field.disabled = false;
  setSphereState('idle');

  // Resume the thread before the user starts typing.
  await loadConversation();

  showComposeInput();
  autoResizeField();
  ignoreOutsideClick = true;
  requestAnimationFrame(() => {
    compose.classList.add('visible');
    field.focus();
    requestAnimationFrame(() => {
      ignoreOutsideClick = false;
    });
  });
}

export function closeTextInput(force = false) {
  if (!compose || !isOpen || (isSending && !force)) return;
  isOpen = false;
  isSending = false;
  setStopVisible(false);
  document.body.classList.remove('compose-active', 'compose-awaiting');
  compose.classList.remove('visible');
  compose.setAttribute('aria-hidden', 'true');
  field.blur();
  setSphereState('idle');
  setTimeout(() => {
    if (!isOpen) compose.classList.add('hidden');
  }, 300);
}

export function isComposeAwaiting() {
  return isSending;
}

export function isTextInputOpen() {
  return isOpen;
}

function autoResizeField() {
  if (!field) return;
  field.style.height = '38px';
  const next = Math.max(38, Math.min(field.scrollHeight, 80));
  field.style.height = `${next}px`;
}

async function submitTextInput() {
  const text = field.value.trim();
  if (!text || isSending) return;

  isSending = true;
  assistantBubble = null;
  renderBubble('user', text);
  startAssistantBubble();
  hideComposeInput();
  setSphereState('processing');
  setStopVisible(true);
  scrollConversation();

  try {
    await onSubmitCallback?.(text, {
      onStream: (partial) => {
        streamAssistantBubble(partial);
      },
      onDone: () => {
        field.value = '';
        finishSending();
      },
      onError: (msg) => {
        streamAssistantBubble(msg);
        finishSending();
      },
      onStopped: () => {
        // Keep whatever arrived; the core has recorded the interruption, so
        // the thread stays coherent for the next question.
        if (assistantBubble) {
          assistantBubble.classList.remove('is-thinking');
          const t = assistantBubble.querySelector('.compose-bubble-text');
          if (t && !t.textContent.trim()) t.textContent = '';
          assistantBubble.classList.add('is-stopped');
        }
        finishSending();
      },
    });
  } catch (e) {
    streamAssistantBubble(e.message || 'Something went wrong');
    finishSending();
  }
}
