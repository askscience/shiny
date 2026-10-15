import { setSphereState } from './sphere.js';
import { currentConversationId, loadConversationMessages, stopActiveTurn } from './agent.js';
import { sanitizeHtml } from './sanitize.js';
import { icon } from '../ui/index.js';

const compose = document.getElementById('compose-mode');
const field = document.getElementById('text-input-field');
const conversationEl = document.getElementById('compose-conversation');
const stopBtn = document.getElementById('compose-stop');
// The composer is the one and only text input, pinned inside the bottom of the
// conversation. It is not moved around: field, send and stop live here always.
const composer = document.getElementById('compose-composer');
const sendBtn = document.getElementById('compose-send');
const closeBtn = document.getElementById('compose-close-btn');

let onSubmitCallback = null;
let isOpen = false;
let isSending = false;
let ignoreOutsideClick = false;
let assistantBubble = null;
let imagePlaceholder = null;
let songPlaceholder = null;

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

function renderBubble(role, text, thinking = '') {
  if (!conversationEl) return null;
  const bubble = document.createElement('div');
  bubble.className = `compose-bubble compose-bubble--${role}`;
  const bubbleText = document.createElement('div');
  bubbleText.className = 'compose-bubble-text';
  if (role === 'assistant') {
    // Model output can carry attacker-influenced HTML (pages the agent read);
    // sanitize before it reaches innerHTML. See sanitize.js.
    bubbleText.innerHTML = sanitizeHtml(renderMarkdown(text || ''));
  } else {
    bubbleText.textContent = text || '';
  }
  bubble.appendChild(bubbleText);
  if (role === 'assistant') {
    // A saved reasoning trace folds above the answer, closed by default.
    if (thinking) bubble.insertBefore(makeCot(thinking), bubbleText);
    syncImageBlocks(bubble);
    syncSongBlocks(bubble);
  }
  // Bubbles go above the composer so the input stays last.
  conversationEl.insertBefore(bubble, composer || null);
  scrollConversation();
  return bubble;
}

/* ── Waiting bubbles: the wheel + one live line ─────────────────────────────
 * Every "the assistant is working" bubble is the same widget: the spinner the
 * image/song placeholders use, next to one line of text. A pending answer
 * starts with "Thinking…" and the line then swaps to the agent's live step
 * (the same labels the voice bar shows in its pill) via the `agent:step`
 * event. The image and song placeholders are the same widget with a fixed
 * label — never a second, drifted copy of the markup.
 * ────────────────────────────────────────────────────────────────────────── */

function makePendingBubble(labelText) {
  if (!conversationEl) return null;
  const bubble = document.createElement('div');
  bubble.className = 'compose-bubble compose-bubble--assistant compose-pending';
  const inner = document.createElement('div');
  inner.className = 'compose-bubble-text';
  const row = document.createElement('div');
  row.className = 'compose-pending-row';
  const spinner = document.createElement('span');
  spinner.className = 'compose-spinner';
  const label = document.createElement('span');
  label.className = 'compose-pending-label';
  label.textContent = labelText || '';
  row.append(spinner, label);
  inner.appendChild(row);
  bubble.appendChild(inner);
  conversationEl.insertBefore(bubble, composer || null);
  scrollConversation();
  return { bubble, label };
}

/** The wait is over: the bubble becomes a normal answer bubble. */
function clearPendingRow(bubble) {
  if (!bubble) return;
  bubble.querySelector('.compose-pending-row')?.remove();
  bubble.classList.remove('compose-pending');
}

/** The model's reasoning trace: a "Thinking" disclosure, closed by default so
 *  it stays out of the way until the user opens it. */
function makeCot(initial = '') {
  const details = document.createElement('details');
  details.className = 'compose-cot';
  const summary = document.createElement('summary');
  summary.className = 'compose-cot-summary';
  summary.append(icon('ui/brain', { size: 13 }), document.createTextNode('Thinking'));
  const body = document.createElement('div');
  body.className = 'compose-cot-body';
  body.textContent = initial;
  details.append(summary, body);
  return details;
}

/** Append one block of the reasoning trace, creating the disclosure on the
 *  first block. Plain textNode/`textContent` — model output never parses. */
function showThinking(text) {
  if (!text) return;
  if (!assistantBubble) startAssistantBubble();
  if (!assistantBubble) return;
  let details = assistantBubble.querySelector('.compose-cot');
  if (!details) {
    details = makeCot();
    assistantBubble.insertBefore(details, assistantBubble.firstChild);
  }
  const body = details.querySelector('.compose-cot-body');
  body.textContent = body.textContent ? `${body.textContent}\n\n${text}` : text;
  scrollConversation();
}

function clearConversation() {
  conversationEl
    ?.querySelectorAll(':scope > .compose-bubble, :scope > .compose-image-block, :scope > .compose-song-block')
    .forEach((b) => b.remove());
  assistantBubble = null;
  imagePlaceholder = null;
  songPlaceholder = null;
}

function startAssistantBubble() {
  assistantBubble = makePendingBubble('Thinking…')?.bubble || null;
}

/** Reserve the spot where a generated image will land (spinner + label). */
export function showImagePlaceholder() {
  if (imagePlaceholder) return;
  imagePlaceholder =
    makePendingBubble('Painting your image — this can take a few minutes…')?.bubble || null;
}

export function clearImagePlaceholder() {
  imagePlaceholder?.remove();
  imagePlaceholder = null;
}

/* ── Generated images: their own block, not part of the answer ─────────────
 * The assistant replies with a markdown image; the bubble keeps the prose and
 * the picture is lifted out into a standalone block (no panel, no border,
 * rounded corners) with hover actions: Save (Files → Pictures) and Open (the
 * Image editor), the same flows Files double-click uses.
 * ────────────────────────────────────────────────────────────────────────── */

const IMAGE_SRC_PREFIX = '/api/imagegen/images/';
const imageBlocks = new WeakMap(); // bubble -> Map(src -> block)

function makeImageBlock(src, name, alt) {
  const block = document.createElement('div');
  block.className = 'compose-image-block';

  const img = document.createElement('img');
  img.src = src;
  img.alt = alt || 'Generated image';
  img.loading = 'lazy';
  img.decoding = 'async';

  const actions = document.createElement('div');
  actions.className = 'compose-image-actions';

  const open = document.createElement('button');
  open.type = 'button';
  open.className = 'compose-image-action';
  open.title = 'Open in the Image editor';
  open.setAttribute('aria-label', 'Open in the Image editor');
  open.appendChild(icon('ui/magic-wand', { size: 16 }));
  open.addEventListener('click', () => {
    import('./files.js')
      .then((m) => m.openWithPlugin({ plugin: 'image', path: `Pictures/Generated/${name}`, name }))
      .catch(() => {});
  });

  const save = document.createElement('button');
  save.type = 'button';
  save.className = 'compose-image-action';
  save.title = 'Save to Pictures';
  save.setAttribute('aria-label', 'Save to Pictures');
  save.appendChild(icon('ui/save', { size: 16 }));
  save.addEventListener('click', async () => {
    save.disabled = true;
    try {
      const res = await fetch(src);
      if (!res.ok) throw new Error('Could not load the image');
      const blob = await res.blob();
      const m = await import('./files.js');
      await m.saveOrDownload(blob, { name, dir: 'Pictures', app: 'Image' });
    } catch (e) {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: e?.message || 'Could not save the image', type: 'error' },
      }));
    } finally {
      save.disabled = false;
    }
  });

  actions.append(open, save);
  block.append(img, actions);
  return block;
}

/** Pull generated images out of an answer bubble into their own block. */
function syncImageBlocks(bubble) {
  const text = bubble?.querySelector('.compose-bubble-text');
  if (!text) return;
  for (const inline of text.querySelectorAll(`img[src^="${IMAGE_SRC_PREFIX}"]`)) {
    const src = inline.getAttribute('src');
    const alt = inline.getAttribute('alt') || '';
    inline.remove();
    let blocks = imageBlocks.get(bubble);
    if (!blocks) {
      blocks = new Map();
      imageBlocks.set(bubble, blocks);
    }
    if (blocks.has(src)) continue;
    const block = makeImageBlock(src, src.slice(IMAGE_SRC_PREFIX.length), alt);
    bubble.after(block);
    blocks.set(src, block);
  }
  // A reply that is only the picture keeps no empty bubble behind it.
  bubble.classList.toggle('is-image-only', text.textContent.trim().length === 0);
}

/* ── Generated songs: their own block, not part of the answer ──────────────
 * The assistant replies with a markdown link; the bubble keeps the prose and
 * the track is lifted out into a standalone player (no panel, no border) —
 * a round play button, the title and a slim seek bar.
 * ────────────────────────────────────────────────────────────────────────── */

const SONG_URL_PREFIX = '/api/songgen/tracks/';
const songBlocks = new WeakMap(); // bubble -> Map(url -> block)
let currentSong = null; // { audio, button }

function fmtTime(secs) {
  if (!Number.isFinite(secs) || secs < 0) return '0:00';
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m}:${String(s).padStart(2, '0')}`;
}

/** Reserve the spot where the generated song will land (spinner + label). */
export function showSongPlaceholder() {
  if (songPlaceholder) return;
  songPlaceholder =
    makePendingBubble('Composing your song — this can take several minutes…')?.bubble || null;
}

export function clearSongPlaceholder() {
  songPlaceholder?.remove();
  songPlaceholder = null;
}

function setSongPlaying(entry, playing) {
  entry.button.replaceChildren(icon(playing ? 'ui/pause' : 'ui/play', { size: 16 }));
  entry.button.setAttribute('aria-label', playing ? 'Pause' : 'Play');
  entry.button.title = playing ? 'Pause' : 'Play';
}

function makeSongBlock(url, title) {
  const block = document.createElement('div');
  block.className = 'compose-song-block';

  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'compose-song-play';
  button.setAttribute('aria-label', 'Play');
  button.title = 'Play';

  const main = document.createElement('div');
  main.className = 'compose-song-main';
  const name = document.createElement('span');
  name.className = 'compose-song-title';
  name.textContent = title || 'Song';
  const track = document.createElement('div');
  track.className = 'compose-song-track';
  const fill = document.createElement('div');
  fill.className = 'compose-song-fill';
  track.appendChild(fill);
  main.append(name, track);

  const time = document.createElement('span');
  time.className = 'compose-song-time';
  time.textContent = '0:00';

  // The source is fetched lazily — on the first press — and played from a
  // blob URL: the same path the spoken replies use, and the only one that has
  // never stalled this machine's webview. A plain `src=url` media element
  // makes the renderer open a streaming HTTP fetch the moment the block is
  // built (and again on every conversation re-render), which freezes the app
  // page for the length of the fetch.
  const audio = document.createElement('audio');
  audio.preload = 'none';
  audio.hidden = true;
  let blobUrl = null;

  const entry = { audio, button };
  setSongPlaying(entry, false);

  audio.addEventListener('loadedmetadata', () => {
    if (Number.isFinite(audio.duration)) time.textContent = fmtTime(audio.duration);
  });
  audio.addEventListener('timeupdate', () => {
    const d = audio.duration;
    if (Number.isFinite(d) && d > 0) {
      fill.style.width = `${(audio.currentTime / d) * 100}%`;
      time.textContent = `${fmtTime(audio.currentTime)} / ${fmtTime(d)}`;
    }
  });
  audio.addEventListener('ended', () => {
    fill.style.width = '0%';
    time.textContent = fmtTime(audio.duration);
    setSongPlaying(entry, false);
    if (currentSong?.audio === audio) currentSong = null;
  });

  button.addEventListener('click', async () => {
    if (audio.paused) {
      if (!blobUrl) {
        button.disabled = true;
        try {
          const res = await fetch(url);
          if (!res.ok) throw new Error('Could not load the track');
          blobUrl = URL.createObjectURL(await res.blob());
        } catch (err) {
          button.disabled = false;
          window.dispatchEvent(new CustomEvent('app:toast', {
            detail: { message: err?.message || 'Could not load the track', type: 'error' },
          }));
          return;
        }
        audio.src = blobUrl;
        button.disabled = false;
      }
      if (currentSong && currentSong.audio !== audio) {
        currentSong.audio.pause();
        setSongPlaying(currentSong, false);
      }
      currentSong = entry;
      void audio.play()
        .then(() => setSongPlaying(entry, true))
        .catch(() => setSongPlaying(entry, false));
    } else {
      audio.pause();
      setSongPlaying(entry, false);
    }
  });

  track.addEventListener('click', (event) => {
    const rect = track.getBoundingClientRect();
    if (!rect.width || !Number.isFinite(audio.duration) || audio.duration <= 0) return;
    const ratio = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
    audio.currentTime = ratio * audio.duration;
  });

  block.append(button, main, time, audio);
  return block;
}

/** Pull generated tracks out of an answer bubble into their own player. */
function syncSongBlocks(bubble) {
  const text = bubble?.querySelector('.compose-bubble-text');
  if (!text) return;
  for (const link of text.querySelectorAll(`a[href^="${SONG_URL_PREFIX}"]`)) {
    const url = link.getAttribute('href');
    const title = (link.textContent || '').trim();
    link.remove();
    let blocks = songBlocks.get(bubble);
    if (!blocks) {
      blocks = new Map();
      songBlocks.set(bubble, blocks);
    }
    if (blocks.has(url)) continue;
    const block = makeSongBlock(url, title);
    bubble.after(block);
    blocks.set(url, block);
  }
  bubble.classList.toggle('is-song-only', text.textContent.trim().length === 0);
}

function streamAssistantBubble(text) {
  if (!assistantBubble) startAssistantBubble();
  if (!text) return; // keep the wheel + step line until real text arrives
  clearPendingRow(assistantBubble);
  const t = assistantBubble?.querySelector('.compose-bubble-text');
  if (t) t.innerHTML = sanitizeHtml(renderMarkdown(text));
  if (assistantBubble) {
    syncImageBlocks(assistantBubble);
    syncSongBlocks(assistantBubble);
  }
  scrollConversation();
}

/** Load the current conversation thread (incl. audio turns, which are saved
 *  as text server-side) so text mode resumes where the user left off. */
async function loadConversation() {
  const id = currentConversationId();
  if (!id) return;
  const entries = await loadConversationMessages(id).catch(() => []);
  for (const m of entries) {
    renderBubble(m.role === 'user' ? 'user' : 'assistant', m.content, m.thinking || '');
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

  // The live step line next to the wheel: the same labels the voice bar shows
  // in its pill (dockStep re-broadcasts every step as `agent:step`).
  window.addEventListener('agent:step', (e) => {
    if (!isSending || !assistantBubble) return;
    const msg = e?.detail?.message;
    if (!msg) return;
    const label = assistantBubble.querySelector('.compose-pending-label');
    if (label && label.textContent !== msg) {
      label.textContent = msg;
      scrollConversation();
    }
  });

  stopBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    stopActiveTurn('button');
  });

  // Close is decisive: a running answer is stopped (the core records the
  // interruption) before the panel slides away.
  closeBtn?.addEventListener('click', (e) => {
    e.stopPropagation();
    closeChat();
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

/** Close the chat from anywhere (head button, voice-bar menu): a running
 *  answer is stopped first, so the panel never outlives a half-live turn. */
export function closeChat() {
  if (isSending) stopActiveTurn('close');
  closeTextInput(true);
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
      onThinking: (chunk) => {
        showThinking(chunk);
      },
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
          clearPendingRow(assistantBubble);
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
