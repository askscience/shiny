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
├── src/{lib,plugin,routes,tools/mod,mail,cache}.rs
└── web/{plugin.js,icon.svg}
```

## Related

[tools](tools.md) · [routes](routes.md) · [window](window.md) ·
[plugin system](../../../docs/plugins/README.md).
