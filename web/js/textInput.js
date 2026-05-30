import { setSphereState } from './sphere.js';

const compose = document.getElementById('compose-mode');
const field = document.getElementById('text-input-field');
const closeBtn = document.getElementById('compose-close');
const replyEl = document.getElementById('compose-reply');
const thinkingEl = document.getElementById('compose-thinking');
const inputWrap = document.querySelector('.compose-input-wrap');

let onSubmitCallback = null;
let isOpen = false;
let isSending = false;

function updateComposeLayout() {
  if (!inputWrap) return;
  const h = inputWrap.classList.contains('hidden') ? 0 : inputWrap.offsetHeight;
  document.documentElement.style.setProperty('--compose-input-height', `${h}px`);
}

function setAwaiting(on) {
  document.body.classList.toggle('compose-awaiting', on);
  requestAnimationFrame(updateComposeLayout);
}

function showComposeInput() {
  inputWrap?.classList.remove('hidden');
  setAwaiting(false);
  updateComposeLayout();
}

function hideComposeInput() {
  inputWrap?.classList.add('hidden');
  field?.blur();
  setAwaiting(true);
  updateComposeLayout();
}

export function initTextInput(onSubmit) {
  onSubmitCallback = onSubmit;

  closeBtn?.addEventListener('click', closeTextInput);

  field?.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
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

  window.addEventListener('resize', updateComposeLayout);
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
  requestAnimationFrame(() => {
    compose.classList.add('visible');
    updateComposeLayout();
    field.focus();
  });
}

export function closeTextInput() {
  if (!compose || !isOpen) return;
  isOpen = false;
  isSending = false;
  document.body.classList.remove('compose-active', 'compose-awaiting');
  compose.classList.remove('visible');
  compose.setAttribute('aria-hidden', 'true');
  field.blur();
  showComposeInput();
  document.documentElement.style.removeProperty('--compose-input-height');
  setSphereState('idle');
  setTimeout(() => {
    if (!isOpen) compose.classList.add('hidden');
  }, 300);
}

export function isTextInputOpen() {
  return isOpen;
}

export function setComposeThinking(on) {
  setThinking(on);
  if (isOpen) setSphereState(on ? 'processing' : 'idle');
}

export function streamComposeReply(text) {
  if (replyEl) replyEl.textContent = text;
  scrollStreamToEnd();
  if (isOpen && isSending) setSphereState('processing');
}

export function clearComposeReply() {
  clearReply();
}

function clearReply() {
  if (replyEl) replyEl.textContent = '';
}

function setThinking(on) {
  thinkingEl?.classList.toggle('hidden', !on);
}

function scrollStreamToEnd() {
  const stream = document.querySelector('.compose-stream');
  if (stream) stream.scrollTop = stream.scrollHeight;
}

function autoResizeField() {
  if (!field) return;
  field.style.height = 'auto';
  field.style.height = `${Math.min(field.scrollHeight, window.innerHeight * 0.22)}px`;
  updateComposeLayout();
}

async function submitTextInput() {
  const text = field.value.trim();
  if (!text || isSending) return;

  isSending = true;
  clearReply();
  setThinking(true);
  hideComposeInput();
  setSphereState('processing');

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
