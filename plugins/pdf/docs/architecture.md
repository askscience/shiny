# PDF — architecture

A self-contained plugin that stores **real `.pdf` bytes** server-side and edits
them with a mix of `pdf_oxide` and `lopdf`. The same engine powers both the
agent tools and the window, so the AI and the human have identical capability.

```
plugins/pdf/
├── plugin.toml
├── skills/pdf.md
├── migrations/001_init.sql     pdf_documents
├── src/
│   ├── lib.rs                  module wiring
│   ├── plugin.rs               manifest, routes, tools, entry
│   ├── routes.rs               /api/pdfs/* handlers
│   ├── ops.rs                  the single operations engine (shared by tools + routes)
│   ├── stream_edit.rs          in-place content-stream text editing (lopdf)
│   └── tools/mod.rs            12 pdf_* agent tools
└── web/plugin.js               the PDF window (pdf.js)
```

## Engine split

| Concern | Crate |
|---|---|
| Rendering to PNG, creation, merge, rotate, annotate, watermark, text extraction, `match_text` | **`pdf_oxide`** |
| In-place text replacement on existing content streams | **`lopdf`** (`stream_edit.rs`) |

`ops.rs` is the single entry point both call sites use. Creation goes through
pdf_oxide's HTML+CSS engine (bundled DejaVu fonts) so long text wraps instead of
running off the page; the plugin also ships a reliable built-in Base-14
semantic-HTML renderer (the skill documents this).

## Why `lopdf` for text replacement

`pdf_oxide`'s `modify_text` re-emits a run **without removing the original
glyphs** — a PDF content stream is append-only — so find & replace produced old
and new text on top of each other. `stream_edit.rs` edits the raw content
stream instead. Two traps make a naive edit fail silently:

- **Text often lives in Form XObjects, not the page stream.** The engine
  recurses into the page's Form XObjects (lopdf's own `replace_text` only scans
  the page stream).
- **Text usually arrives as a `TJ` array, not one string.** Glyphs are split by
  kerning numbers (`[(T) -3 (I) 10 (M)]`); the array is flattened, edited, then
  re-emitted with a **kerning correction placed after** the string so the line
  keeps its width. (Putting it first shifted the replacement left of its margin
  — a fixed bug.)

Style changes (bold/size/colour) are **not supported** and the controls were
removed; see the root [`PLUGINS.md` §19 "PDF window"](../../../PLUGINS.md) for
the full record of why each attempted approach failed. `stream_edit::style_text`
exists and is documented but has no route.

## Data

`pdf_documents` stores `id`, `traveler_id`, `title`, bytes/BLOB and metadata.
Migrations live in `migrations/`. Uninstalling leaves the table.

## Storage & routes

The window fetches the document through `GET /api/pdfs/:id/file` (inline,
`no-store`) — deliberately separate from `/export` (attachment). See
[routes.md](routes.md).

## Related

[README](README.md) · [tools](tools.md) · [routes](routes.md) ·
[window](window.md) · [plugin system](../../../docs/plugins/README.md).
