/**
 * chatHistory.js — the core chat history panel (new chat / continue old chats).
 *
 * The AI agent stores conversations server-side (text only, never audio); this
 * panel lists them so a conversation can be resumed, and offers a "New chat".
 */

import { button, emptyState, icon, modal, toast } from '../ui/index.js';
import { setIcon } from '../ui/index.js';
import {
  currentConversationId, setCurrentConversation, newChat,
  listConversations, loadConversationMessages, removeConversation,
} from './agent.js';
import { t } from './i18n.js';

let panel = null;
let view = 'list'; // 'list' | 'messages'
let conversations = [];
// Bumped on every view switch so a slow `loadConversationMessages` can
// detect that the user navigated away while it was in flight.
let renderToken = 0;

function h(tag, className, text) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text != null) el.textContent = text;
  return el;
}

function fmtWhen(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleString([], {
    month: 'short', day: 'numeric',
    hour: '2-digit', minute: '2-digit',
  });
}

function renderList() {
  renderToken += 1; // any in-flight message render is obsolete now
  view = 'list';
  panel.body.textContent = '';

  const head = h('div', 'chat-history-head');
  const title = h('h3', 'chat-history-title', t('chat.chats', null, 'Chats'));
  const newBtn = button({
    icon: 'ui/plus', label: t('chat.newChat', null, 'New chat'), variant: 'primary', size: 'sm',
  });
  newBtn.addEventListener('click', () => {
    newChat();
    panel.close();
    toast(t('ctx.chatStarted', null, 'Started a new chat'), { type: 'info' });
  });
  head.append(title, newBtn);
  panel.body.appendChild(head);

  const list = h('div', 'chat-history-list');
  if (!conversations.length) {
    list.appendChild(emptyState({
      title: t('chat.noChatsYet', null, 'No chats yet'),
      body: t('chat.historyEmptyBody', null, "Your conversations with PEAK'D! will appear here."),
    }));
    panel.body.appendChild(list);
    return;
  }

  const current = currentConversationId();
  for (const c of conversations) {
    const row = h('button', 'chat-history-item');
    row.type = 'button';
    if (c.id === current) row.classList.add('is-active');

    const meta = h('div', 'chat-history-meta');
    meta.appendChild(h('div', 'chat-history-item-title', c.title || t('chat.newChat', null, 'New chat')));
    meta.appendChild(h('div', 'chat-history-item-preview', c.preview || ''));
    row.appendChild(meta);

    const right = h('div', 'chat-history-side');
    right.appendChild(h('span', 'chat-history-item-time', fmtWhen(c.updated_at)));
    const del = h('button', 'chat-history-del');
    del.type = 'button';
    del.title = t('chat.deleteChat', null, 'Delete chat');
    const ic = h('span', 'chat-history-del-icon');
    del.appendChild(ic);
    void setIcon(ic, 'ui/trash', { size: 14 });
    del.addEventListener('click', async (e) => {
      e.stopPropagation();
      if (!window.confirm(t('chat.deleteChatConfirm', null, 'Delete this chat?'))) return;
      if (await removeConversation(c.id)) {
        toast(t('chat.deleted', null, 'Chat deleted'), { type: 'info' });
        await refreshList();
      }
    });
    right.appendChild(del);
    row.appendChild(right);

    row.addEventListener('click', () => void openMessages(c));
    list.appendChild(row);
  }
  panel.body.appendChild(list);
}

async function openMessages(c) {
  const token = ++renderToken;
  view = 'messages';
  panel.body.textContent = '';

  const back = button({
    icon: 'ui/arrow-left', label: t('common.back', null, 'Back'), variant: 'ghost', size: 'sm',
  });
  back.classList.add('ui-btn--icon');
  back.addEventListener('click', () => { renderList(); });

  const cont = button({
    label: t('chat.continueChat', null, 'Continue this chat'), variant: 'primary', size: 'sm',
  });
  cont.addEventListener('click', () => {
    setCurrentConversation(c.id);
    panel.close();
    toast(t('chat.resumed', null, 'Resumed chat'), { type: 'info' });
  });

  const head = h('div', 'chat-history-head');
  const meta = h('div', 'chat-history-meta');
  meta.appendChild(h('div', 'chat-history-title', c.title || t('chat.newChat', null, 'New chat')));
  meta.appendChild(back);
  head.appendChild(meta);
  head.appendChild(cont);
  panel.body.appendChild(head);

  const msgs = h('div', 'chat-history-messages');
  const entries = await loadConversationMessages(c.id);
  // The user hit Back (or opened another chat) while loading — appending
  // now would stack the message view on top of the list.
  if (token !== renderToken) return;
  if (!entries.length) {
    msgs.appendChild(emptyState({
      title: t('chat.noMessages', null, 'No messages'),
      body: t('chat.noMessagesBody', null, 'Say something to start.'),
    }));
  }
  for (const m of entries) {
    const isUser = m.role === 'user';
    let text = m.content;
    // Mid-turn rows are not plain text: show the command or the generated
    // media name instead of the stored JSON.
    if (m.role === 'approval' || m.role === 'media') {
      try {
        const record = JSON.parse(m.content);
        if (m.role === 'approval') {
          text = `$ ${record.command || ''}`;
        } else if (record.kind === 'song') {
          text = `${t('chat.song', null, 'Song')}: ${record.title || record.file || ''}`;
        } else {
          text = `${t('chat.generatedImage', null, 'Generated image')}: ${record.file || ''}`;
        }
      } catch (_) { /* keep the raw content */ }
    }
    const bubble = h('div', `chat-history-bubble chat-history-bubble--${isUser ? 'user' : 'assistant'}`);
    const role = isUser ? t('chat.you', null, 'You') : "PEAK'D!";
    bubble.appendChild(h('div', 'chat-history-bubble-role', role));
    bubble.appendChild(h('div', 'chat-history-bubble-text', text));
    msgs.appendChild(bubble);
  }
  panel.body.appendChild(msgs);
}

async function refreshList() {
  conversations = await listConversations();
  if (view === 'list') renderList();
}

function ensurePanel() {
  if (panel) return;
  panel = modal({ title: null, label: t('chat.history'), wide: true });
  panel.el.classList.add('chat-history-panel');
  panel.body.style.gap = '0'; // title bar + padded content, like a plugin window
}

export function openChatHistory() {
  ensurePanel();
  panel.open();
  void refreshList();
}

export function initChatHistory() {
  const btn = document.getElementById('chat-history-btn');
  if (btn) {
    btn.addEventListener('click', () => {
      if (panel?.isOpen?.()) panel.close();
      else openChatHistory();
    });
  }
}
