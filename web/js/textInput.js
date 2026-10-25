import { controlButton } from '../ui/components/button.js';
import { setSphereState } from './sphere.js';
import { currentConversationId, loadConversationMessages, stopActiveTurn, respondToApproval } from './agent.js';
import { sanitizeHtml } from './sanitize.js';
import { ensureMarked } from './vendorLoader.js';
import { icon, button } from '../ui/index.js';
import { t } from './i18n.js';

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
// Media URLs already shown in this thread (live tool outcome or a persisted
// row), so the same generated image/track is never rendered twice.
const renderedMedia = new Set();

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
  expireApprovalCards();
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
 *  it stays out of the way until the user opens it. The arrow beside the label
 *  turns down as the trace opens (see .compose-cot-arrow). */
function makeCot(initial = '') {
  const details = document.createElement('details');
  details.className = 'compose-cot';
  const summary = document.createElement('summary');
  summary.className = 'compose-cot-summary';
  summary.append(
    icon('ui/chevron-right', { size: 13, className: 'compose-cot-arrow' }),
    document.createTextNode(t('chat.thinking', null, 'Thinking')),
  );
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

/* ── Terminal command approval ───────────────────────────────
 * The assistant's shell is gated per command. The live card asks; the same
 * record is stored in the conversation, so reopening the chat still shows the
 * command and its outcome as a green check or a red cross.
 * ────────────────────────────────────────────────────────── */

const approvalCards = new Set();

function makeApprovalStatus(tone, label) {
  const status = document.createElement('span');
  status.className = `compose-approval-status is-${tone}`;
  status.title = label;
  status.setAttribute('aria-label', label);
  status.append(icon(tone === 'allowed' ? 'ui/check' : 'ui/close', { size: 10 }));
  return status;
}

function settleApprovalCard(card, tone, label) {
  if (!card || card.classList.contains('is-resolved')) return;
  card.classList.add('is-resolved', `is-${tone}`);
  card.querySelector('.compose-approval-actions')?.remove();
  card.appendChild(makeApprovalStatus(tone, label));
  approvalCards.delete(card);
}

function makeApprovalCard(approval) {
  const card = document.createElement('div');
  card.className = 'compose-approval';
  card.setAttribute('role', 'group');
  card.setAttribute('aria-label', t('chat.approval.title', null, 'Run this command?'));

  const command = document.createElement('code');
  command.className = 'compose-approval-command';
  command.textContent = approval.command || '';

  const actions = document.createElement('div');
  actions.className = 'compose-approval-actions';

  const send = async (decision) => {
    const buttons = [...actions.querySelectorAll('button')];
    buttons.forEach((b) => { b.disabled = true; });
    try {
      const res = await respondToApproval({
        turnId: approval.turnId,
        requestId: approval.requestId,
        decision,
      });
      if (!res?.success) throw new Error(t('chat.approval.failed', null, 'Could not send your choice'));
      if (decision === 'allow') {
        settleApprovalCard(card, 'allowed', t('chat.approval.allowed', null, 'Allowed'));
      } else if (decision === 'allow_chat') {
        settleApprovalCard(card, 'allowed', t('chat.approval.allowedChat', null, 'Allowed for this chat'));
      } else {
        settleApprovalCard(card, 'denied', t('chat.approval.denied', null, 'Denied'));
      }
    } catch (e) {
      buttons.forEach((b) => { b.disabled = false; });
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: {
          message: e?.message || t('chat.approval.failed', null, 'Could not send your choice'),
          type: 'error',
        },
      }));
    }
  };

  actions.append(
    button({
      label: t('chat.approval.deny', null, 'Deny'),
      variant: 'ghost',
      size: 'sm',
      onClick: () => void send('deny'),
    }),
    button({
      label: t('chat.approval.allowChat', null, 'Allow for this chat'),
      variant: 'quiet',
      size: 'sm',
      onClick: () => void send('allow_chat'),
    }),
    button({
      label: t('chat.approval.allow', null, 'Allow'),
      variant: 'primary',
      size: 'sm',
      onClick: () => void send('allow'),
    }),
  );

  card.append(command, actions);
  // The answer bubble is already reserved when the prompt arrives: put the
  // card before it, matching the stored order (user, command, answer).
  const anchor = assistantBubble?.parentElement === conversationEl
    ? assistantBubble
    : (composer || null);
  conversationEl?.insertBefore(card, anchor);
  approvalCards.add(card);
  scrollConversation();
  return card;
}

/** A command from the stored conversation: same block, no buttons, icon only. */
function renderApprovalRecord(raw) {
  let record;
  try { record = JSON.parse(raw); } catch { return; }
  if (!record?.command) return;

  const allowed = record.decision === 'allow';
  const tone = allowed ? 'allowed' : 'denied';
  const label = t(allowed ? 'chat.approval.allowed' : 'chat.approval.denied', null,
    allowed ? 'Allowed' : 'Denied');

  const card = document.createElement('div');
  card.className = `compose-approval is-resolved is-${tone}`;
  card.setAttribute('role', 'group');
  card.setAttribute('aria-label', t('chat.approval.title', null, 'Run this command?'));

  const command = document.createElement('code');
  command.className = 'compose-approval-command';
  command.textContent = record.command;

  card.append(command, makeApprovalStatus(tone, label));
  conversationEl?.insertBefore(card, composer || null);
}

/** Insert one media block once per URL, in thread order. */
function appendMediaBlock(block, url) {
  if (!conversationEl || !block) return;
  if (url && renderedMedia.has(url)) return;
  if (url) renderedMedia.add(url);
  conversationEl.insertBefore(block, composer || null);
  scrollConversation();
}

/** A generated image/track stored in the conversation. */
function renderMediaRecord(raw) {
  let media;
  try { media = JSON.parse(raw); } catch { return; }
  if (media?.kind === 'image' && media.url) {
    appendMediaBlock(makeImageBlock(media.url, media.file || '', ''), media.url);
  } else if (media?.kind === 'song' && media.url) {
    appendMediaBlock(makeSongBlock(media.url, media.title || ''), media.url);
  }
}

/** Render the images/tracks a finished turn produced, from the tool payloads
 *  (the reply's markdown copy is stripped server-side). */
export function renderAgentMedia(actions) {
  for (const action of actions || []) {
    const data = action?.data;
    if (action?.result !== 'ok' || !data?.url) continue;
    if (action.action === 'generate_image') {
      clearImagePlaceholder();
      appendMediaBlock(makeImageBlock(data.url, data.file || '', ''), data.url);
    } else if (action.action === 'generate_song') {
      clearSongPlaceholder();
      appendMediaBlock(makeSongBlock(data.url, data.title || ''), data.url);
    }
  }
}

/** The turn ended (done, stopped, closed) before the user chose: the pending
 *  cards can no longer be answered, so make that visible instead of leaving
 *  live-looking buttons behind. */
function expireApprovalCards() {
  for (const card of [...approvalCards]) {
    settleApprovalCard(card, 'expired', t('chat.approval.expired', null, 'Turn ended'));
  }
}

function clearConversation() {
  conversationEl
    ?.querySelectorAll(':scope > .compose-bubble, :scope > .compose-image-block, :scope > .compose-song-block, :scope > .compose-approval')
    .forEach((b) => b.remove());
  approvalCards.clear();
  renderedMedia.clear();
  assistantBubble = null;
  imagePlaceholder = null;
  songPlaceholder = null;
}

function startAssistantBubble() {
  assistantBubble = makePendingBubble(t('chat.thinkingPending', null, 'Thinking…'))?.bubble || null;
}

/** Reserve the spot where a generated image will land. The assistant's pending
 *  bubble is reused when it still holds the wheel, so generation never shows
 *  two spinners at once. */
function reuseAssistantPending(labelText) {
  if (!assistantBubble) return null;
  const row = assistantBubble.querySelector('.compose-pending-row');
  if (!row) return null;
  const label = row.querySelector('.compose-pending-label');
  if (label) label.textContent = labelText;
  return assistantBubble;
}

export function showImagePlaceholder() {
  if (imagePlaceholder) return;
  const label = t('chat.imagePending', null,
    'Painting your image — this can take a few minutes…');
  imagePlaceholder = reuseAssistantPending(label)
    || makePendingBubble(label)?.bubble
    || null;
}

export function clearImagePlaceholder() {
  // The reused assistant bubble belongs to the answer: never remove it here.
  if (imagePlaceholder && imagePlaceholder !== assistantBubble) imagePlaceholder.remove();
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
  img.alt = alt || t('chat.generatedImage', null, 'Generated image');
  img.loading = 'lazy';
  img.decoding = 'async';

  const actions = document.createElement('div');
  actions.className = 'compose-image-actions';

  const open = controlButton();
  open.type = 'button';
  open.className = 'compose-image-action';
  open.title = t('chat.openInImageEditor', null, 'Open in the Image editor');
  open.setAttribute('aria-label', t('chat.openInImageEditor', null, 'Open in the Image editor'));
  open.appendChild(icon('ui/magic-wand', { size: 16 }));
  open.addEventListener('click', () => {
    import('./files.js')
      .then((m) => m.openWithPlugin({ plugin: 'image', path: `Pictures/Generated/${name}`, name }))
      .catch(() => {});
  });

  const save = controlButton();
  save.type = 'button';
  save.className = 'compose-image-action';
  save.title = t('chat.saveToPictures', null, 'Save to Pictures');
  save.setAttribute('aria-label', t('chat.saveToPictures', null, 'Save to Pictures'));
  save.appendChild(icon('ui/save', { size: 16 }));
  save.addEventListener('click', async () => {
    save.disabled = true;
    try {
      const res = await fetch(src);
      if (!res.ok) throw new Error(t('chat.imageLoadFailed', null, 'Could not load the image'));
      const blob = await res.blob();
      const m = await import('./files.js');
      await m.saveOrDownload(blob, { name, dir: 'Pictures', app: 'Image' });
    } catch (e) {
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: {
          message: e?.message || t('chat.imageSaveFailed', null, 'Could not save the image'),
          type: 'error',
        },
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
    if (blocks.has(src) || renderedMedia.has(src)) continue;
    const block = makeImageBlock(src, src.slice(IMAGE_SRC_PREFIX.length), alt);
    bubble.after(block);
    blocks.set(src, block);
    renderedMedia.add(src);
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

/** Reserve the spot where the generated song will land, reusing the pending
 *  answer bubble so only one wheel is ever visible. */
export function showSongPlaceholder() {
  if (songPlaceholder) return;
  const label = t('chat.songPending', null,
    'Composing your song — this can take several minutes…');
  songPlaceholder = reuseAssistantPending(label)
    || makePendingBubble(label)?.bubble
    || null;
}

export function clearSongPlaceholder() {
  if (songPlaceholder && songPlaceholder !== assistantBubble) songPlaceholder.remove();
  songPlaceholder = null;
}

function setSongPlaying(entry, playing) {
  entry.button.replaceChildren(icon(playing ? 'ui/pause' : 'ui/play', { size: 16 }));
  entry.button.setAttribute('aria-label', playing ? t('chat.pause', null, 'Pause') : t('chat.play', null, 'Play'));
  entry.button.title = playing ? t('chat.pause', null, 'Pause') : t('chat.play', null, 'Play');
}

function makeSongBlock(url, title) {
  const block = document.createElement('div');
  block.className = 'compose-song-block';

  const button = controlButton();
  button.type = 'button';
  button.className = 'compose-song-play';
  button.setAttribute('aria-label', t('chat.play', null, 'Play'));
  button.title = t('chat.play', null, 'Play');

  const main = document.createElement('div');
  main.className = 'compose-song-main';
  const name = document.createElement('span');
  name.className = 'compose-song-title';
  name.textContent = title || t('chat.song', null, 'Song');
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
          if (!res.ok) throw new Error(t('chat.songLoadFailed', null, 'Could not load the track'));
          blobUrl = URL.createObjectURL(await res.blob());
        } catch (err) {
          button.disabled = false;
          window.dispatchEvent(new CustomEvent('app:toast', {
            detail: {
              message: err?.message || t('chat.songLoadFailed', null, 'Could not load the track'),
              type: 'error',
            },
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
    if (blocks.has(url) || renderedMedia.has(url)) continue;
    const block = makeSongBlock(url, title);
    bubble.after(block);
    blocks.set(url, block);
    renderedMedia.add(url);
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
    if (m.role === 'approval') renderApprovalRecord(m.content);
    else if (m.role === 'media') renderMediaRecord(m.content);
    else renderBubble(m.role === 'user' ? 'user' : 'assistant', m.content, m.thinking || '');
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

  // Markdown is only needed once the chat opens; keep it out of boot.
  try {
    await ensureMarked();
  } catch (_) {
    // renderMarkdown falls back to escaped plain text.
  }

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
      onApproval: (approval) => makeApprovalCard(approval),
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
    streamAssistantBubble(e.message || t('chat.somethingWentWrong', null, 'Something went wrong'));
    finishSending();
  }
}
