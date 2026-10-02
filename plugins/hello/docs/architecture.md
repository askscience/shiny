# Hello — architecture

The minimal plugin, used as the canonical authoring example. It registers one
tool.

```
plugins/hello/
├── plugin.toml
├── skills/hello.md
├── migrations/001_init.sql     hello_pings
├── src/
│   ├── lib.rs        module wiring (pub mod plugin; pub mod tool;)
│   ├── plugin.rs     manifest + register + entry
│   └── tool.rs       HelloTool
└── web/icon.svg
```

## Load path

1. Core parses `plugin.toml` (`name = "hello"`, `api_level = 1`, …).
2. `dlopen`s `libshiny_hello_plugin.so`, resolves and calls
   `shiny_plugin_entry()`, getting `*mut dyn Plugin`.
3. Runs `migrations/001_init.sql` (`hello_pings`) if not yet recorded.
4. Calls `register()`, which sets skills markdown and adds `HelloTool` via
   `builder.tool(crate::tool::HelloTool)`.

> The `hello` plugin is the one bundled plugin that does **not** wrap its tool
> with `bridged(...)` — it performs no async I/O, so it is safe on the host
> runtime. Any plugin that touches the DB or network **must** use `bridged`.

## Tool

`HelloTool` (`src/tool.rs`):

- `name` = `hello`, `step_label` = `Saying hello…`.
- `doc_fragment` advertises `{ name?: string }`.
- `invoke` returns `ActionOutcome::ok("hello", { who, reply })`.

## Exercises

See [README](README.md) and [tools](tools.md). For the full authoring
walkthrough that mirrors this plugin, see
[Authoring](../../../docs/plugins/authoring.md).
