/**
 * address-suggest.js — the Browser's address bar completes what you type.
 *
 * Two helps, both from `/api/browser/suggest` (bookmarks, history and past
 * searches; see `src/suggest.rs`):
 *
 * - **Inline completion.** Typing `you` fills in `youtube.com` with the added
 *   tail selected, so Enter goes there and the next letter simply replaces it.
 *   Only the tail is ever added: the user's own letters, scheme and `www.`
 *   included, are never rewritten. Deleting never completes (Backspace must
 *   be able to remove the suggestion).
 * - **A short list** under the bar. Arrow keys move through it, Enter or a
 *   click opens the row (a past search is searched again), Escape closes it
 *   and drops the inline tail.
 *
 * The list is a popover of the window, so the window lists it with the other
 * popovers for the native page's mask (`panel`).
 */
import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';
import { controlButton, icon } from '../../ui/index.js';

const ICONS = { bookmark: 'ui/star', history: 'ui/history', search: 'ui/search' };

/** The address as typed without scheme and `www.`, lowercased. */
export function addressBody(text) {
  return String(text || '')
    .trim()
    .replace(/^https?:\/\//i, '')
    .replace(/^www\./i, '')
    .toLowerCase();
}

/**
 * The value and selection for an inline completion of `typed`, or null when
 * `completion` does not extend it.
 */
export function inlineCompletion(typed, completion) {
  if (!typed || !completion || /\s/.test(typed)) return null;
  const body = addressBody(typed);
  const target = String(completion).toLowerCase();
  if (!body || !target.startsWith(body) || target.length <= body.length) return null;
  const value = typed + target.slice(body.length);
  return { value, start: typed.length, end: value.length };
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/**
 * Wire suggestions onto the address `input`.
 *
 * `onPick(value)` navigates (an address or a search phrase); `onChange()` tells
 * the window the popover opened or closed (its mask follows); `place(panel)`
 * positions the list under the bar.
 */
export function createAddressSuggest({
  input,
  onPick,
  onChange = () => {},
  place = () => {},
  fetchSuggestions = defaultFetch,
}) {
  const panel = el('div', 'browser-popover hud-menu browser-suggest hidden');
  panel.setAttribute('role', 'listbox');
  panel.setAttribute('aria-label', t('plugin.browser.suggest.title', null, 'Suggestions'));
  input.setAttribute('aria-autocomplete', 'both');
  input.setAttribute('role', 'combobox');
  input.setAttribute('aria-expanded', 'false');

  let items = [];
  let active = -1;
  let typed = '';
  let seq = 0;
  // The window may hide the list with its other popovers: the class is the truth.
  const isOpen = () => !panel.classList.contains('hidden');

  function setOpen(on) {
    if (isOpen() === on) return;
    panel.classList.toggle('hidden', !on);
    input.setAttribute('aria-expanded', on ? 'true' : 'false');
    if (on) place(panel);
    onChange();
  }

  function close() {
    seq += 1; // a reply still in flight must not reopen it
    items = [];
    active = -1;
    setOpen(false);
  }

  function highlight(index) {
    active = index;
    panel.querySelectorAll('.browser-suggest-row').forEach((row, i) => {
      row.classList.toggle('is-active', i === index);
      row.setAttribute('aria-selected', i === index ? 'true' : 'false');
    });
  }

  function row(item, index) {
    const node = controlButton();
    node.type = 'button';
    node.className = 'browser-suggest-row';
    node.setAttribute('role', 'option');
    const text = el('span', 'browser-suggest-text');
    const main = item.kind === 'search' ? item.value : (item.title || item.display || item.value);
    text.append(el('span', 'browser-suggest-main', main));
    if (item.kind !== 'search' && item.title && item.display) {
      text.append(el('span', 'browser-suggest-sub', item.display));
    }
    node.append(icon(ICONS[item.kind] || 'ui/link', { size: 14 }), text);
    // Keep the focus in the bar: a blur would close the list mid-click.
    node.addEventListener('pointerdown', (e) => e.preventDefault());
    node.addEventListener('mousedown', (e) => e.preventDefault());
    node.addEventListener('click', () => pick(index));
    node.addEventListener('pointerenter', () => highlight(index));
    return node;
  }

  function render() {
    panel.replaceChildren(...items.map(row));
    highlight(-1);
    setOpen(items.length > 0);
  }

  function pick(index) {
    const item = items[index];
    if (!item) return;
    close();
    input.blur();
    onPick(item.value);
  }

  async function update(allowInline) {
    const mine = ++seq;
    typed = input.value;
    if (!typed.trim()) {
      close();
      return;
    }
    let result;
    try {
      result = await fetchSuggestions(typed);
    } catch (_) {
      return;
    }
    // Typed on, picked, or left the bar while the reply was in flight.
    if (mine !== seq || input.value !== typed || document.activeElement !== input) return;
    items = Array.isArray(result?.items) ? result.items : [];
    if (allowInline) {
      const inline = inlineCompletion(typed, result?.completion);
      const caretAtEnd = input.selectionStart === typed.length && input.selectionEnd === typed.length;
      if (inline && caretAtEnd) {
        input.value = inline.value;
        input.setSelectionRange(inline.start, inline.end);
      }
    }
    render();
  }

  function onInput(event) {
    const kind = String(event?.inputType || '');
    // Deleting must be able to remove what we added; pasting is taken as is.
    void update(!kind.startsWith('delete') && kind !== 'insertFromPaste');
  }

  function onKeydown(event) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      if (!isOpen() || !items.length) return;
      event.preventDefault();
      const step = event.key === 'ArrowDown' ? 1 : -1;
      const next = active + step;
      highlight(next < -1 ? items.length - 1 : next >= items.length ? -1 : next);
    } else if (event.key === 'Enter') {
      if (isOpen() && active >= 0) {
        event.preventDefault();
        pick(active);
      } else {
        close();
      }
    } else if (event.key === 'Escape') {
      if (!isOpen() && input.value === typed) return;
      event.preventDefault();
      event.stopPropagation();
      // Drop the inline tail, keep what the user typed.
      if (input.selectionEnd > input.selectionStart && input.value.startsWith(typed)) {
        input.value = typed;
      }
      close();
    }
  }

  input.addEventListener('input', onInput);
  input.addEventListener('keydown', onKeydown);
  input.addEventListener('blur', () => close());

  return {
    panel,
    close,
    isOpen,
  };
}

async function defaultFetch(typed) {
  const res = await apiFetch(`/api/browser/suggest?q=${encodeURIComponent(typed)}`);
  return res?.data;
}
