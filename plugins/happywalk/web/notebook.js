// notebook.js — the detective's notebook, beside the game.
//
// The game publishes the case in play (`hw_case`, see game/src/mystery.rs):
// its notes in the player's language, the suspects and where they are, who
// the character is next to and the conversation with them. This panel shows
// it with LiteAura's controls and sends the detective's moves back:
//
//   * the chat with whoever is near: typed, or spoken (the microphone button
//     writes down what is said, with the speech recogniser of the user's
//     voice language) and sent to the game (`hw_chat`), which asks the
//     language model to answer as that person;
//   * a walk to someone's door, or to the office (`hw_go`);
//   * an accusation (`hw_accuse`), and the next case (`hw_next_case`).
//
// The chat box is built once and kept: the rest is redrawn when the game's
// notebook changes, so what the player is typing is never lost.

import { button, chip, iconButton, input } from '../../ui/index.js';

const WORDS = {
  it: {
    title: 'Taccuino', suspects: 'Indiziati', notes: 'Indizi', accuse: 'Accusa', next: 'Prossimo caso',
    solved: 'Caso risolto!', failed: 'Il colpevole è scappato', level: 'Livello', fold: 'Chiudi il taccuino',
    open: 'Apri il taccuino', score: 'Risolti', talking: 'Parli con', write: 'Scrivi una domanda…',
    send: 'Invia', speak: 'Parla', stop: 'Basta, invia', listening: 'Ti ascolto…', thinking: 'sta pensando…',
    go: 'Vai', toOffice: 'Torna in ufficio: arriverà un cliente.', waiting: 'Aspetta: sta arrivando un cliente…',
    away: 'm', talked: 'sentito', nobody: 'Avvicinati a qualcuno per parlargli.', noMic: 'Microfono non disponibile',
  },
  en: {
    title: 'Notebook', suspects: 'Suspects', notes: 'Clues', accuse: 'Accuse', next: 'Next case',
    solved: 'Case solved!', failed: 'The culprit got away', level: 'Level', fold: 'Close the notebook',
    open: 'Open the notebook', score: 'Solved', talking: 'Talking to', write: 'Write a question…',
    send: 'Send', speak: 'Speak', stop: 'Done, send', listening: 'Listening…', thinking: 'is thinking…',
    go: 'Go', toOffice: 'Back to the office: a client will come.', waiting: 'Wait: a client is coming…',
    away: 'm', talked: 'questioned', nobody: 'Go up to someone to talk to them.', noMic: 'No microphone',
  },
};

export function word(lang, key) {
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
 * `lang` the player's language; `dictate` writes down speech
 * (`{ start(onText), stop() → Promise<text> }`, or null with no microphone);
 * `onProgress` is told whenever the progress changes, to keep it. Returns
 * `{ element, update(json) }`.
 */
export function createNotebook({ game, lang = 'en', dictate = null, onProgress }) {
  const w = (key) => word(lang, key);
  const panel = el('aside', 'notebook');
  panel.setAttribute('aria-label', w('title'));
  panel.hidden = true;
  const head = el('div', 'notebook-head');
  const title = el('span', 'ui-eyebrow', w('title'));
  const fold = iconButton({ icon: 'ui/chevron-up', label: w('fold'), onClick: () => setFolded(!folded) });
  head.append(title, fold);
  const body = el('div', 'notebook-body');
  panel.append(head, body);

  // Parts redrawn from the game's notebook.
  const status = el('div', 'notebook-status');
  const chatBox = el('section', 'notebook-chat');
  const who = el('p', 'notebook-near');
  const lines = el('div', 'notebook-lines');
  const asks = el('div', 'notebook-asks');
  const rest = el('div', 'notebook-rest');

  // The chat's input row: built once, kept.
  const field = input({ placeholder: w('write'), label: w('write'), autocomplete: 'off', className: 'notebook-input' });
  const send = iconButton({ icon: 'ui/send', label: w('send'), onClick: () => submit() });
  const mic = iconButton({ icon: 'ui/mic', label: w('speak'), onClick: () => toggleMic() });
  if (!dictate) mic.hidden = true;
  const inputRow = el('div', 'notebook-input-row');
  inputRow.append(field, mic, send);
  chatBox.append(who, lines, asks, inputRow);
  body.append(status, chatBox, rest);

  field.addEventListener('focus', () => game.hw_typing(true));
  field.addEventListener('blur', () => game.hw_typing(false));
  field.addEventListener('keydown', (event) => {
    event.stopPropagation();
    if (event.key === 'Enter') { event.preventDefault(); submit(); }
  });

  let folded = false;
  let last = '';
  let lastProgress = '';
  let book = null;
  let listening = false;

  function setFolded(value) {
    folded = value;
    panel.classList.toggle('is-folded', folded);
    fold.setAttribute('aria-label', w(folded ? 'open' : 'fold'));
    fold.title = w(folded ? 'open' : 'fold');
  }

  function submit(text = field.value) {
    const message = String(text || '').trim();
    if (!message || !book?.talking || book.thinking) return;
    game.hw_chat(message);
    field.value = '';
  }

  async function toggleMic() {
    if (!dictate) return;
    if (!listening) {
      listening = true;
      mic.setAttribute('aria-label', w('stop'));
      mic.classList.add('is-active');
      field.placeholder = w('listening');
      try {
        await dictate.start((partial) => { field.value = partial; });
      } catch (_) {
        listening = false;
        mic.classList.remove('is-active');
        mic.setAttribute('aria-label', w('speak'));
        field.placeholder = w('noMic');
      }
      return;
    }
    listening = false;
    mic.classList.remove('is-active');
    mic.setAttribute('aria-label', w('speak'));
    field.placeholder = w('write');
    const text = await dictate.stop().catch(() => field.value);
    submit(text || field.value);
  }

  function goButton(target) {
    return iconButton({ icon: 'ui/navigator', label: w('go'), onClick: () => game.hw_go(target) });
  }

  function render() {
    title.textContent = `${w('title')} · ${w('level')} ${book.level}`;
    // Where the story is.
    status.textContent = '';
    if (book.phase === 'waiting') {
      const line = el('div', 'notebook-row');
      line.appendChild(el('p', 'notebook-hint', book.room === 'office' ? w('waiting') : w('toOffice')));
      if (book.room === 'town') line.append(el('span', 'notebook-meta', `${book.office} ${w('away')}`), goButton(-1));
      status.appendChild(line);
    } else if (book.phase === 'solved' || book.phase === 'failed') {
      status.append(
        el('p', `notebook-result is-${book.phase}`, w(book.phase)),
        button({ label: w('next'), variant: 'primary', onClick: () => game.hw_next_case() }),
      );
    }
    // The conversation.
    chatBox.hidden = !book.talking;
    if (book.talking) {
      who.textContent = `${w('talking')} ${book.talking.name}, ${book.talking.role}`;
      lines.textContent = '';
      for (const line of book.chat) lines.appendChild(el('p', `notebook-line is-${line.from}`, line.text));
      if (book.thinking) lines.appendChild(el('p', 'notebook-line is-note', `${book.talking.name} ${w('thinking')}`));
      lines.scrollTop = lines.scrollHeight;
      asks.textContent = '';
      if (!book.chat.length && book.talking.who !== 'client') {
        for (const question of book.questions) asks.appendChild(chip({ label: question, onClick: () => submit(question) }));
      }
      field.disabled = !!book.thinking;
      send.disabled = !!book.thinking;
    } else if (listening) {
      toggleMic();
    }
    // Notes and people.
    rest.textContent = '';
    if (!book.talking && book.phase === 'investigating' && book.room !== 'town') rest.appendChild(el('p', 'notebook-meta', w('nobody')));
    if (book.notes.length) {
      const notes = el('section', 'notebook-section');
      notes.appendChild(el('span', 'ui-eyebrow', w('notes')));
      const list = el('ul', 'notebook-notes');
      for (const line of book.notes) list.appendChild(el('li', '', line));
      notes.appendChild(list);
      rest.appendChild(notes);
    }
    if (book.suspects.length) {
      const people = el('section', 'notebook-section');
      people.appendChild(el('span', 'ui-eyebrow', w('suspects')));
      for (const [index, person] of book.suspects.entries()) {
        const row = el('div', 'notebook-suspect');
        const text = el('div', 'notebook-suspect-text');
        const where = book.room === 'town' ? `${person.distance} ${w('away')}` : '';
        text.append(
          el('strong', '', person.name),
          el('span', 'notebook-meta', [person.role, person.place, where, person.talked ? w('talked') : ''].filter(Boolean).join(' · ')),
        );
        row.appendChild(text);
        const actions = el('div', 'notebook-actions');
        if (book.room === 'town' && book.phase === 'investigating') actions.appendChild(goButton(index));
        if (book.phase === 'investigating') {
          actions.appendChild(button({ label: w('accuse'), variant: 'secondary', size: 'sm', onClick: () => game.hw_accuse(index) }));
        }
        row.appendChild(actions);
        people.appendChild(row);
      }
      rest.appendChild(people);
    }
    rest.appendChild(el('p', 'notebook-meta', `${w('score')}: ${book.progress.solved}`));
  }

  return {
    element: panel,
    /** Show the latest notebook (a JSON string from `hw_case`). */
    update(json) {
      if (!json || json === last) return;
      last = json;
      try { book = JSON.parse(json); } catch (_) { return; }
      const progress = JSON.stringify(book.progress);
      if (progress !== lastProgress) {
        lastProgress = progress;
        onProgress?.(book.progress);
      }
      panel.hidden = false;
      render();
    },
  };
}
