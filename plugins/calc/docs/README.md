# Calc — OpenDocument spreadsheets

The `calc` plugin is Shiny's spreadsheet. Sheets are a grid of **A1-addressed
cells** stored as a JSON map (`"A1" -> "value"`), with `=`-prefixed formulas
evaluated live in the window. Real **`.ods` import/export** uses the SDK codec
[`crates/shiny-plugin-sdk/src/ods.rs`](../../../crates/shiny-plugin-sdk/src/ods.rs).

Part of the OpenDocument office suite with [`word`](../../word/docs/README.md)
(`.odt`) and [`impress`](../../impress/docs/README.md) (`.odp`).

| | |
|---|---|
| Plugin name | `calc` |
| Category | `Office` |
| Version / API level | `0.1.0` / `1` |
| Crate | `shiny-calc-plugin` (`libshiny_calc_plugin.so`) |
| Database table | `spreadsheets` |
| File format | OpenDocument Spreadsheet (`application/vnd.oasis.opendocument.spreadsheet`) |
| Codec | `crates/shiny-plugin-sdk/src/ods.rs` |
| Web surface | `plugins/calc/web/plugin.js` (prefix `calc-*`) |

## What it adds

- The `calc_*` agent tools ([tools.md](tools.md)).
- The `/api/spreadsheets` REST routes ([routes.md](routes.md)).
- The Calc window (grid editor, formula bar, `.ods` import/export).

## Source layout

```
plugins/calc/
├── plugin.toml
├── skills/calc.md
├── migrations/001_init.sql     spreadsheets
├── src/{lib,plugin,routes,tools/mod}.rs
└── web/{plugin.js,icon.svg}
```

## Import / export

The window imports/exports `.ods` through `/api/spreadsheets/import` and
`/:id/export`. When the [Files plugin](../../files/docs/README.md) is installed,
save/export writes into the user's home via
[`web/js/files.js`](../../../web/js/files.js) `saveOrDownload`; otherwise it
falls back to a browser download.

## Related

[tools](tools.md) · [routes](routes.md) · [window](window.md) ·
[word](../../word/docs/README.md) · [impress](../../impress/docs/README.md) ·
[plugin system](../../../docs/plugins/README.md).
