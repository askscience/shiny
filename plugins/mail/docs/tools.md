# Mail — agent tools

Every tool takes an optional `account` (id or email) and falls back to the
first verified account. List/search/read use the **local cache**; call
`mail_sync` first when the very latest mail is needed.

| Tool | Params | Returns / behaviour |
|---|---|---|
| `mail_status` | `{}` | Which accounts are configured, verified and connected. |
| `mail_sync` | `{ account?, folder? }` | Downloads new mail into the cache (default `INBOX`). |
| `mail_list` | `{ account?, folder?, page? }` | Up to 60 cached messages: subject, sender, date, `seen`. |
| `mail_search` | `{ account?, folder?, query }` | Cached subject/sender/body search; omit `folder` to search all. |
| `mail_read` | `{ account?, folder?, id }` | One full message: subject, from/to/cc, date, plain body (+HTML), attachment names. |
| `mail_send` | `{ account?, to[], cc?, bcc?, subject, body }` | Sends via the account's SMTP; always goes to the server. |

## Guidance

- Find a specific email with `mail_search` before `mail_read`.
- `id` is the message id returned by `mail_list`/`mail_search`.
- The sender address is the account's email.

## Registration

Wrapped with `bridged(..)` in [`src/plugin.rs`](../src/plugin.rs); the skill is
[`skills/mail.md`](../skills/mail.md).
