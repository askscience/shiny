# Word — OpenDocument word processor

The `word` plugin is Shiny's simple word processor. It stores every document
server-side as a real **OpenDocument Text (`.odt`)** file and edits it through a
flat, editorial window. The AI talks to it with the `doc_*` tools; the window
talks to it over `/api/documents`.

It is part of the OpenDocument office suite alongside
[`calc`](../../calc/docs/README.md) (`.ods` spreadsheets) and
[`impress`](../../impress/docs/README.md) (`.odp` presentations). See the
[plugins overview](../../../docs/plugins/README.md).

## At a glance

| | |
|---|---|
| Plugin name | `word` |
| Category | `Office` |
| Version / API level | `0.1.0` / `1` |
| Entry symbol | `shiny_plugin_entry` |
| Crate | `shiny-word-plugin` (`libshiny_word_plugin.so`) |
| Database table | `documents` (owned by this plugin) |
| File format | OpenDocument Text — `application/vnd.oasis.opendocument.text` |
| Codec | `crates/shiny-plugin-sdk/src/odt.rs` |
| Web surface | `plugins/word/web/plugin.js` (prefix `word-*`) |

## What it adds

- **Seven agent tools** — `doc_create`, `doc_write`, `doc_edit`, `doc_append`,
  `doc_read` (aliases `doc_open`), `doc_list`, `doc_delete`.
- **Seven REST routes** under `/api/documents` for CRUD, `.odt` import and
  `.odt` export.
- **A Word window** with a `contentEditable` editor, bold/italic/underline,
  headings, bullet lists, a host-font picker, autosave and a document menu.
- **A persona fragment** — *"a writer's assistant; draft, edit and manage the
  user's documents"* — plus the embedded [skill](../skills/word.md) that teaches the
  agent the `doc_*` contract.
- **A context line** shown to the model while the plugin is enabled.

## Manifest

`plugins/word/plugin.toml`:

```toml
name = "word"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
description = "Simple word processor — documents stored as open .odt files"
summary = "Word processor: create, edit and read OpenDocument (.odt) documents"
author = "shiny"
skills_dir = "skills"
web_dir = "web"
category = "Office"
```

`migrations_dir` is omitted and therefore defaults to `"migrations"`. The Rust
`Manifest` in `src/plugin.rs` mirrors these fields; `target_triple` and
`signature` are `None`.

## Dependencies

| Dependency | Use |
|---|---|
| `shiny-plugin-sdk` (path) | `Plugin`, `RegistryBuilder`, `PluginCtx`, `odt` codec, `bridged_route` |
| `async-trait` | `Tool`/`Plugin` async impls |
| `serde`, `serde_json` | REST bodies and tool params |
| `semver` | manifest version |
| `uuid` | document ids |
| `sqlx` (sqlite) | direct pool reads/writes in tools |
| `tokio`, `axum` | async runtime and extractors |

The core binary also links `odt` directly because it serves this plugin's
routes; no runtime state crosses the `dlopen` boundary — only bytes.

## Database

`plugins/word/migrations/001_init.sql`:

```sql
CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES travelers(id),
    title TEXT NOT NULL DEFAULT 'Untitled',
    odt BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_documents_user ON documents(user_id, updated_at DESC);
```

The `odt` column holds the complete `.odt` ZIP bytes. The title lives in the
row, not inside the ZIP. Every query is scoped by `user_id`, so one user can
never see another's documents.

## Source layout

```
plugins/word/
├── Cargo.toml              # cdylib + rlib, crate shiny-word-plugin
├── plugin.toml             # manifest
├── migrations/001_init.sql # documents table
├── skills/word.md          # agent doc fragment (embedded via include_str!)
├── src/
│   ├── lib.rs              # pub mod plugin/routes/tools
│   ├── plugin.rs           # WordPlugin, route_specs(), register(), entry
│   ├── routes.rs           # REST handlers (bridge from core to plugin)
│   └── tools/mod.rs        # Tool impls + markdown→HTML helpers
└── web/
    ├── icon.svg            # ui/doc-style document glyph
    └── plugin.js           # Word window (UI + API + AI wiring)
```

## Build & install

The crate is a workspace member, so it builds with the rest of the tree:

```sh
cargo build -p shiny-word-plugin            # debug
cargo build --release -p shiny-word-plugin   # release
# -> target/{debug,release}/libshiny_word_plugin.so
```

Package `plugin.toml`, `skills/`, `migrations/` and the `.so` into an archive
and install it at runtime (no restart):

```sh
mkdir -p /tmp/pkg/word/skills /tmp/pkg/word/migrations
cp plugins/word/plugin.toml /tmp/pkg/word/
cp plugins/word/skills/word.md /tmp/pkg/word/skills/
cp plugins/word/migrations/001_init.sql /tmp/pkg/word/migrations/
cp target/release/libshiny_word_plugin.so /tmp/pkg/word/
tar -C /tmp/pkg -czf /tmp/word.tar.gz word
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $TOKEN" -F file=@/tmp/word.tar.gz
```

At boot the loader discovers `plugin.toml`, validates `api_level <= 1`, runs the
migrations, `dlopen`s the cdylib and calls `shiny_plugin_entry()`.

### Dev notes

- The window sends **HTML**, not ODT: the server converts with
  `odt::html_to_odt` / `odt::odt_to_html` on save/load.
- The AI tools send **plain text**: `content_to_html` turns lines into `<p>`,
  `#`/`##`/`###` into headings and `**bold**`/`*italic*` into `<b>`/`<i>`.
- `doc_edit` replaces at the HTML level so inline formatting elsewhere survives;
  the match is case-insensitive after an exact match fails, and a miss returns
  the current plain text (never a silent guess).
- `doc_read` returns `odt_to_plain_text` — one line per paragraph — so a
  read → write round-trip keeps its structure.
- The codec's style names are `T1`–`T7` (bold/italic/underline masks) and
  `TF<n>` for fonts; see [architecture.md](architecture.md).

## Agent tools (summary)

| Tool | Aliases | Purpose |
|---|---|---|
| `doc_create` | `create_document`, `new_document` | New document, optional content |
| `doc_write` | `write_document`, `update_document` | Replace entire content |
| `doc_edit` | `edit_document`, `modify_document`, `change_document`, `replace_text` | Replace first `old`→`new` |
| `doc_append` | `append_text`, `add_to_document` | Append content at the end |
| `doc_read` | `read_document`, `open_document`, `doc_open` | Read plain text |
| `doc_list` | `list_documents`, `documents` | List documents |
| `doc_delete` | `delete_document`, `remove_document` | Delete a document |

Full parameter/return/error detail is in [tools.md](tools.md).

## REST routes (summary)

| Method | Path | Auth | Handler tag | Purpose |
|---|---|---|---|---|
| `GET` | `/api/documents` | auth | `doc_list` | List documents |
| `POST` | `/api/documents` | auth | `doc_create` | Create from HTML |
| `POST` | `/api/documents/import` | auth | `doc_import` | Upload a `.odt` |
| `GET` | `/api/documents/:id` | auth | `doc_get` | Read as HTML |
| `PUT` | `/api/documents/:id` | auth | `doc_save` | Save title/HTML |
| `DELETE` | `/api/documents/:id` | auth | `doc_delete` | Delete |
| `GET` | `/api/documents/:id/export` | auth | `doc_export` | Download `.odt` |

Full request/response detail is in [routes.md](routes.md).

## More

- [architecture.md](architecture.md) — source map, document model, ODT codec
- [tools.md](tools.md) — every agent tool
- [routes.md](routes.md) — every REST route
- [window.md](window.md) — the `web/plugin.js` surface
- [skill source](../skills/word.md) — what the agent is told
