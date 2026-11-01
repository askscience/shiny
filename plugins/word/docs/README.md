# Word — OpenDocument word processor

The `word` plugin is Shiny's word processor. It stores every document
server-side as a real **OpenDocument Text (`.odt`)** file and edits it through a
flat, editorial window. It imports Word (`.docx`), RTF, Markdown, HTML and text
files, and exports **`.docx`** and **PDF** composed with a professional design:
a theme, a cover page, a table of contents with page numbers, a running header
and page numbers. The AI talks to it with the `doc_*` tools; the window talks to
it over `/api/documents`.

The document engine is [WordCraft](https://github.com/storytold/wordcraft)'s
model, codecs, layout and PDF writer, used as libraries (see
[NOTICE](../NOTICE.md) and [architecture.md](architecture.md#engine-srcenginers)).

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
| Engine | `src/engine.rs` (WordCraft `doc`, `docx`, `formats`, `layout`, `pdf`) |
| Imports | `.odt`, `.docx`, `.rtf`, `.md`, `.html`, `.txt` |
| Exports | `.docx`, `.pdf`, `.odt`, `.md`, `.html` |
| Web surface | `plugins/word/web/plugin.js` (prefix `word-*`) |

## What it adds

- **Ten agent tools** — `doc_create`, `doc_write`, `doc_edit`, `doc_append`,
  `doc_read` (aliases `doc_open`), `doc_list`, `doc_delete`, `doc_design`,
  `doc_export`, `doc_insert_image`. Content is Markdown, tables included;
  tools without `doc_id` act on the document open in the window.
- **Twelve REST routes** for CRUD, multi-format import, export in any format,
  the design, the rendered page preview and the document fonts.
- **A page-faithful Word window** ([window.md](window.md)): white paper at its
  real size set in the document's theme, rulers with margin handles, cover /
  contents / header previews, page guides, an exact **Preview** of the
  exported pages, an outline pane, a formatting row (styles, fonts, sizes,
  colours, alignment, lists, links, pictures, tables, page breaks), a bottom
  status line with zoom, Word/PDF export, a **Design** dialog and an **AI**
  menu that asks the assistant about the open document.
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
description = "Word processor — open .odt documents with Word (.docx) and PDF export"
summary = "Word processor: write .odt documents, import Word files, export polished .docx and PDF"
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
| `shiny-plugin-sdk` (path) | `Plugin`, `RegistryBuilder`, `PluginCtx`, `bridged_route` |
| `wordcraft-doc`, `-docx`, `-formats`, `-layout`, `-pdf` (git, pinned) | Document model, `.docx`, ODT/HTML/Markdown/RTF codecs, pagination, PDF export |
| `async-trait` | `Tool`/`Plugin` async impls |
| `serde`, `serde_json` | REST bodies and tool params |
| `semver` | manifest version |
| `uuid` | document ids |
| `sqlx` (sqlite) | direct pool reads/writes in tools |
| `tokio`, `axum` | async runtime and extractors |

No runtime state crosses the `dlopen` boundary — only bytes. The workspace
`Cargo.toml` patches `krilla` to craft-libs' fork, which `wordcraft-pdf` needs.

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
row, not inside the ZIP. `migrations/002_design.sql` adds
`design TEXT NOT NULL DEFAULT '{}'`: the JSON `Design` applied on export. Every query is scoped by `user_id`, so one user can
never see another's documents.

## Source layout

```
plugins/word/
├── Cargo.toml              # cdylib + rlib, crate shiny-word-plugin
├── plugin.toml             # manifest
├── NOTICE.md               # WordCraft third-party notice
├── migrations/001_init.sql # documents table
├── migrations/002_design.sql # design column
├── migrations/003_opened.sql # opened_at (agent default target)
├── skills/word.md          # agent doc fragment (embedded via include_str!)
├── src/
│   ├── lib.rs              # pub mod engine/plugin/routes/tools
│   ├── engine.rs           # WordCraft codecs, design, compose, export, preview, pictures
│   ├── plugin.rs           # WordPlugin, route_specs(), register(), entry
│   ├── routes.rs           # REST handlers (bridge from core to plugin)
│   └── tools/mod.rs        # Tool impls + markdown→HTML helpers
└── web/
    ├── icon.svg            # ui/doc-style document glyph
    ├── plugin.js           # Word window (UI + API + AI wiring)
    └── page.js             # page model: paper, margins, theme (mirrors engine.rs)
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
  `engine::html_to_odt` / `engine::odt_to_html` on save/load.
- The AI tools send **Markdown**: `engine::markdown_to_html` reads headings,
  lists, tables, quotes, emphasis and links.
- `doc_edit` replaces at the HTML level so inline formatting elsewhere survives;
  the match is case-insensitive after an exact match fails, and a miss returns
  the current plain text (never a silent guess).
- `doc_read` returns Markdown, so a read → write round trip keeps its
  structure.
- Documents written before the engine change (SDK codec, `T1`–`T7`/`TF<n>`
  styles) open with their formatting.

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
| `doc_design` | `doc_layout`, `format_document`, `style_document` | Theme, cover, contents, header, page numbers |
| `doc_export` | `export_document`, `save_document_as`, `doc_save_as` | Save as .docx/.pdf/.odt/.md/.html to Documents |
| `doc_insert_image` | `insert_image`, `doc_add_image`, `add_picture` | Put a picture from the user's files (e.g. `generate_image`'s) into a document |

Full parameter/return/error detail is in [tools.md](tools.md).

## REST routes (summary)

| Method | Path | Auth | Handler tag | Purpose |
|---|---|---|---|---|
| `GET` | `/api/documents` | auth | `doc_list` | List documents |
| `POST` | `/api/documents` | auth | `doc_create` | Create from HTML |
| `POST` | `/api/documents/import` | auth | `doc_import` | Upload a document (any import format) |
| `GET` | `/api/documents/:id` | auth | `doc_get` | Read as HTML |
| `PUT` | `/api/documents/:id` | auth | `doc_save` | Save title/HTML |
| `DELETE` | `/api/documents/:id` | auth | `doc_delete` | Delete |
| `GET` | `/api/documents/:id/export` | auth | `doc_export` | Download, `?format=docx\|pdf\|odt\|md\|html` |
| `PUT` | `/api/documents/:id/design` | auth | `doc_design` | Set the design |
| `POST` | `/api/documents/:id/opened` | auth | `doc_opened` | The window opened it (agent default target) |
| `GET` | `/api/documents/:id/pages` | auth | `doc_pages` | Page sizes of the composed document |
| `GET` | `/api/documents/:id/page/:n` | auth | `doc_page` | One page as PNG (`?scale=`) |
| `GET` | `/api/word-fonts/:name` | public | `doc_font` | The bundled document faces |

Full request/response detail is in [routes.md](routes.md).

## More

- [architecture.md](architecture.md) — source map, document model, ODT codec
- [tools.md](tools.md) — every agent tool
- [routes.md](routes.md) — every REST route
- [window.md](window.md) — the `web/plugin.js` surface
- [skill source](../skills/word.md) — what the agent is told
