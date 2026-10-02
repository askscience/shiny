# Calculator plugin — architecture

Module-by-module source map and the key logic (the evaluator grammar, and how
the AI tool and the window share it).

## Module map

| Module | Entry points | Responsibility |
|---|---|---|
| `src/lib.rs` | `pub mod eval/plugin/routes/tools`, `pub use CalculatorPlugin` | Crate wiring. |
| `src/plugin.rs` | `CalculatorPlugin`, `PERSONA`, `route_specs()`, `register()`, `route_handler()`, `shiny_plugin_entry()` | SDK integration: manifest, tool/route registration, route dispatch. |
| `src/eval.rs` | `evaluate(&str) -> Result<f64, String>`, `format_number(f64) -> String` | The single math engine. No external crates. |
| `src/routes.rs` | `handle(ctx, tag) -> Option<RouteHandler>`, `eval`, `history_list`, `history_clear` | REST surface used by the window. DB via `ctx.db()` (synchronous). |
| `src/tools/mod.rs` | `CalculatorEval`, `CalculatorHistory`, `CalculatorClearHistory` | Agent tools. DB via `ctx.pool().await` (sqlx). |

### Why two DB paths?

Routes are served through the SDK's `bridged_route` and use the synchronous
`ctx.db()` (`libsqlite3-sys`) helper. Tools run on the plugin's own Tokio
runtime (wrapped with `bridged(...)`) and use the async `ctx.pool().await`.
Both point at the same SQLite file and the same `calculator_history` table.
See PLUGINS.md §15 for the runtime-bridge rules that make this necessary.

## `src/plugin.rs`

- `route_specs()` returns three specs, all `auth: "auth"`:
  `POST /api/calculator/eval` → `eval`, `GET /api/calculator/history` →
  `history_list`, `DELETE /api/calculator/history` → `history_clear`.
- `register()` stores `Arc<PluginCtx>` in `self.ctx` (`OnceLock`), sets the
  persona, includes `../skills/calculator.md`, adds a context line, registers
  the route specs, then registers the three tools each wrapped in
  `shiny_plugin_sdk::tools::bridged`.
- `route_handler(tag)` maps a tag to `crate::routes::handle(ctx, tag)`.

## The evaluator grammar (`src/eval.rs`)

A hand-written **tokenizer + recursive-descent parser** over `f64`. The public
`evaluate` tokenizes, parses one expression, requires the token stream to be
fully consumed, and rejects non-finite results.

### Tokens

```rust
enum Tok { Num(f64), Ident(String), Op(char) }
```

- Whitespace is skipped.
- Numbers consume digits, one optional `.`, and optional scientific notation
  (`1.5e3`, `2E-4`).
- Identifiers are `[A-Za-z_][A-Za-z0-9_]*`, lowercased before lookup.
- Operators: `+ - * / % ^ ! ( ) ,`. Any other character is `unexpected character`.

### Precedence / associativity (lowest → highest)

| Level | Function | Operators | Associativity |
|---|---|---|---|
| 1 | `parse_expr` → `parse_add` | `+ -` | left |
| 2 | `parse_mul` | `* / %` | left |
| 3 | `parse_unary` | unary `+ -` | right (recursive) |
| 4 | `parse_pow` | `^` | **right** (`base` then `parse_unary` exponent, so `2^-3` works) |
| 5 | `parse_postfix` | postfix `!` | — |
| 6 | `parse_primary` | numbers, `( … )`, `ident( args )`, constants | — |

So `-2^2 = -4` (unary binds tighter than `^` in this implementation: `parse_unary`
sees `-`, recurses into `parse_unary` → `parse_pow`, giving `-(2^2)`), and `2^3^2`
is right-associative.

### Primary

- Number → value.
- `(` expr `)` → value, requires `)`.
- Identifier followed by `(` → function call with a comma-separated argument
  list (possibly empty, though no zero-arg function exists).
- Bare identifier → `constant(name)` or `unknown symbol "<name>"`.

### Errors (all strings, never panics)

`empty expression`, `unexpected trailing input at position N`,
`result is not a finite number`, `unexpected character "<c>"`,
`invalid number "<t>"`, `unexpected "<op>"`, `unexpected end of expression`,
`expected ')'`, `expected ',' or ')' in function call`, `unknown symbol "<n>"`,
`division by zero`, `modulo by zero`, `factorial (!) requires a non-negative
integer`, `factorial argument too large`, per-function argument-count errors,
`unknown function "<n>"`, domain errors (`ln`/`log`/`log2`/`sqrt` of invalid
input). `call_function` enforces arity with a `need(n)` helper before every branch.

### Functions & constants

| Kind | Names |
|---|---|
| Trig (radians) | `sin cos tan` |
| Inverse trig | `asin acos atan` |
| Trig (degrees) | `sind cosd tand`, `asind acosd atand` |
| Hyperbolic | `sinh cosh tanh asinh acosh atanh` |
| Angle conversion | `deg` (rad→deg), `rad` (deg→rad) |
| Log/exp | `ln`, `log` (1-arg base-10 or 2-arg `log(base,x)`), `log2`, `exp` |
| Roots/powers | `sqrt` (rejects negatives), `cbrt`, `pow(a,b)` |
| Rounding | `abs floor ceil round trunc sign` |
| Factorial | `fact`, `factorial`, and the postfix `!` |
| Two-arg | `atan2(y,x)`, `pow(a,b)`, `mod(a,b)`, `hypot(a,b)` |
| Constants | `pi`, `e`, `tau`, `phi` (≈1.618033988749895) |

### `format_number`

- Integral value and `|v| < 1e15` → integer string (no `.0`).
- `|v| ≥ 1e12` or (`|v| < 1e-9` and non-zero) → scientific `{:e}`.
- Otherwise `{:.10}` with trailing zeros (and a trailing `.`) trimmed.

This is why results are stored as TEXT: the display string is the canonical
value the LLM reports back via `result_text`.

## Sharing between tool and window

`calculator_eval` (`src/tools/mod.rs`) calls `evaluate` then `format_number`,
persists to `calculator_history` via `ctx.pool()`, and returns
`{ expression, result: f64, result_text }`.

`POST /api/calculator/eval` (`src/routes.rs`) does the exact same evaluate +
format + insert, returning the same three fields. The window's `equals()`
calls that route; the AI's `agent:actions` handler reads the tool outcome. Both
render `result_text`, so they always agree.

## AI wiring

`web/plugin.js::onAgentActions` filters `agent:actions` for `calculator_*`.
On a successful `calculator_eval` it shows the tool's `result_text` as the
display result and refreshes the history panel if open. It also dispatches
`plugin:focus` so core brings the Calculator window forward.

## Tests

`src/eval.rs` has an inline `#[cfg(test)] mod tests` covering arithmetic,
scientific functions, and rejection of bad input (`1/0`, `sqrt(-1)`, `1 +`, ``).
