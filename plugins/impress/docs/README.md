# Impress — OpenDocument presentations

The `impress` plugin is Shiny's presentation builder. A deck is a JSON array of
slides (layout + text fields); decks export as real **OpenDocument Presentation
(`.odp`)** via the SDK codec
[`crates/shiny-plugin-sdk/src/odp.rs`](../../../crates/shiny-plugin-sdk/src/odp.rs),
which writes transitions as ODF `presentation:transition-style`.

Part of the OpenDocument office suite with [`word`](../../word/docs/README.md)
(`.odt`) and [`calc`](../../calc/docs/README.md) (`.ods`).

| | |
|---|---|
| Plugin name | `impress` |
| Category | `Office` |
| Version / API level | `0.1.0` / `1` |
| Crate | `shiny-impress-plugin` (`libshiny_impress_plugin.so`) |
| Database table | `presentations` |
| File format | OpenDocument Presentation (`application/vnd.oasis.opendocument.presentation`) |
| Codec | `crates/shiny-plugin-sdk/src/odp.rs` |
| Web surface | `plugins/impress/web/plugin.js` (prefix `impress-*`) |

## Slide model

- `layout`: `title`, `section`, `content`, `two-column`, `quote`, `blank`
  (default `content`).
- `title`, `subtitle`, `bullets[]`, `columns[][]`, `body`, `attribution`,
  `notes`.
- `transition`: `none` (default), `fade`, `slide`, `push`, `zoom`.
- `reveal`: `all` (default) or `bullets` (one bullet per advance — a build).
- Themes: `aurora` (default), `slate`, `ocean`, `mono`, `ember`.

`transition` is written to `.odp`; `reveal` is app-side only.

## Source layout

```
plugins/impress/
├── plugin.toml
├── skills/impress.md
├── migrations/001_init.sql     presentations
├── src/{lib,plugin,routes,tools/mod}.rs
└── web/{plugin.js,icon.svg}
```

## Related

[tools](tools.md) · [routes](routes.md) · [window](window.md) ·
[word](../../word/docs/README.md) · [calc](../../calc/docs/README.md) ·
[plugin system](../../../docs/plugins/README.md).
