# Word — architecture

This document maps the `word` plugin module by module, describes the document
model, and documents its engine (`src/engine.rs`), built on WordCraft's
libraries.

See also: [README.md](README.md) · [tools.md](tools.md) · [routes.md](routes.md)
· [window.md](window.md) · [plugins overview](../../../docs/plugins/README.md).

## Module-by-module source map

| File | Items | Responsibility |
|---|---|---|
| `src/lib.rs` | `pub mod engine/plugin/routes/tools`, `pub use plugin::WordPlugin` | Crate facade |
| `src/engine.rs` | `Format`, `import()`, content codecs, `Design`, `compose()`, `export()`, `Preview`, `FONT_FILES`/`font_file()`, `picture_html()`, `insert_after_block()` | WordCraft model, codecs, composition, page rendering, pictures |
| `src/plugin.rs` | `WordPlugin`, `PERSONA`, `route_specs()`, `register()`, `route_handler()`, `shiny_plugin_entry()` | Manifest, route registration, tool registration, handler dispatch |
| `src/routes.rs` | `handle()`, `as_blob()`, `take_path()`, `take_query()`, `export_document()`, eight handlers | REST surface — a bridge that turns core HTTP requests into DB + engine work |
| `src/tools/mod.rs` | `content_to_html()`, `find_ci()`, `last_doc_id()`, `target_doc()`, `DocCreate/Write/Edit/Append/Read/List/Delete/Design/Export` | Agent tool surface |
| `web/plugin.js` | window, API, editor, menu, AI wiring | Front end |
| `migrations/001_init.sql`, `002_design.sql`, `003_opened.sql` | `documents` table, `design` and `opened_at` columns | Storage |
| `web/page.js` | `themeVars()`, `geometry()`, `installDocumentFonts()` | The window's page model (mirrors the engine) |
| `skills/word.md` | agent instructions | Embedded into the prompt by `register()` |

### `WordPlugin` lifecycle

`WordPlugin` holds a `OnceLock<Arc<PluginCtx>>` set on first `register()`.
`register()`:

1. stores the `PluginCtx`,
2. appends the persona (`PERSONA`), the embedded skill and a context line,
3. registers each `RouteSpec` from `route_specs()`,
4. registers the nine tools, each wrapped in `tools::bridged(tool)`.

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
PUT    /api/documents/:id/design   doc_design
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
| `odt` | `BLOB` | complete `.odt` ZIP bytes (the content) |
| `design` | `TEXT` | JSON `engine::Design`, default `{}` (plain) |
| `created_at` / `updated_at` | `TEXT` | SQLite `datetime('now')` |

Every conversion goes through WordCraft's `Document` model. At the HTML layer
the document is a body fragment with inline CSS (`<p>`, `<h1>`…`<h6>`,
`<ul>`/`<ol>`, `<table>`, `<blockquote>`, `<strong>`, `<em>`, `<u>`, `<a>`,
`<span style="font-family:…;color:…">`). At the AI layer it is Markdown. The
conversions:

```
AI Markdown ──markdown_to_doc──▶ Document ──export_fragment──▶ editor HTML
editor HTML ──html::import──▶ Document ──odt::export──▶ .odt bytes ──▶ DB
DB ──▶ .odt bytes ──odt::import──▶ Document ──▶ HTML (window) / Markdown (doc_read)
                                      └──compose(design)──▶ .docx / .pdf / .odt / .md / .html
```

`doc_edit`/`doc_append` operate on the HTML so existing formatting and fonts
survive a targeted change.

### Title handling

`clean_title()` in `routes.rs` trims, defaults empty to `Untitled`, and caps at
120 chars. `filename_for_title()` sanitises to `[A-Za-z0-9 ._-]`, trims dots and
returns `<name>.odt` (falling back to `document.odt`). Export replaces `"` in
the Content-Disposition header.

## Engine (`src/engine.rs`)

The engine wraps five WordCraft crates (MIT OR Apache-2.0, git dependencies
pinned to one commit, see [NOTICE](../NOTICE.md)):

| Crate | Used for |
|---|---|
| `wordcraft-doc` | The document model: paragraphs, runs, styles, numbering, tables, sections, header/footer parts, fields |
| `wordcraft-formats` | ODT, HTML, Markdown, RTF and text import/export |
| `wordcraft-docx` | `.docx` read and write |
| `wordcraft-layout` | Pagination, for the table of contents' page numbers |
| `wordcraft-pdf` | PDF export (embedded subset fonts, selectable text, links, outline, tagged) |
| `wordcraft-render` | Page rasters for the window's Preview (CPU, `vello_cpu` 0.2 from crates.io — the workspace's patched 0.0.8 fork serves PdfCraft only) |
| `wordcraft-fonts` | The bundled faces, served to the window |

`wordcraft-pdf` writes through craft-libs' `craft-pdf`, which needs its patched
`krilla`; the workspace `Cargo.toml` applies that `[patch]` (only the root of
a build can).

### Formats and import

`Format::parse` maps extensions and names (`docx`, `pdf`, `markdown`, …) to
formats; `Format::mime`/`ext` drive export headers and file names.
`import(name, bytes)` picks the format from the extension, or sniffs ZIP
packages (ODT by its `mimetype`, otherwise DOCX), RTF and HTML; PDF is
refused. Readers are WordCraft's: lenient, depth-limited and size-capped, so a
hostile file fails cleanly.

### Content codecs

`html_to_odt`, `odt_to_html`, `odt_to_markdown`, `markdown_to_html` and their
`Document` halves. Documents written by the old SDK codec (`T1`–`T7`, `TF<n>`
automatic styles) read back with their formatting (test
`reads_documents_written_by_the_old_codec`). `separate_lines` makes the
agent's one-paragraph-per-line habit valid Markdown.

### Composition

`Design` (stored as JSON) says how `compose(content, title, design)` builds the
exported document. The editor never holds the composed parts: they are
regenerated from the current content on every export.

| Option | Effect |
|---|---|
| `template` | Theme: `classic` (Source Serif 4, navy, centred title, ruled Heading 1), `modern` (Source Sans 3, blue), `minimal` (Source Sans 3, black/grey); restyles Normal, Heading 1–6, Title, Subtitle, Quote and a `ShinyTable` table style (accent header row, light banding) applied to unstyled tables. Empty/`plain` keeps the content's own styles. |
| `paper` | A4 (default) or Letter |
| `margin_top`, `margin_right`, `margin_bottom`, `margin_left` | Millimetres (5–80); 0 = 25 mm (`Design::margins_pt`) |
| `cover` | Title (accent rule), subtitle, author and date on page 1, with empty first-page header/footer (`title_page`) |
| `toc` | A contents heading (localised for the 26 UI languages via `lang`) and one entry per Heading 1–3, with dot leaders and page numbers |
| `header` | Title left, subtitle (or author) right, thin accent rule |
| `page_numbers` | `PAGE / NUMPAGES` fields in the footer |
| `subtitle`, `author`, `date` | Cover/title block and document properties (`core.subject`, `core.creator`) |
| `lang` | Text language (hyphenation, labels) |
| `accent` | `#RRGGBB` override of the theme colour |

Without a cover, a themed document starts with a Title block. A first heading
repeating the title is dropped. Content after a cover or contents starts on a
new page.

**Contents page numbers** are real: `compose` inserts the entries with empty
numbers, lays the document out once (`wordcraft_layout::layout`), reads each
heading's page with `DocLayout::page_of` and writes it into its entry. The
entries are static text, so they are right in every format, including `.docx`
opened in Word (where no field update is needed).

### Preview

`Preview::new(content, title, design)` composes and lays the document out
once; `pages()` lists page sizes and `page_png(i, scale)` renders one page
with `wordcraft_render::render_page` and encodes it as PNG (`image` crate).
The routes cache a few previews per (document, `updated_at`, title, design).

### Pictures

`picture_html(bytes, alt, width_pct)` decodes any PNG/JPEG/WebP/GIF/BMP with
the `image` crate, keeps small PNG/JPEG byte for byte and otherwise scales to
1600 px and re-encodes (JPEG 86, or PNG with alpha), returning an `<img>` with
a `data:` URI and a width capped at 600 px × `width_pct`.
`insert_after_block(html, after, block)` finds the block holding `after`
(ASCII case folding, so byte offsets stay aligned) and inserts after it, or
after the enclosing list/table.

### Export

`export(doc, format)`: `.docx` via `wordcraft_docx::write`, `.pdf` via
`wordcraft_pdf::export` (title and author in the metadata), `.odt`, `.md`,
`.html`, `.rtf`, `.txt` via `wordcraft_formats`. Fonts: themes use faces
WordCraft bundles, so PDF output is identical on every machine; a `.docx`
names them and Word substitutes when they are not installed.

### Tests

`cargo test -p shiny-word-plugin --lib engine` covers Markdown and ODT round
trips (tables, fonts), old-codec documents, composition with a contents page
and every export format, `.docx` read-back, sniffing and validation.
`WORD_SAMPLE_DIR=… cargo test -p shiny-word-plugin --lib write_samples --
--ignored` writes a sample in every theme as `.pdf` and `.docx` to look at.

## Cross-links

- [WordCraft](https://github.com/storytold/wordcraft) · [NOTICE](../NOTICE.md)
- [SDK ODT codec](../../../crates/shiny-plugin-sdk/src/odt.rs) (still used by the Files preview)
- [SDK overview](../../../crates/shiny-plugin-sdk/src/lib.rs)
- [Agent injection fallback](../../../PLUGINS.md)
