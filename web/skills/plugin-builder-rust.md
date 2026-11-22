### Rust engine

This chat builds plugins in Rust. The engine is a small crate; the template's files are the reference:

- `src/plugin.rs` — what the plugin registers: its skill text, its routes (`RouteSpec::get("/api/<name>/…", "tag")`) and its tools (`.tools([Arc::new(MyTool) as Arc<dyn Tool>, …])`). A new tool or route must be added here.
- `src/tools.rs` — one struct per tool implementing `Tool`: `name()` (the name the assistant calls), `invoke()` (reads `req.params` with `require_str` / `param_str` / `param_u32` / `param_bool`, returns `ActionOutcome::ok(self.name(), json!({…}))`). `req.user_id` is the user the call is for.
- `src/routes.rs` — `handle(ctx, tag)` maps each route tag to a handler built with `bridged_route`; `user_id(&req)?` is the caller; a JSON body is read with `axum::Json::<T>::from_request(req, &()).await`; answer with `Ok(ok(json!({…})))`.
- `src/store.rs` — the data, shared by tools and routes. Use `ctx.db()`: `execute(sql, &[Value::text(..), Value::Int(..)])` and `query(sql, &[..])` (rows of `Value`; read them with `as_text` / `as_int`). Always filter by `user_id`.
- `src/main.rs`, `Cargo.toml` — leave them as they are. Add a dependency only when the plugin truly needs it: a new crate has to be downloaded and compiled.

Errors are `AppError` (`AppError::BadRequest("…".into())`, `AppError::NotFound(…)`); `?` works on database and JSON errors. Do not hold a `std::sync::Mutex` across an `.await`, and put CPU-heavy work in `tokio::task::spawn_blocking`.

- `rust_build` — compile the engine. params: `{}`. The first build of a new plugin takes a few minutes; say so before you start it. Errors come with file, line and the compiler's own suggestion (`help`): fix them all, then build again. Warnings do not block.
- `rust_test` — start the built engine as the server would, without installing it, and report whether it starts and which tools and routes it declares. params: `{}`. Run it after a successful build, before packing.
- `rust_doctor` — what this machine has and lacks for building Rust plugins. params: `{}`
