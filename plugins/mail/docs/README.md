# Mail plugin

An IMAP + SMTP mail client with a local cache. The Mail window configures
accounts and reads mail; the AI uses the `mail_*` tools. Mail is cached locally
so list/read/search are instant.

| | |
|---|---|
| Plugin name | `mail` |
| Category | `Office` |
| Version / API level | `0.1.0` / `1` |
| Crate | `shiny-mail-plugin` (`libshiny_mail_plugin.so`) |
| Database tables | `mail_accounts`, `mail_messages`, `mail_sync_state` |
| Web surface | `plugins/mail/web/plugin.js` (prefix `mail-*`) |
| Depends on | `io-email` (IMAP/SMTP) |

## What it adds

- The `mail_*` tools ([tools.md](tools.md)).
- 14 `/api/mail/*` routes ([routes.md](routes.md)).
- The Mail window: accounts, folders, message list, reader/composer
  ([window.md](window.md)).
- A **"New mail"** notification when the unread count grows.

## Credentials

Account passwords are **encrypted at rest** (AES-256-GCM, stored as
`enc:v1:<nonce||ciphertext>`). The 32-byte key is generated on first use at
`data/mail.key` (mode 0600) or at `SHINY_MAIL_KEY_FILE` when set. Rows written
before encryption existed are migrated when the account is loaded and once at
plugin load, so no manual step is needed. Responses never contain the
password, and losing the key file means the stored passwords must be entered
again.

## Caching

`src/cache.rs` stores messages locally; the first `list`/`search` of a folder
downloads it automatically. `mail_sync` pulls new mail. This is why the tools
read "instantly, no network" and why `mail_sync` exists.

## Source layout

```
plugins/mail/
├── plugin.toml
├── skills/mail.md
├── migrations/{001_init.sql,002_cache.sql}
├── src/{lib,plugin,routes,tools/mod,mail,cache,crypto}.rs
└── web/{plugin.js,icon.svg}
```

## Related

[tools](tools.md) · [routes](routes.md) · [window](window.md) ·
[plugin system](../../../docs/plugins/README.md).
