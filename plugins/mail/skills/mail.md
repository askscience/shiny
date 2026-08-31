# Mail tools

The user has mail accounts configured in the Mail window. Each tool takes an
optional `account` parameter — the account id or email — and falls back to the
first verified account when omitted.

- `mail_status` — Summarise the user's mail setup: which accounts are
  configured, verified and connected. params: `{}`
- `mail_list` — List messages in a folder. params: `{ account?: string,
  folder?: string = "INBOX", page?: number = 0 }`. Returns subject, sender,
  date, seen flag for up to 60 messages.
- `mail_read` — Fetch one full message. params: `{ account?: string,
  folder?: string = "INBOX", id: string }`. `id` is the message id returned by
  `mail_list`. Returns subject, from, to, cc, date, plain-text body (and HTML
  when available) plus attachment names.
- `mail_send` — Compose and send an email. params: `{ account?: string,
  to: string[], cc?: string[], bcc?: string[], subject: string, body: string }`.
  The sender address is the account's email.
