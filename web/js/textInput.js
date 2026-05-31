import { setSphereState } from './sphere.js';
import { getDockSummaries } from './artifactStore.js';

const compose = document.getElementById('compose-mode');
const field = document.getElementById('text-input-field');
const replyEl = document.getElementById('compose-reply');
const thinkingEl = document.getElementById('compose-thinking');
const dock = document.getElementById('artifact-dock');
const dockIcons = document.getElementById('artifact-dock-icons');
const dockInput = document.getElementById('compose-dock-input');

let onSubmitCallback = null;
let isOpen = false;
let isSending = false;
let ignoreOutsideClick = false;

function isInsideComposeInput(target) {
  if (!target) return false;
  if (target.closest('#text-input-field, #compose-dock-input')) return true;
  if (!document.body.classList.contains('compose-awaiting') && target.closest('#artifact-dock')) {
    return true;
  }
  return false;
}

function isInsideComposeContent(target) {
  return isInsideComposeInput(target) || !!target?.closest('.compose-stream');
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

export function openTextInput() {
  if (!compose || isOpen) return;
  isOpen = true;
  document.body.classList.add('compose-active');
  compose.classList.remove('hidden');
  compose.setAttribute('aria-hidden', 'false');
  clearReply();
  setThinking(false);
  field.value = '';
  field.disabled = false;
  showComposeInput();
  autoResizeField();
  setSphereState('idle');
  ignoreOutsideClick = true;
  requestAnimationFrame(() => {
    compose.classList.add('visible');
    field.focus();
    requestAnimationFrame(() => {
      ignoreOutsideClick = false;
    });
  });
}

export function closeTextInput() {
  if (!compose || !isOpen || isSending) return;
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

export function setComposeThinking(on) {
  setThinking(on);
  if (isOpen) setSphereState(on ? 'processing' : 'idle');
}

export function streamComposeReply(text) {
  if (replyEl) {
    replyEl.classList.remove('is-waiting');
    replyEl.textContent = text;
  }
  scrollStreamToEnd();
  if (isOpen && isSending) setSphereState('processing');
}

export function clearComposeReply() {
  clearReply();
}

function clearReply() {
  if (replyEl) {
    replyEl.textContent = '';
    replyEl.classList.remove('is-waiting');
  }
}

function setWaitingReply(on) {
  if (!replyEl) return;
  if (on) {
    replyEl.textContent = 'Thinking…';
    replyEl.classList.add('is-waiting');
  } else {
    replyEl.classList.remove('is-waiting');
  }
}

function setThinking(on) {
  thinkingEl?.classList.add('hidden');
  if (on) setWaitingReply(true);
}

function scrollStreamToEnd() {
  const stream = document.querySelector('.compose-stream');
  if (stream) stream.scrollTop = stream.scrollHeight;
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
  clearReply();
  setThinking(true);
  hideComposeInput();
  setSphereState('processing');
  scrollStreamToEnd();

  try {
    await onSubmitCallback?.(text, {
      onStream: (partial) => {
        setThinking(false);
        streamComposeReply(partial);
      },
      onDone: () => {
        setThinking(false);
        field.value = '';
        field.disabled = false;
        autoResizeField();
        isSending = false;
        showComposeInput();
        setSphereState('idle');
        field.focus();
      },
      onError: (msg) => {
        setThinking(false);
        streamComposeReply(msg);
        field.disabled = false;
        isSending = false;
        showComposeInput();
        setSphereState('idle');
        field.focus();
      },
    });
  } catch (e) {
    setThinking(false);
    streamComposeReply(e.message || 'Something went wrong');
    field.disabled = false;
    isSending = false;
    showComposeInput();
    setSphereState('idle');
    field.focus();
  }
}
