// notebook.js — the detective's notebook, beside the game.
//
// The game publishes the case in play (`hw_case`, see game/src/mystery.rs):
// its lines in the player's language, the suspects, who is next to the
// character. This panel shows it with LiteAura's controls and sends the
// detective's moves back: a quick question to the suspect nearby, an
// accusation, the next case. Everything also works by voice; the panel is
// what makes the game playable without a microphone.

import { button, chip, iconButton } from '../../ui/index.js';

const WORDS = {
  it: {
    title: 'Taccuino', suspects: 'Indiziati', notes: 'Indizi', accuse: 'Accusa', next: 'Prossimo caso',
    near: 'Stai parlando con', talked: 'sentito', solved: 'Caso risolto!', failed: 'Il colpevole è scappato',
    level: 'Livello', away: 'm', fold: 'Chiudi il taccuino', open: 'Apri il taccuino', score: 'Risolti',
  },
  en: {
    title: 'Notebook', suspects: 'Suspects', notes: 'Clues', accuse: 'Accuse', next: 'Next case',
    near: 'Talking to', talked: 'questioned', solved: 'Case solved!', failed: 'The culprit got away',
    level: 'Level', away: 'm', fold: 'Close the notebook', open: 'Open the notebook', score: 'Solved',
  },
};

function word(lang, key) {
  return WORDS[String(lang).split('-')[0]]?.[key] ?? WORDS.en[key];
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/**
 * Build the notebook. `game` is the game module (its `hw_*` exports);
 * `lang` the player's language; `onProgress` is told whenever the progress
 * changes, to keep it. Returns `{ element, update(json) }`.
 */
export function createNotebook({ game, lang = 'en', onProgress }) {
  const panel = el('aside', 'notebook');
  panel.setAttribute('aria-label', word(lang, 'title'));
  panel.hidden = true;
  const head = el('div', 'notebook-head');
  const title = el('span', 'ui-eyebrow', word(lang, 'title'));
  const fold = iconButton({ icon: 'ui/chevron-up', label: word(lang, 'fold'), onClick: () => setFolded(!folded) });
  head.append(title, fold);
  const body = el('div', 'notebook-body');
  panel.append(head, body);
  let folded = false;
  let last = '';
  let lastProgress = '';

  function setFolded(value) {
    folded = value;
    panel.classList.toggle('is-folded', folded);
    fold.setAttribute('aria-label', word(lang, folded ? 'open' : 'fold'));
    fold.title = word(lang, folded ? 'open' : 'fold');
  }

  function render(book) {
    body.textContent = '';
    title.textContent = `${word(lang, 'title')} · ${word(lang, 'level')} ${book.level}`;
    // The outcome first, when the case is closed.
    if (book.phase === 'solved' || book.phase === 'failed') {
      const result = el('p', `notebook-result is-${book.phase}`, word(lang, book.phase));
      const next = button({ label: word(lang, 'next'), variant: 'primary', onClick: () => game.hw_next_case() });
      body.append(result, next);
    }
    // Who the character is talking to, and what to ask them.
    if (book.phase === 'investigating' && book.near != null) {
      const who = book.suspects[book.near];
      const talk = el('div', 'notebook-talk');
      talk.append(el('p', 'notebook-near', `${word(lang, 'near')} ${who.name}, ${who.role}`));
      const asks = el('div', 'notebook-asks');
      book.questions.forEach((question, index) => {
        asks.appendChild(chip({ label: question, onClick: () => game.hw_ask(book.near, index) }));
      });
      talk.appendChild(asks);
      body.appendChild(talk);
    }
    const notes = el('section', 'notebook-section');
    notes.appendChild(el('span', 'ui-eyebrow', word(lang, 'notes')));
    const list = el('ul', 'notebook-notes');
    for (const line of book.notes) list.appendChild(el('li', '', line));
    notes.appendChild(list);
    body.appendChild(notes);
    if (book.suspects.length) {
      const people = el('section', 'notebook-section');
      people.appendChild(el('span', 'ui-eyebrow', word(lang, 'suspects')));
      for (const [index, who] of book.suspects.entries()) {
        const row = el('div', 'notebook-suspect');
        const text = el('div', 'notebook-suspect-text');
        text.append(
          el('strong', '', who.name),
          el('span', 'notebook-meta', `${who.role} · ${who.place} · ${who.distance} ${word(lang, 'away')}${who.talked ? ` · ${word(lang, 'talked')}` : ''}`),
        );
        row.appendChild(text);
        if (book.phase === 'investigating') {
          row.appendChild(button({ label: word(lang, 'accuse'), variant: 'secondary', size: 'sm', onClick: () => game.hw_accuse(index) }));
        }
        people.appendChild(row);
      }
      body.appendChild(people);
    }
    body.appendChild(el('p', 'notebook-meta', `${word(lang, 'score')}: ${book.progress.solved}`));
  }

  return {
    element: panel,
    /** Show the latest notebook (a JSON string from `hw_case`). */
    update(json) {
      if (!json || json === last) return;
      last = json;
      let book;
      try { book = JSON.parse(json); } catch (_) { return; }
      const progress = JSON.stringify(book.progress);
      if (progress !== lastProgress) {
        lastProgress = progress;
        onProgress?.(book.progress);
      }
      panel.hidden = book.phase === 'waiting';
      if (!panel.hidden) render(book);
    },
  };
}
