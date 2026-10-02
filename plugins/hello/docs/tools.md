# hello — agent tools

> [Plugin README](./README.md) · [Plugins overview](../../../docs/plugins/README.md)

`hello` registers exactly one agent tool. Tools are dispatched from LLM action
blocks of the form `{"action":"<name>","params":{…}}`.

## `hello`

Say hello to someone. The reference implementation of a `Tool`.

| Property | Value |
|---|---|
| **Name** | `hello` |
| **Aliases** | none |
| **Step label** | `Saying hello…` |
| **Doc fragment** | `` - `hello` — Say hello. params: `{ name?: string }` `` |
| **Humanize** | `Said hello to <who>` |
| **Code** | `plugins/hello/src/tool.rs:9` |
| **Registered** | `plugins/hello/src/plugin.rs:51` via `bridged(Arc::new(HelloTool))` |

### Parameters

| Param | Type | Required | Default | Notes |
|---|---|---|---|---|
| `name` | `string` | no | `"world"` | Read with `req.params.param_str("name")` |

```json
{"action":"hello","params":{"name":"world"}}
```

### Returns

`ActionOutcome::ok("hello", …)`:

```json
{
  "action": "hello",
  "result": "ok",
  "data": { "who": "world", "reply": "Hello, world!" }
}
```

`data.who` is the resolved name (defaulted to `"world"`); `data.reply` is
`format!("Hello, {name}!")`. `humanize` reads `data.who` to produce the
completed-steps log line, falling back to `"world"`.

### Errors

The tool does not return `ActionOutcome::error` and cannot fail under normal
input: a missing/non-string `name` silently falls back to `"world"`. It reuses
the SDK helper `param_str`, which returns `None` rather than raising. There is
therefore no `AppError` path from this tool.

### Invocation flow

1. LLM emits `{"action":"hello","params":{"name":"Alice"}}`.
2. The core `ToolRegistry` resolves `hello` to the bridged `HelloTool`.
3. `BridgedTool::invoke` copies the request onto the plugin runtime and calls
   `HelloTool::invoke` (`plugins/hello/src/tool.rs:25`).
4. The runner records the `ActionOutcome` in `actions_taken`; the LLM then
   writes the natural-language reply.
