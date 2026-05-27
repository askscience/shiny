const compose = document.getElementById('compose-mode');
const field = document.getElementById('text-input-field');
const closeBtn = document.getElementById('compose-close');
const replyEl = document.getElementById('compose-reply');
const thinkingEl = document.getElementById('compose-thinking');

let onSubmitCallback = null;
let isOpen = false;
let isSending = false;

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

  field?.addEventListener('input', autoResizeField);

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && isOpen && !isSending) closeTextInput();
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
  autoResizeField();
  requestAnimationFrame(() => {
    compose.classList.add('visible');
    field.focus();
  });
}

export function closeTextInput() {
  if (!compose || !isOpen) return;
  isOpen = false;
  isSending = false;
  document.body.classList.remove('compose-active');
  compose.classList.remove('visible');
  compose.setAttribute('aria-hidden', 'true');
  field.blur();
  setTimeout(() => {
    if (!isOpen) compose.classList.add('hidden');
  }, 300);
}

export function isTextInputOpen() {
  return isOpen;
}

export function setComposeThinking(on) {
  setThinking(on);
}

export function streamComposeReply(text) {
  if (replyEl) replyEl.textContent = text;
  scrollStreamToEnd();
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
  field.style.height = `${Math.min(field.scrollHeight, window.innerHeight * 0.3)}px`;
}

async function submitTextInput() {
  const text = field.value.trim();
  if (!text || isSending) return;

  isSending = true;
  field.disabled = true;
  clearReply();
  setThinking(true);

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
        field.focus();
        isSending = false;
      },
      onError: (msg) => {
        setThinking(false);
        streamComposeReply(msg);
        field.disabled = false;
        isSending = false;
      },
    });
  } catch (e) {
    setThinking(false);
    streamComposeReply(e.message || 'Something went wrong');
    field.disabled = false;
    isSending = false;
  }
}
