# Mail tools

The user has mail accounts configured in the Mail window. Each tool takes an
optional `account` parameter — the account id or email — and falls back to the
first verified account when omitted.

Mail is cached locally: `mail_list`, `mail_search` and `mail_read` read from the
local cache (instant, no network). The first time you list/search a folder it is
downloaded into the cache automatically. To pull in brand-new mail, call
`mail_sync`.

- `mail_status` — Summarise the user's mail setup: which accounts are
  configured, verified and connected. params: `{}`
- `mail_sync` — Download new mail into the local cache for a folder (default
  INBOX). params: `{ account?: string, folder?: string = "INBOX" }`. Call this
  before list/read when the user wants the very latest mail.
- `mail_list` — List messages in a folder from the cache. params:
  `{ account?: string, folder?: string = "INBOX", page?: number = 0 }`. Returns
  subject, sender, date, seen flag for up to 60 messages. Use `page` to page
  through older mail.
- `mail_search` — Find messages by text, searching the cached subject, sender
  and body (local, no network). params: `{ account?: string, folder?: string,
  query: string }`. Omit `folder` to search all folders. Use it to find a
  specific email ("the email about the invoice") before calling `mail_read`.
- `mail_read` — Fetch one full message from the cache. params:
  `{ account?: string, folder?: string = "INBOX", id: string }`. `id` is the
  message id returned by `mail_list`/`mail_search`. Returns subject, from, to,
  cc, date, plain-text body (and HTML when available) plus `attachments`
  (`index`, `filename`, `content_type`, `size`).
- `mail_send` — Compose and send an email. params: `{ account?: string,
  to: string[], cc?: string[], bcc?: string[], subject: string, body: string,
  attachments?: string[] }`. The sender address is the account's email. Sending
  always goes through the mail server. `attachments` are files in the user's
  home as home-relative paths (`Documents/report.pdf`) — the paths the Files
  tools list or a document/image tool reports after saving. 18 MB in total.
  To forward an attachment you received, save it first with
  `mail_save_attachment`, then attach the returned path.
- `mail_save_attachment` — Save one attachment of a received message into the
  user's home. params: `{ account?: string, folder?: string = "INBOX",
  id: string, index?: number = 0, dir?: string = "Downloads" }`. `index` is the
  attachment's position in `mail_read`'s list. Returns the saved `path`.
