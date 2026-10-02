# Word — architecture

This document maps the `word` plugin module by module, describes the document
model, and documents the ODT codec it shares with core
(`crates/shiny-plugin-sdk/src/odt.rs`).

See also: [README.md](README.md) · [tools.md](tools.md) · [routes.md](routes.md)
· [window.md](window.md) · [plugins overview](../../../docs/plugins/README.md).

## Module-by-module source map

| File | Items | Responsibility |
|---|---|---|
| `src/lib.rs` | `pub mod plugin/routes/tools`, `pub use plugin::WordPlugin` | Crate facade |
| `src/plugin.rs` | `WordPlugin`, `PERSONA`, `route_specs()`, `register()`, `route_handler()`, `shiny_plugin_entry()` | Manifest, route registration, tool registration, handler dispatch |
| `src/routes.rs` | `handle()`, `user_id()`, `ok()`, `clean_title()`, `filename_for_title()`, `as_text()`, `as_blob()`, `take_path()`, `take_query()`, seven handlers | REST surface — a bridge that turns core HTTP requests into DB + codec work |
| `src/tools/mod.rs` | `content_to_html()`, `inline_html()`, `find_ci()`, `last_doc_id()`, `doc_summary_json()`, `DocCreate/Write/Edit/Append/Read/List/Delete` | Agent tool surface |
| `web/plugin.js` | window, API, editor, menu, AI wiring | Front end |
| `migrations/001_init.sql` | `documents` table | Storage |
| `skills/word.md` | agent instructions | Embedded into the prompt by `register()` |

### `WordPlugin` lifecycle

`WordPlugin` holds a `OnceLock<Arc<PluginCtx>>` set on first `register()`.
`register()`:

1. stores the `PluginCtx`,
2. appends the persona (`PERSONA`), the embedded skill and a context line,
3. registers each `RouteSpec` from `route_specs()`,
4. registers the seven tools, each wrapped in `tools::bridged(tool)`.

Core calls `route_handler(tag)` per HTTP request; the returned closure is the
handler for that tag. The plugin's tools run through `ctx.pool()` (its own
`SqlitePool`), while routes use the **synchronous** `ctx.db()` because they run
on the plugin's runtime thread.

### Route specs

`route_specs()` returns exactly these specs, all `auth: "auth"`:

```rust
GET    /api/documents              doc_list
POST   /api/documents              doc_create
POST   /api/documents/import       doc_import
GET    /api/documents/:id          doc_get
PUT    /api/documents/:id          doc_save
DELETE /api/documents/:id          doc_delete
GET    /api/documents/:id/export   doc_export
```

`handle(ctx, tag)` matches the tag to a handler closure; unknown tags return
`None`. See [routes.md](routes.md).

## Document model

A document is one row in `documents`:

| Field | Type | Notes |
|---|---|---|
| `id` | `TEXT` PK | UUID v4 |
| `user_id` | `TEXT` | FK → `travelers(id)`; ownership scope |
| `title` | `TEXT` | default `Untitled`, capped at 120 chars on write |
| `odt` | `BLOB` | complete `.odt` ZIP bytes |
| `created_at` / `updated_at` | `TEXT` | SQLite `datetime('now')` |

At the HTML layer the document is a body fragment (`<p>`, `<h1>`…`<h6>`,
`<ul>`, `<b>`, `<i>`, `<u>`, `<a>`, `<span style="font-family:…">`). At the AI
layer it is plain text (one paragraph per line). The conversions:

```
AI plain text ──content_to_html──▶ editor HTML ──html_to_odt──▶ .odt bytes ──DB
                                                                     │
AI plain text ◀──odt_to_plain_text── HTML ◀──odt_to_html── .odt bytes ◀┘
```

`doc_read` uses the plain-text path; `doc_edit`/`doc_append` operate on the
HTML path so existing formatting and fonts survive a targeted change.

### Title handling

`clean_title()` in `routes.rs` trims, defaults empty to `Untitled`, and caps at
120 chars. `filename_for_title()` sanitises to `[A-Za-z0-9 ._-]`, trims dots and
returns `<name>.odt` (falling back to `document.odt`). Export replaces `"` in
the Content-Disposition header.

## ODT codec (`crates/shiny-plugin-sdk/src/odt.rs`)

A minimal, valid, dependency-light ODT codec. A `.odt` is a ZIP whose **first,
stored (uncompressed)** entry is `mimetype`, followed by
`META-INF/manifest.xml` and deflated `content.xml`.

### Public API

| Function | Direction | Notes |
|---|---|---|
| `html_to_odt(title, html) -> Result<Vec<u8>, AppError>` | HTML → `.odt` | Writes the ZIP; `title` is accepted but lives in the DB row, not the file |
| `odt_to_html(odt) -> Result<String, AppError>` | `.odt` → HTML | Parses `content.xml` with `roxmltree` |
| `odt_to_plain_text(odt) -> Result<String, AppError>` | `.odt` → text | `strip_html(odt_to_html(..))`, one line per paragraph, `- ` for list items |

Errors are `AppError::Internal` with a prefixed message: *"Not a valid .odt
file"*, *"Missing content.xml in .odt"*, *"Invalid ODT content.xml"*, or *"ODT
write failed"*.

### HTML → ODT

`convert_blocks()` runs a hand-written recursive-descent parser over the HTML
(`struct P`, fields `c`/`i`/`fonts`):

- Block tags (`p`, `div`, `blockquote`, headings, lists, tables, `pre`, …) are
  emitted as `<text:p>` / `<text:h text:outline-level="n">`; `<ul>`/`<ol>`
  become `<text:list>` with `<text:list-item>`.
- Inline `<b>`/`<strong>` → span `T1`, `<i>`/`<em>` → `T2`, `<u>` → `T3`,
  `<br>` → `<text:line-break/>`, `<a href>` → `<text:a xlink:href>`.
- Style masks combine: bold=1, italic=2, underline=4, mapped by `style_name()`
  to `T1`,`T2`,`T3`,`T4`(bold+italic),`T5`,`T6`,`T7`. Note the codec's naming:
  underline-only is `T3`, bold-italic is `T4`, bold-underline `T5`,
  italic-underline `T6`, all three `T7`.
- `font-family` (inline `style`, legacy `<font face>`, or a block default) is
  registered in first-use order and written as automatic character styles
  `TF1`, `TF2`, … via `automatic_styles()`.
- `<img>`, `<script>`, `<style>`, `<iframe>`, `<svg>`, `<video>`, `<audio>` are
  skipped entirely.
- Text is XML-escaped and whitespace-collapsed by `escape_and_collapse()`;
  literal `<` in prose is escaped so "5 < 6" survives.

### ODT → HTML

`read_content_xml()` unzips and reads `content.xml`. `collect_font_styles()`
builds a `style name → fo:font-family` map so `TF<n>` spans round-trip back to
`<span style="font-family:…">`. Then `render_block()`/`render_inline()` walk
the tree:

- `text:p` → `<p>`, `text:h` → `<h{n}>` (outline level clamped 1–6).
- `text:list` → `<ul>`/`<ol>` (ordered when its style name contains
  `"number"`), `text:list-item` → `<li>`.
- `text:span` style masks → `<b>`/`<i>`/`<u>` (`style_mask`, `style_wrap`).
- `text:a` → `<a href>`, `text:line-break` → `<br>`, `text:s` → spaces,
  `text:tab` → tab.
- Other elements are flattened by recursing into their inline content.

`strip_html()` is the plain-text reader; it keeps paragraph/list breaks and
turns list items into `- …`.

### Round-trip guarantees (tests in the codec)

`roundtrip_keeps_formatting`, `roundtrip_keeps_inline_font`,
`roundtrip_keeps_block_font_and_weight`, `roundtrip_keeps_legacy_font_tag`,
`literal_angle_brackets_are_escaped`. These lock in the heading, bold/italic,
list, font and angle-bracket behaviours the window and AI rely on.

### Limitations

- Only automatic character styles for bold/italic/underline and fonts; no
  colours, sizes (beyond heading level), alignment or images are modelled.
- `title` is not embedded in `content.xml`.
- Foreign ODT files are read as best effort: unknown inline markup is
  flattened, lists/tables may reduce to paragraphs.

## Cross-links

- [SDK ODT codec](../../../crates/shiny-plugin-sdk/src/odt.rs)
- [SDK overview](../../../crates/shiny-plugin-sdk/src/lib.rs)
- [Agent injection fallback](../../../PLUGINS.md)
