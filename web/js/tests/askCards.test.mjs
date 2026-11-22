/**
 * Question, plan and choice cards: what a live card sends for each control,
 * how it closes, and how a stored one reads.
 *
 * Run:  node web/js/tests/askCards.test.mjs
 */

import './helpers/i18n.mjs';
import { dispatched } from './helpers/dom.mjs';

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

const { makeAskCard, renderAskRecord, parseAskRecord, askOutcome, lineText } = await import('../askCards.js');

const text = (node) => node?.textContent || '';
const buttons = (card) => card.querySelectorAll('button');
const byLabel = (card, label) => buttons(card).find((b) => text(b) === label);
/** A `send` that records the reply and answers as the server would. */
function recorder(accept = true) {
  const sent = [];
  return { sent, send: async (reply) => { sent.push(reply); return accept; } };
}
const settled = () => new Promise((resolve) => setTimeout(resolve, 0));

/* ── records ────────────────────────────────────────────────── */

check('a stored row parses from JSON text',
  parseAskRecord('{"kind":"question","title":{"text":"Hi"}}')?.kind === 'question');
check('an unknown kind is not a card', parseAskRecord({ kind: 'approval' }) === null);
check('broken JSON is not a card', parseAskRecord('{nope') === null && renderAskRecord('{nope') === null);
check('a keyed line is translated', lineText({ key: 'chat.ask.plan.approve', text: 'x' }) === 'Approve');
check('an unkeyed line is shown as written', lineText({ text: 'Notes plugin' }) === 'Notes plugin');
check('an unknown key falls back to its English', lineText({ key: 'chat.ask.nope', text: 'Fallback' }) === 'Fallback');

/* ── question ───────────────────────────────────────────────── */

{
  const question = {
    kind: 'question',
    request_id: 'r1',
    title: { text: 'Light or dark?' },
    options: [{ id: '1', label: { text: 'Light' } }, { id: '2', label: { text: 'Dark' }, detail: { text: 'easier at night' } }],
    allow_text: true,
  };
  const { sent, send } = recorder();
  let closed = 0;
  const live = makeAskCard(question, { send, onSettled: () => { closed += 1; } });
  check('question: tool card with its kind', live.card.classList.contains('tool-card') && live.card.classList.contains('ask-card--question'));
  check('question: title shown', text(live.card.querySelector('.ask-card-title')) === 'Light or dark?');
  check('question: header names the kind', text(live.card.querySelector('.tool-card-label')) === 'Question');
  check('question: two options and Send', buttons(live.card).length === 3, String(buttons(live.card).length));
  check('question: an option carries its detail as a hint', byLabel(live.card, 'Dark').title === 'easier at night');

  byLabel(live.card, 'Dark').click();
  await settled();
  check('question: picking an option sends its id', sent.length === 1 && sent[0].option === '2', JSON.stringify(sent));
  check('question: the card closes with the answer',
    live.resolved && text(live.card.querySelector('.ask-card-answer')) === 'Dark' && buttons(live.card).length === 0);
  check('question: the outcome is in the header', text(live.card.querySelector('.ask-card-status')) === 'Answered');
  check('question: onSettled runs once', closed === 1, String(closed));
  byLabel(live.card, 'Dark')?.click();
  check('question: a closed card sends nothing more', sent.length === 1);
}

{
  const { sent, send } = recorder();
  const live = makeAskCard({ kind: 'question', title: { text: 'Which city?' }, allow_text: true }, { send });
  const field = live.card.querySelector('input');
  check('question: free answer has a field and no options', !!field && buttons(live.card).length === 1);
  byLabel(live.card, 'Send').click();
  await settled();
  check('question: an empty answer is not sent', sent.length === 0);
  field.value = '  Milan ';
  field.fire('keydown', { key: 'Enter' });
  await settled();
  check('question: Enter sends the typed answer, trimmed', sent[0]?.text === 'Milan' && !sent[0]?.option, JSON.stringify(sent));
}

{
  const { send } = recorder(false);
  dispatched.length = 0;
  const live = makeAskCard({ kind: 'question', title: { text: 'Q' }, options: [{ id: '1', label: { text: 'A' } }] }, { send });
  byLabel(live.card, 'A').click();
  await settled();
  check('a refused answer leaves the card open', !live.resolved && !byLabel(live.card, 'A').disabled);
  check('a refused answer says so', dispatched.some((e) => e.type === 'app:toast' && e.detail?.type === 'error'));
  live.expire();
  check('an expired card reads "No answer"',
    live.resolved && text(live.card.querySelector('.ask-card-status')) === 'No answer' && buttons(live.card).length === 0);
}

/* ── plan ───────────────────────────────────────────────────── */

const plan = {
  kind: 'plan',
  title: { text: 'Notes plugin' },
  text: 'A small window with a list.',
  steps: ['Scaffold', 'Write the window', 'Install'],
  options: [
    { id: 'cancel', label: { key: 'chat.ask.plan.cancel', text: 'Cancel' } },
    { id: 'changes', label: { key: 'chat.ask.plan.changes', text: 'Ask for changes' } },
    { id: 'approve', label: { key: 'chat.ask.plan.approve', text: 'Approve' } },
  ],
  allow_text: true,
};

{
  const { sent, send } = recorder();
  const live = makeAskCard(plan, { send });
  check('plan: steps are a numbered list', live.card.querySelectorAll('li').map(text).join('|') === 'Scaffold|Write the window|Install');
  check('plan: three answers', ['Cancel', 'Ask for changes', 'Approve'].every((l) => byLabel(live.card, l)));
  const field = live.card.querySelector('input');
  field.value = 'looks great';
  field.fire('keydown', { key: 'Enter' });
  await settled();
  check('plan: Enter in the remark asks for changes, it never approves',
    sent[0]?.option === 'changes' && sent[0]?.text === 'looks great', JSON.stringify(sent));
  check('plan: closed as changes requested', text(live.card.querySelector('.ask-card-status')) === 'Changes requested');
}

{
  const { sent, send } = recorder();
  const live = makeAskCard(plan, { send });
  byLabel(live.card, 'Approve').click();
  await settled();
  check('plan: Approve sends approve', sent[0]?.option === 'approve');
  check('plan: closed as approved', live.card.classList.contains('is-ok') && text(live.card.querySelector('.ask-card-status')) === 'Approved');
}

check('plan: a cancelled plan reads so', askOutcome({ ...plan, answer: { option: 'cancel' } }).label === 'Cancelled');
check('plan: a remark alone is a request for changes',
  askOutcome({ ...plan, answer: { option: null, text: 'shorter' } }).label === 'Changes requested');

/* ── choice ─────────────────────────────────────────────────── */

const choice = {
  kind: 'choice',
  title: { key: 'chat.ask.language.title', text: 'Which language for this plugin?' },
  options: [
    { id: 'python', label: { text: 'Python' }, state: 'ready', detail: { key: 'chat.ask.language.pythonReady', text: 'Ready.' } },
    {
      id: 'rust',
      label: { text: 'Rust' },
      state: 'unavailable',
      detail: { key: 'chat.ask.language.needsDownload', text: 'Not here yet.' },
      notes: [{ key: 'chat.ask.missing.rustCompiler', text: 'Rust compiler' }, { key: 'chat.ask.missing.sdk', text: 'Plugin SDK sources' }],
    },
  ],
};

{
  const { sent, send } = recorder();
  const live = makeAskCard(choice, { send });
  const rows = live.card.querySelectorAll('.ask-card-choice');
  check('choice: one row per option', rows.length === 2);
  check('choice: a ready option says so', text(rows[0]).includes('Python') && text(rows[0]).includes('Ready'));
  check('choice: what is missing is listed on the option that needs it',
    rows[1].querySelectorAll('li').map(text).join('|') === 'Rust compiler (about 1.5 GB)|Plugin SDK sources'
      && text(rows[1]).includes('Not available yet'), text(rows[1]));
  check('choice: an option the machine lacks cannot be picked', rows[1].disabled && rows[1].classList.contains('is-unavailable'));
  rows[1].click();
  await settled();
  check('choice: clicking it sends nothing', sent.length === 0 && !live.resolved);
  rows[0].click();
  await settled();
  check('choice: the row sends its id', sent[0]?.option === 'python');
  const closed = live.card.querySelectorAll('.ask-card-choice');
  check('choice: both options stay in view, the chosen one marked',
    closed.length === 2 && closed[0].classList.contains('is-picked') && !closed[1].classList.contains('is-picked'));
  check('choice: no buttons left', buttons(live.card).length === 0);
}

{
  const stored = renderAskRecord(JSON.stringify({ ...choice, answer: { option: 'python', text: null } }));
  check('stored choice: rebuilt closed', stored.classList.contains('is-resolved') && buttons(stored).length === 0);
  check('stored choice: the pick is marked', stored.querySelectorAll('.is-picked').length === 1
    && text(stored.querySelector('.is-picked')).includes('Python'));
  const abandoned = renderAskRecord({ ...choice, abandoned: true });
  check('stored choice: left unanswered', text(abandoned.querySelector('.ask-card-status')) === 'No answer'
    && abandoned.querySelectorAll('.is-picked').length === 0);
  const closedCard = renderAskRecord({ ...choice, answer: { option: null, text: null } });
  check('stored choice: closed without a pick is no answer', text(closedCard.querySelector('.ask-card-status')) === 'No answer');
}

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall ask-card checks passed');
