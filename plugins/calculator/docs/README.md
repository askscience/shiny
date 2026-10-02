# Calculator plugin

A **self-contained** plugin that adds basic and scientific math to Shiny: a
Calculator window (keypad, scientific pad, shared history) and three agent
tools backed by one dependency-free expression evaluator. The AI tool and the
window call the **same** Rust engine (`src/eval.rs`), so the spoken answer and
the on-screen result are always identical.

Category: **Office**. See the [plugin docs index](../../../docs/plugins/README.md).

---

## Manifest

`plugins/calculator/plugin.toml`:

```toml
name = "calculator"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
description = "Scientific calculator — evaluate arithmetic and scientific expressions in the Calculator window"
summary = "Calculator: basic and scientific math"
author = "shiny"
skills_dir = "skills"
web_dir = "web"
category = "Office"
```

| Field | Value | Notes |
|---|---|---|
| `name` | `calculator` | Install dir + tool prefix. |
| `version` | `0.1.0` | Surfaced by `GET /api/plugins`. |
| `api_level` | `1` | Must be ≤ `CORE_API_LEVEL`. |
| `entry_symbol` | `shiny_plugin_entry` | The exported `#[no_mangle] extern "C"` symbol. |
| `migrations_dir` | *(absent → `migrations`)* | SQL files applied at load. |
| `skills_dir` | `skills` | `skills/calculator.md` is injected into the system prompt. |
| `web_dir` | `web` | `web/plugin.js` + `web/icon.svg` are served to the frontend. |
| `category` | `Office` | Tray grouping hint. |

`Manifest` is also built in `src/plugin.rs` (`CalculatorPlugin::manifest`).

---

## What it adds

- **Persona fragment** — `"a calculator AI; evaluate arithmetic and scientific math expressions for the user"` (`PERSONA`, `src/plugin.rs`).
- **Skills markdown** — `skills/calculator.md` via `builder.skills(include_str!(...))`.
- **Context line** — `"Calculator: enabled — the Calculator window evaluates basic and scientific expressions."`
- **3 tools** — `calculator_eval`, `calculator_history`, `calculator_clear_history` (registered with `bridged(...)`).
- **3 REST routes** — `POST /api/calculator/eval`, `GET /api/calculator/history`, `DELETE /api/calculator/history`.
- **Window** — `web/plugin.js` (Calculator tile), icon `web/icon.svg` (mapped to shared `apps/calculator` in `web/js/pluginIcon.js`).
- **1 table** — `calculator_history`.

---

## Dependencies

`plugins/calculator/Cargo.toml`:

| Crate | Why |
|---|---|
| `shiny-plugin-sdk` | `Plugin`/`Tool` traits, routes, `PluginCtx`, bridged runtime. |
| `async-trait` | `#[async_trait]` for the `Plugin`/`Tool` impls. |
| `serde`, `serde_json` | Tool/route request + response JSON. |
| `semver` | Manifest version. |
| `sqlx` (`runtime-tokio`, `sqlite-unbundled`) | Tool DB writes via `ctx.pool()`. |
| `tokio` (`full`) | Async runtime for the tool path. |
| `axum` (`json`, `query`) | Route extractors + responses. |

There is **no dependency** for math — `src/eval.rs` has no external crates.

---

## Database

`migrations/001_init.sql`:

```sql
CREATE TABLE IF NOT EXISTS calculator_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    expression TEXT NOT NULL,
    result TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_calculator_history_user ON calculator_history(user_id, id DESC);
```

| Column | Type | Notes |
|---|---|---|
| `id` | INTEGER PK | Auto-increment, ordering key. |
| `user_id` | TEXT | Owner; both tools and routes filter by it. |
| `expression` | TEXT | The evaluated source expression. |
| `result` | TEXT | Formatted result (`format_number`). Stored as text, not a float. |
| `created_at` | TEXT | `datetime('now')` default. |

The migration is idempotent. Applying it again creates nothing.

---

## Source layout

```
plugins/calculator/
├── Cargo.toml
├── plugin.toml
├── migrations/001_init.sql
├── skills/calculator.md        # LLM-facing tool contract
├── src/
│   ├── lib.rs                  # module exports
│   ├── plugin.rs               # Manifest, register(), routes, entry symbol
│   ├── eval.rs                 # tokenizer + recursive-descent parser (no deps)
│   ├── routes.rs               # 3 REST handlers (synchronous ctx.db())
│   └── tools/mod.rs            # 3 tools (async ctx.pool())
└── web/
    ├── plugin.js               # Calculator window
    └── icon.svg                # 24×24 currentColor icon
```

### Source map

| Path | Role |
|---|---|
| `src/lib.rs` | Declares `eval`, `plugin`, `routes`, `tools`; re-exports `CalculatorPlugin`. |
| `src/plugin.rs` | `Manifest`, `PERSONA`, `route_specs()`, `register()`, `route_handler()`, `shiny_plugin_entry`. |
| `src/eval.rs` | `evaluate`, `format_number`, tokenizer, `Parser`, `call_function`, `factorial`, constants. |
| `src/routes.rs` | `handle(tag)` → `eval` / `history_list` / `history_clear`. |
| `src/tools/mod.rs` | `CalculatorEval`, `CalculatorHistory`, `CalculatorClearHistory`. |
| `skills/calculator.md` | Advertises the JSON contract + supported syntax to the LLM. |
| `web/plugin.js` | Mounts the tile, keypad, sci pad, history, and AI wiring. |

---

## Build / install

Build for the running platform (all plugins build with the workspace):

```bash
cargo build --release -p shiny-calculator-plugin
# → target/release/libshiny_calculator_plugin.so  (Linux)
```

Package and install through the admin API:

```bash
mkdir -p /tmp/pkg/calculator/migrations /tmp/pkg/calculator/skills
cp plugins/calculator/plugin.toml /tmp/pkg/calculator/
cp plugins/calculator/migrations/001_init.sql /tmp/pkg/calculator/migrations/
cp plugins/calculator/skills/calculator.md /tmp/pkg/calculator/skills/
cp -r plugins/calculator/web /tmp/pkg/calculator/
cp target/release/libshiny_calculator_plugin.so /tmp/pkg/calculator/
( cd /tmp/pkg && zip -r calculator.zip calculator )
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $TOKEN" -F "file=@/tmp/pkg/calculator.zip"
```

### Dev notes

- `node --check plugins/calculator/web/plugin.js` catches most JS syntax errors; core only logs a `console.warn` if `import()` fails, leaving the window silently absent.
- The app serves `data/plugins/calculator/web/` (the installed copy). After editing `plugins/calculator/web/plugin.js` or `icon.svg`, copy it to `data/plugins/calculator/web/` or reinstall.
- The migration file is immutable once applied — add a new higher-numbered file instead of editing `001_init.sql`.
- `src/eval.rs` has unit tests (`basic_arithmetic`, `scientific`, `rejects_bad_input`): `cargo test -p shiny-calculator-plugin`.

---

## Tools summary

| Tool | Aliases | Step label | Params | Returns |
|---|---|---|---|---|
| `calculator_eval` | `calculate`, `compute`, `evaluate` | `Calculating…` | `{ expression: string }` (alias `expr`) | `{ expression, result, result_text }` |
| `calculator_history` | `calculation_history`, `calc_history` | `Reading calculator history…` | `{ limit?: number }` | `{ history: [{expression,result,at}], count }` |
| `calculator_clear_history` | `clear_calculator_history`, `clear_calc_history` | `Clearing calculator history…` | `{}` | `{ cleared: true }` |

Full detail in [tools.md](./tools.md).

## Routes summary

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `POST` | `/api/calculator/eval` | auth | Evaluate + persist one expression. |
| `GET` | `/api/calculator/history` | auth | List recent calculations (`?limit=`). |
| `DELETE` | `/api/calculator/history` | auth | Clear the caller's history. |

Full detail in [routes.md](./routes.md).
