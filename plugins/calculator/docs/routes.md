# Calculator plugin — REST routes

Three routes, declared in `src/plugin.rs::route_specs()` and implemented in
`src/routes.rs::handle`. All require `auth: "auth"` and are wrapped in
`bridged_route`. Handlers resolve the caller with
`user_id_from_request(req)` → `AppError::Unauthorized("not authenticated")`
when absent. Successful data is always `{ "success": true, "data": <payload> }`
built by the local `ok()` helper. DB access is the synchronous `ctx.db()`.

Common helpers in `src/routes.rs`:

| Helper | Purpose |
|---|---|
| `user_id(req)` | Extract the authenticated user. |
| `ok(data)` | Wrap `data` in the `{success,data}` envelope. |
| `as_text(&Value)` | `Text`/`Int` → `String`. |
| `take_query::<T>(req)` | Parse the query string, returning `(T, Request)` so the body is preserved. |

---

## `POST /api/calculator/eval`

| | |
|---|---|
| Method / path | `POST /api/calculator/eval` |
| Auth | Bearer / session (`auth`) |
| Handler tag | `eval` |
| Code | `src/routes.rs` → `fn eval` |

### Request

JSON body (`EvalBody`, all optional with `#[serde(default)]`):

```json
{ "expression": "(2+3)*4" }
```

### Response

```json
{
  "success": true,
  "data": { "expression": "(2+3)*4", "result": 20.0, "result_text": "20" }
}
```

Side effect: inserts a `calculator_history` row for the caller.

### Error cases

| Status | Error |
|---|---|
| 400 | `expression required` (empty body/blank expression). |
| 400 | `invalid JSON body: …`. |
| 400 | Evaluator failure message (e.g. `division by zero`, `sqrt of a negative number`). |
| 401 | `not authenticated`. |
| 500 | DB insert failure. |

---

## `GET /api/calculator/history`

| | |
|---|---|
| Method / path | `GET /api/calculator/history` |
| Auth | Bearer / session (`auth`) |
| Handler tag | `history_list` |
| Code | `src/routes.rs` → `fn history_list` |

### Request

Query parameters (`Q`, all optional):

| Query | Type | Default | Notes |
|---|---|---|---|
| `limit` | integer | `50` | Clamped to `1..=200`. |

### Response

```json
{
  "success": true,
  "data": {
    "history": [
      { "expression": "2+2", "result": "4", "at": "2026-10-02 12:00:00" }
    ],
    "count": 1
  }
}
```

Rows are `ORDER BY id DESC` (newest first).

### Error cases

| Status | Error |
|---|---|
| 400 | `invalid query: …`. |
| 401 | `not authenticated`. |
| 500 | DB query failure. |

---

## `DELETE /api/calculator/history`

| | |
|---|---|
| Method / path | `DELETE /api/calculator/history` |
| Auth | Bearer / session (`auth`) |
| Handler tag | `history_clear` |
| Code | `src/routes.rs` → `fn history_clear` |

### Request

No body, no query.

### Response

```json
{ "success": true, "data": { "cleared": true } }
```

Deletes every `calculator_history` row for the caller. Deleting an empty
history still returns `cleared: true`.

### Error cases

| Status | Error |
|---|---|
| 401 | `not authenticated`. |
| 500 | DB delete failure. |
