# Calculator plugin — agent tools

Three tools, registered in `src/plugin.rs::register` and implemented in
`src/tools/mod.rs`. Every tool is wrapped with
`shiny_plugin_sdk::tools::bridged(...)` (PLUGINS.md §15) and uses the async
`ctx.pool().await` for SQLite.

Shared helper:

```rust
fn expression_param(req: &ToolRequest<'_>) -> Result<String, AppError>
```

Accepts `expression` or the `expr` alias, trims, and rejects empty with
`BadRequest("expression required")` (`src/tools/mod.rs`).

---

## `calculator_eval`

| Field | Value |
|---|---|
| Name | `calculator_eval` |
| Aliases | `calculate`, `compute`, `evaluate` |
| Step label | `Calculating…` |
| Code | `plugins/calculator/src/tools/mod.rs` → `CalculatorEval` |

### Parameters

| Param | Type | Required | Notes |
|---|---|---|---|
| `expression` | string | yes | Full expression. Alias: `expr`. Trimmed; empty rejected. |

### Invoke

1. `expression_param(&req)?`.
2. `crate::eval::evaluate(&expression)` → `f64`; maps engine error to
   `AppError::BadRequest`.
3. `format_number(result)`.
4. `INSERT INTO calculator_history (user_id, expression, result, created_at)`
   with `req.traveler_id` (the current user id).
5. Returns `ActionOutcome::ok("calculator_eval", json!({...}))`.

### Returns

```json
{
  "action": "calculator_eval",
  "result": "ok",
  "data": {
    "expression": "(2+3)*4",
    "result": 20.0,
    "result_text": "20"
  }
}
```

`humanize`: `"<expr> = <result_text>"`, or `"Calculated"` when `expression` is
empty.

### Error cases

| Error | When |
|---|---|
| `BadRequest("expression required")` | Missing/blank `expression`/`expr`. |
| `BadRequest(<eval message>)` | Parser/domain failure (e.g. `division by zero`, `sqrt of a negative number`). |
| DB error (`AppError` from `sqlx`) | Insert fails. |

---

## `calculator_history`

| Field | Value |
|---|---|
| Name | `calculator_history` |
| Aliases | `calculation_history`, `calc_history` |
| Step label | `Reading calculator history…` |
| Code | `plugins/calculator/src/tools/mod.rs` → `CalculatorHistory` |

### Parameters

| Param | Type | Required | Notes |
|---|---|---|---|
| `limit` | number | no | Default `20`, clamped to `1..=100`. |

### Invoke

`SELECT expression, result, created_at FROM calculator_history WHERE user_id = ?1
ORDER BY id DESC LIMIT ?2` for `req.traveler_id`.

### Returns

```json
{
  "action": "calculator_history",
  "result": "ok",
  "data": {
    "history": [ { "expression": "2+2", "result": "4", "at": "2026-10-02 12:00:00" } ],
    "count": 1
  }
}
```

`humanize`: `"Found <count> recent calculations"`.

### Error cases

- DB error only.

---

## `calculator_clear_history`

| Field | Value |
|---|---|
| Name | `calculator_clear_history` |
| Aliases | `clear_calculator_history`, `clear_calc_history` |
| Step label | `Clearing calculator history…` |
| Code | `plugins/calculator/src/tools/mod.rs` → `CalculatorClearHistory` |

### Parameters

None (`{}`).

### Invoke

`DELETE FROM calculator_history WHERE user_id = ?1` for `req.traveler_id`.

### Returns

```json
{ "action": "calculator_clear_history", "result": "ok", "data": { "cleared": true } }
```

`humanize`: `"Calculator history cleared"`.

### Error cases

- DB error only. Deleting an empty history is a no-op success.
