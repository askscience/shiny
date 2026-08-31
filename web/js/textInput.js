import { setSphereState } from './sphere.js';
import { getDockSummaries } from './artifactStore.js';
import { currentConversationId, loadConversationMessages } from './agent.js';

const compose = document.getElementById('compose-mode');
const field = document.getElementById('text-input-field');
const conversationEl = document.getElementById('compose-conversation');
const dock = document.getElementById('artifact-dock');
const dockIcons = document.getElementById('artifact-dock-icons');
const dockInput = document.getElementById('compose-dock-input');

let onSubmitCallback = null;
let isOpen = false;
let isSending = false;
let ignoreOutsideClick = false;
let assistantBubble = null;

function isInsideComposeInput(target) {
  if (!target) return false;
  if (target.closest('#text-input-field, #compose-dock-input')) return true;
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

function showComposeInput() {
  dock?.classList.remove('hidden');
  dockIcons?.classList.add('hidden');
  dockInput?.classList.remove('hidden');
  dockInput?.setAttribute('aria-hidden', 'false');
  setAwaiting(false);
  requestAnimationFrame(() => field?.focus());
}

function hideComposeInput() {
  dockInput?.classList.add('hidden');
  dockInput?.setAttribute('aria-hidden', 'true');
  field?.blur();
  setAwaiting(true);
  showDockIcons();
}

function showDockIcons() {
  dockIcons?.classList.remove('hidden');
  dock?.classList.remove('hidden');
  window.dispatchEvent(new CustomEvent('artifact:dock', { detail: getDockSummaries() }));
}

function restoreDock() {
  dockInput?.classList.add('hidden');
  dockInput?.setAttribute('aria-hidden', 'true');
  showDockIcons();
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
  conversationEl.appendChild(bubble);
  scrollConversation();
  return bubble;
}

function clearConversation() {
  if (conversationEl) conversationEl.textContent = '';
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
      closeTextInput();
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
    if (e.key === 'Escape' && isOpen && !isSending) closeTextInput();
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
  document.body.classList.remove('compose-active', 'compose-awaiting');
  compose.classList.remove('visible');
  compose.setAttribute('aria-hidden', 'true');
  field.blur();
  restoreDock();
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
  scrollConversation();

  try {
    await onSubmitCallback?.(text, {
      onStream: (partial) => {
        streamAssistantBubble(partial);
      },
      onDone: () => {
        field.value = '';
        field.disabled = false;
        autoResizeField();
        isSending = false;
        showComposeInput();
        setSphereState('idle');
        field.focus();
      },
      onError: (msg) => {
        streamAssistantBubble(msg);
        field.disabled = false;
        isSending = false;
        showComposeInput();
        setSphereState('idle');
        field.focus();
      },
    });
  } catch (e) {
    streamAssistantBubble(e.message || 'Something went wrong');
    field.disabled = false;
    isSending = false;
    showComposeInput();
    setSphereState('idle');
    field.focus();
  }
}
