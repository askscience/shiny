/**
 * Cards the assistant waits on in the middle of an answer: a question, a plan
 * to approve, a choice between options (the plugin builder's Python / Rust).
 *
 * The core sends one as an `ask` event and parks the turn until the answer is
 * posted to `/api/agent/answer`; the same card, with its answer, is stored as
 * an `ask` row of the conversation, so a reopened chat shows what was asked
 * and what was decided.
 *
 * A line the core wrote carries a catalogue key next to its English; a line
 * the model wrote has none and is shown as it is.
 */
import { icon, button, input, badge } from '../ui/index.js';
import { t } from './i18n.js';

const KIND_ICON = { question: 'ui/message-circle', plan: 'ui/list-ordered', choice: 'ui/puzzle' };

/** A stored row (JSON text) or a live event, as one object. */
export function parseAskRecord(raw) {
  let record = raw;
  if (typeof raw === 'string') {
    try { record = JSON.parse(raw); } catch { return null; }
  }
  if (!record || typeof record !== 'object') return null;
  if (!['question', 'plan', 'choice'].includes(record.kind)) return null;
  return record;
}

/** A line's text in the interface language. */
export function lineText(line) {
  if (!line) return '';
  if (typeof line === 'string') return line;
  const text = typeof line.text === 'string' ? line.text : '';
  return line.key ? t(line.key, null, text) : text;
}

function kindLabel(kind) {
  if (kind === 'plan') return t('chat.ask.plan.title', null, 'Plan');
  if (kind === 'choice') return t('chat.ask.kind.choice', null, 'Choice');
  return t('chat.ask.kind.question', null, 'Question');
}

/**
 * How an answered card reads: `tone` colours the mark in its header, `label`
 * names the outcome, `answer` is what the user picked or wrote.
 */
export function askOutcome(record) {
  const answer = record?.answer;
  const option = typeof answer?.option === 'string' ? answer.option : '';
  const text = typeof answer?.text === 'string' ? answer.text.trim() : '';
  const picked = (record?.options || []).find((o) => o.id === option);

  if (!answer || record.abandoned) {
    return { tone: 'none', label: t('chat.ask.noAnswer', null, 'No answer'), answer: '' };
  }
  if (record.kind === 'plan') {
    if (option === 'approve') {
      return { tone: 'ok', label: t('chat.ask.plan.approved', null, 'Approved'), answer: text };
    }
    if (option === 'cancel') {
      return { tone: 'no', label: t('chat.ask.plan.cancelled', null, 'Cancelled'), answer: text };
    }
    return { tone: 'neutral', label: t('chat.ask.plan.changesAsked', null, 'Changes requested'), answer: text };
  }
  if (record.kind === 'choice') {
    if (!picked) return { tone: 'none', label: t('chat.ask.noAnswer', null, 'No answer'), answer: '' };
    return {
      tone: 'ok',
      label: t('chat.ask.language.chosen', null, 'Chosen for this chat'),
      answer: lineText(picked.label),
    };
  }
  // A secret (a password) is never shown back.
  const parts = [picked ? lineText(picked.label) : '', record.secret ? '' : text].filter(Boolean);
  if (record.secret && text) {
    return { tone: 'ok', label: t('chat.ask.answered', null, 'Answered'), answer: '' };
  }
  if (!parts.length) return { tone: 'none', label: t('chat.ask.noAnswer', null, 'No answer'), answer: '' };
  return { tone: 'ok', label: t('chat.ask.answered', null, 'Answered'), answer: parts.join(' — ') };
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null && text !== '') node.textContent = text;
  return node;
}

/** The part of a card that never changes: header, title, text, plan steps. */
function shell(record) {
  const card = el('article', `tool-card tool-card--ask ask-card ask-card--${record.kind}`);
  card.setAttribute('role', 'group');
  const head = el('header', 'tool-card-head');
  head.append(
    icon(KIND_ICON[record.kind], { size: 14, className: 'tool-card-icon' }),
    el('span', 'tool-card-label', kindLabel(record.kind)),
  );
  card.append(head);

  const title = lineText(record.title);
  if (title) card.append(el('div', 'ask-card-title', title));
  card.setAttribute('aria-label', title || kindLabel(record.kind));
  if (record.text) card.append(el('p', 'tool-card-text', record.text));

  if (Array.isArray(record.steps) && record.steps.length) {
    const steps = el('ol', 'ask-card-steps');
    for (const step of record.steps) steps.append(el('li', '', step));
    card.append(steps);
  }
  return { card, head };
}

/** One option of a choice: its name, whether it is ready, what it still needs. */
function choiceBody(option) {
  const body = el('span', 'ask-card-choice-body');
  const top = el('span', 'ask-card-choice-top');
  top.append(el('span', 'ask-card-choice-name', lineText(option.label)));
  if (option.state === 'ready') {
    top.append(badge(t('chat.ask.language.ready', null, 'Ready'), { tone: 'ok' }));
  } else if (option.state === 'setup') {
    top.append(badge(t('chat.ask.language.setup', null, 'Will be installed'), { tone: 'accent' }));
  } else if (option.state === 'unavailable') {
    top.append(badge(t('chat.ask.language.unavailable', null, 'Not available yet'), { tone: 'warn' }));
  }
  body.append(top);
  const detail = lineText(option.detail);
  if (detail) body.append(el('span', 'ask-card-choice-detail', detail));
  if (Array.isArray(option.notes) && option.notes.length) {
    const notes = el('ul', 'ask-card-choice-notes');
    for (const note of option.notes) notes.append(el('li', '', lineText(note)));
    body.append(notes);
  }
  return body;
}

/** Close the card: no controls any more, the outcome in the header. */
function settle(card, head, record) {
  if (card.classList.contains('is-resolved')) return;
  const outcome = askOutcome(record);
  card.classList.add('is-resolved', `is-${outcome.tone}`);
  card.querySelectorAll('.ask-card-controls').forEach((node) => node.remove());
  const mark = el('span', `ask-card-status is-${outcome.tone}`, outcome.label);
  head.append(mark);
  if (record.kind === 'choice') {
    // Keep the options in view, with the picked one marked: what the other
    // needed is part of what was decided.
    const picked = record.answer?.option;
    const list = el('div', 'ask-card-choices is-resolved');
    for (const option of record.options || []) {
      const row = el('div', `ask-card-choice${option.id === picked ? ' is-picked' : ''}`);
      row.append(choiceBody(option));
      list.append(row);
    }
    card.append(list);
  } else if (outcome.answer) {
    card.append(el('div', 'ask-card-answer', outcome.answer));
  }
}

/** A card from the stored conversation: what was asked and how it ended. */
export function renderAskRecord(raw) {
  const record = parseAskRecord(raw);
  if (!record) return null;
  const { card, head } = shell(record);
  settle(card, head, record);
  return card;
}

/**
 * A live card. `send(reply)` posts `{ option?, text? }` and resolves to
 * whether the turn took it; `onSettled()` runs once the card has closed.
 * Returns the card and `expire()`, for a turn that ended before the user
 * answered.
 */
export function makeAskCard(prompt, { send, onSettled } = {}) {
  const record = parseAskRecord(prompt);
  if (!record) return null;
  const { card, head } = shell(record);
  const controls = el('div', 'ask-card-controls');
  card.append(controls);

  let busy = false;
  const answer = async (reply) => {
    if (busy || card.classList.contains('is-resolved')) return;
    busy = true;
    const buttons = [...controls.querySelectorAll('button, input')];
    buttons.forEach((b) => { b.disabled = true; });
    let ok = false;
    try { ok = !!(await send?.(reply)); } catch { ok = false; }
    if (ok) {
      if (record.secret && textField) textField.value = '';
      settle(card, head, { ...record, answer: reply });
      onSettled?.();
    } else {
      busy = false;
      buttons.forEach((b) => { b.disabled = false; });
      window.dispatchEvent(new CustomEvent('app:toast', {
        detail: { message: t('chat.ask.failed', null, 'Could not send your answer'), type: 'error' },
      }));
    }
  };

  let textField = null;
  const fieldRow = (placeholder) => {
    const row = el('div', 'ask-card-field');
    textField = input({
      placeholder,
      label: placeholder,
      maxlength: 2000,
      className: 'ask-card-input',
      // A secret is typed hidden and kept out of the browser's suggestions.
      type: record.secret ? 'password' : 'text',
      autocomplete: record.secret ? 'off' : undefined,
    });
    row.append(textField);
    return row;
  };
  const typed = () => (textField?.value || '').trim();

  if (record.kind === 'plan') {
    controls.append(fieldRow(t('chat.ask.remarkPlaceholder', null, 'What should change? (optional)')));
    const actions = el('div', 'ask-card-actions');
    actions.append(
      button({
        label: t('chat.ask.plan.cancel', null, 'Cancel'),
        variant: 'ghost',
        size: 'sm',
        onClick: () => void answer({ option: 'cancel', text: typed() }),
      }),
      button({
        label: t('chat.ask.plan.changes', null, 'Ask for changes'),
        variant: 'quiet',
        size: 'sm',
        onClick: () => void answer({ option: 'changes', text: typed() }),
      }),
      button({
        label: t('chat.ask.plan.approve', null, 'Approve'),
        variant: 'primary',
        size: 'sm',
        onClick: () => void answer({ option: 'approve', text: typed() }),
      }),
    );
    controls.append(actions);
    // Enter in the remark asks for those changes; it never approves.
    textField.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && typed()) {
        e.preventDefault();
        void answer({ option: 'changes', text: typed() });
      }
    });
  } else if (record.kind === 'choice') {
    const list = el('div', 'ask-card-choices');
    for (const option of record.options || []) {
      // An option the machine cannot use yet is shown with what it lacks,
      // and cannot be picked.
      const unavailable = option.state === 'unavailable';
      const row = button({
        variant: 'quiet',
        disabled: unavailable,
        onClick: () => { if (!unavailable) void answer({ option: option.id }); },
      });
      row.classList.add('ask-card-choice');
      if (unavailable) row.classList.add('is-unavailable');
      row.textContent = '';
      row.append(choiceBody(option));
      list.append(row);
    }
    controls.append(list);
  } else {
    if (Array.isArray(record.options) && record.options.length) {
      const list = el('div', 'ask-card-options');
      for (const option of record.options) {
        const pick = button({
          label: lineText(option.label),
          variant: 'quiet',
          size: 'sm',
          onClick: () => void answer({ option: option.id }),
        });
        const detail = lineText(option.detail);
        if (detail) pick.title = detail;
        list.append(pick);
      }
      controls.append(list);
    }
    if (record.allow_text !== false) {
      const row = fieldRow(record.secret
        ? t('chat.ask.passwordPlaceholder', null, 'Password')
        : t('chat.ask.answerPlaceholder', null, 'Type your answer…'));
      const sendButton = button({
        label: t('chat.ask.send', null, 'Send'),
        variant: 'primary',
        size: 'sm',
        onClick: () => { if (typed()) void answer({ text: typed() }); },
      });
      row.append(sendButton);
      controls.append(row);
      textField.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' && typed()) {
          e.preventDefault();
          void answer({ text: typed() });
        }
      });
    }
  }

  return {
    card,
    focus: () => textField?.focus({ preventScroll: true }),
    expire: () => settle(card, head, { ...record, abandoned: true }),
    get resolved() { return card.classList.contains('is-resolved'); },
  };
}
