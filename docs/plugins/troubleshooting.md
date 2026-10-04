# Plugin troubleshooting

Always inspect **`PLUGINS_DIR/install.log`** first (default
`data/plugins/install.log`, or `GET /api/plugins/install.log`). Every install
step is timestamped there.

---

## Install & load

| Symptom | Likely cause | Fix |
|---|---|---|
| `Unrecognised archive format` | Upload wasn't zip/gz/tar | `file archive.zip`; re-export. |
| `Invalid plugin.toml: …` | Missing/mistyped field | Re-serialize with `toml::to_string(&manifest)`. |
| `Plugin api_level X > core Y` | Built against a newer SDK | Upgrade the core, or rebuild the plugin. |
| `Plugin built for 'x86_64…' but host is 'aarch64…'` | Wrong platform binary | Rebuild the cdylib on the target. |
| `No cdylib found` | Forgot `.so/.dylib/.dll` | `ls` the archive; include the cdylib. |
| `… was built before the security floor …` warning | A stale cdylib is loaded after a security fix | Rebuild and reinstall the plugin; `SHINY_MIN_PLUGIN_TS=0` only for a knowingly frozen build. |
| `Missing symbol shiny_plugin_entry` | Entry fn missing/wrapped | Add `#[no_mangle] pub extern "C" fn shiny_plugin_entry()`. |
| 401 on install | Not logged in / bad token | Use a valid Bearer token. |
| Tool not registered | `register()` didn't add it | Ensure `builder.tool_arc(bridged(tool))`. |
| LLM doesn't call the tool | `doc_fragment()` returns `None` | Return a doc fragment so it is advertised. |

## Runtime crashes

| Symptom | Likely cause | Fix |
|---|---|---|
| Host aborts: `this functionality requires a Tokio context` | Tool ran on the host runtime | Wrap with `bridged(...)` ([runtime & ABI](runtime-abi.md)). |
| SIGSEGV in `sqlite3_value_free` / `ValueHandle::drop` | A `SqlitePool` crossed the dlopen boundary | Use `ctx.db()` (preferred) or `ctx.pool()` — never a host pool. |
| Abort in `hyper-util … dns.rs`: `there is no reactor running` | Host-built reqwest client on the plugin runtime | Use `ctx.ollama()/ctx.search()/ctx.supertonic()`. |
| `there is no reactor running` in `on_load` | Bare `tokio::spawn` on the host executor | Use `shiny_plugin_sdk::rt::spawn(...)`. |
| Plugin panics but server survives | Expected: jobs are isolated | Check logs; the tool returns an `AppError`. |
| Server hangs | A job blocked the single plugin thread | Avoid blocking the plugin runtime; keep work async. |

## Routes

| Symptom | Likely cause | Fix |
|---|---|---|
| Route 404 | Not mounted, or `route_handler(tag)` returned `None` | Ensure the `RouteSpec.handler_tag` matches a tag your `route_handler` resolves. |
| Route 401 unexpectedly | Spec `auth` defaults to auth | Set `auth = "public"` if intended. |
| Path param is empty | Using `axum::extract::Path` in the plugin | Read `path_params_from_request(req)` instead. |
| Empty identity | Reading an extension | Read `user_id_from_request(req)` / headers. |

## Migrations

| Symptom | Likely cause | Fix |
|---|---|---|
| New column missing on an existing install | Edited an already-applied file | Add a **new numbered** file; see [migrations](migrations.md). |
| `CREATE TABLE IF NOT EXISTS` did nothing | Table created by an earlier file | Replay must produce the final shape; add a repair. |
| Runtime "no such column" but install "succeeded" | Plugin swallows DB errors | Inspect the actual DB; repair from `on_load`. |

## Window surfaces

| Symptom | Likely cause | Fix |
|---|---|---|
| Window never appears | `web/plugin.js` is not loaded or default export lacks `mount` | Check the browser console; the app logs one `console.warn`. |
| Edits to `plugins/<name>/web/` do nothing | The app serves the **installed** copy | Copy to `data/plugins/<name>/web/` or reinstall. |
| Window mounts but looks unstyled | Plugin shipped CSS or wrong class | Build with the core UI library; use the `<name>-*` prefix. |
| Icon not shown | No `apps/<name>` mapping and no `web/icon.svg` | Add one; see [themes & icons](../core/themes-icons.md). |

### Smoke-testing a window

`node --check web/plugin.js` catches syntax errors. For more, follow
[`plugins/browser/web/plugin.smoke.mjs`](../../plugins/browser/web/plugin.smoke.mjs):
it mounts the real surface against a small DOM shim and asserts the generated
markup and the postMessage contract.

---

## Useful commands

```bash
# Build everything.
cargo build --release --workspace

# Check one plugin.
cargo check -p shiny-myplugin-plugin

# Watch the log while installing.
tail -f data/plugins/install.log

# List installed plugins.
curl -H "Authorization: Bearer $TOKEN" localhost:8080/api/plugins

# Read the last 200 log lines over HTTP.
curl -H "Authorization: Bearer $TOKEN" localhost:8080/api/plugins/install.log
```

---

## Debugging checklist

1. `install.log` — did the install actually succeed?
2. `cargo check` the plugin crate.
3. Did you wrap every tool with `bridged(...)` and every route with
   `bridged_route(...)`?
4. Are all DB/HTTP calls through `PluginCtx` accessors?
5. Does the plugin swallow DB errors? (Search for `.ok()`/`unwrap_or_default()`
   around writes.)
6. Does `web/plugin.js` parse, and is the installed copy up to date?
7. Is the tool's `doc_fragment()` present?
