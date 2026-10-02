# Mail — REST routes

All routes are `auth`, registered in [`src/plugin.rs`](../src/plugin.rs).

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/mail/status` | Account/connection summary. |
| `GET` | `/api/mail/accounts` | List accounts. |
| `POST` | `/api/mail/accounts` | Create an account. |
| `POST` | `/api/mail/accounts/test` | Test credentials/connection. |
| `PUT` | `/api/mail/accounts/:id` | Update an account. |
| `DELETE` | `/api/mail/accounts/:id` | Delete an account. |
| `GET` | `/api/mail/folders` | List folders. |
| `GET` | `/api/mail/list` | Cached messages in a folder. |
| `GET` | `/api/mail/search` | Search the cache. |
| `POST` | `/api/mail/sync` | Sync a folder. |
| `GET` | `/api/mail/message` | One full message. |
| `POST` | `/api/mail/send` | Send. |
| `POST` | `/api/mail/flag` | Flag/read state. |
| `POST` | `/api/mail/delete` | Delete. |

Responses use `{ "success": true, "data": … }`. `:id` arrives via the
`x-shiny-path-params` header.
