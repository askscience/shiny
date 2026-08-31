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

let panel = null;
let view = 'list'; // 'list' | 'messages'
let conversations = [];

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
  view = 'list';
  panel.body.textContent = '';

  const head = h('div', 'chat-history-head');
  const title = h('h3', 'chat-history-title', 'Chats');
  const newBtn = button({ icon: 'ui/plus', label: 'New chat', variant: 'primary', size: 'sm' });
  newBtn.addEventListener('click', () => {
    newChat();
    panel.close();
    toast('Started a new chat', { type: 'info' });
  });
  head.append(title, newBtn);
  panel.body.appendChild(head);

  const list = h('div', 'chat-history-list');
  if (!conversations.length) {
    list.appendChild(emptyState({ title: 'No chats yet', body: 'Your conversations with Shiny will appear here.' }));
    panel.body.appendChild(list);
    return;
  }

  const current = currentConversationId();
  for (const c of conversations) {
    const row = h('button', 'chat-history-item');
    row.type = 'button';
    if (c.id === current) row.classList.add('is-active');

    const meta = h('div', 'chat-history-meta');
    meta.appendChild(h('div', 'chat-history-item-title', c.title || 'New chat'));
    meta.appendChild(h('div', 'chat-history-item-preview', c.preview || ''));
    row.appendChild(meta);

    const right = h('div', 'chat-history-side');
    right.appendChild(h('span', 'chat-history-item-time', fmtWhen(c.updated_at)));
    const del = h('button', 'chat-history-del');
    del.type = 'button';
    del.title = 'Delete chat';
    const ic = h('span', 'chat-history-del-icon');
    del.appendChild(ic);
    void setIcon(ic, 'ui/trash', { size: 14 });
    del.addEventListener('click', async (e) => {
      e.stopPropagation();
      if (!window.confirm('Delete this chat?')) return;
      if (await removeConversation(c.id)) {
        toast('Chat deleted', { type: 'info' });
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
  view = 'messages';
  panel.body.textContent = '';

  const back = button({ icon: 'ui/arrow-left', label: 'Back', variant: 'ghost', size: 'sm' });
  back.classList.add('ui-btn--icon');
  back.addEventListener('click', () => { renderList(); });

  const cont = button({ label: 'Continue this chat', variant: 'primary', size: 'sm' });
  cont.addEventListener('click', () => {
    setCurrentConversation(c.id);
    panel.close();
    toast('Resumed chat', { type: 'info' });
  });

  const head = h('div', 'chat-history-head');
  const t = h('div', 'chat-history-meta');
  t.appendChild(h('div', 'chat-history-title', c.title || 'New chat'));
  t.appendChild(back);
  head.appendChild(t);
  head.appendChild(cont);
  panel.body.appendChild(head);

  const msgs = h('div', 'chat-history-messages');
  const entries = await loadConversationMessages(c.id);
  if (!entries.length) {
    msgs.appendChild(emptyState({ title: 'No messages', body: 'Say something to start.' }));
  }
  for (const m of entries) {
    const bubble = h('div', `chat-history-bubble chat-history-bubble--${m.role}`);
    bubble.appendChild(h('div', 'chat-history-bubble-role', m.role === 'user' ? 'You' : 'Shiny'));
    bubble.appendChild(h('div', 'chat-history-bubble-text', m.content));
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
  panel = modal({ title: null, wide: true });
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
