# Mail — agent tools

Every tool takes an optional `account` (id or email) and falls back to the
first verified account. List/search/read use the **local cache**; call
`mail_sync` first when the very latest mail is needed.

| Tool | Params | Returns / behaviour |
|---|---|---|
| `mail_status` | `{}` | `{ configured, accounts }`: which accounts are configured and verified. |
| `mail_sync` | `{ account?, folder? }` | Downloads new mail into the cache (default `INBOX`). |
| `mail_list` | `{ account?, folder?, page? }` | Up to 60 cached messages: subject, sender, date, `seen`. |
| `mail_search` | `{ account?, folder?, query }` | Cached subject/sender/body search. A never-cached `folder` is downloaded first; omit `folder` to search all — INBOX plus up to 8 never-cached folders with mail are downloaded first (folder list re-checked at most every 10 min). |
| `mail_read` | `{ account?, folder?, id }` | One full message: subject, from/to/cc, date, plain body (+HTML), attachment names. |
| `mail_send` | `{ account?, to[], cc?, bcc?, subject, body, attachments? }` | Sends via the account's SMTP; always goes to the server. `attachments`: home-relative file paths. Refused when `subject` and `body` are both empty. |
| `mail_save_attachment` | `{ account?, folder?, id, index?, dir? }` | Saves one attachment of a received message into the home (default `Downloads`) and returns its path. |

## Guidance

- Find a specific email with `mail_search` before `mail_read`.
- `id` is the message id returned by `mail_list`/`mail_search`.
- The sender address is the account's email.

## Registration

Wrapped with `bridged(..)` in [`src/plugin.rs`](../src/plugin.rs); the skill is
[`skills/mail.md`](../skills/mail.md).
