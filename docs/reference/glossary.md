# Glossary

Terms that recur across the Shiny docs.

| Term | Meaning |
|---|---|
| **AI sphere** | The core product: a conversational agent plus the voice bar, chat, web search and artifact dock. Everything domain-specific is a plugin. |
| **Action / action block** | A JSON object an LLM emits to call a tool, e.g. `{"action":"hello","params":{"name":"world"}}`. `parse_actions` extracts them from a model reply. |
| **AgentContext** | Per-request state passed to every tool: lat/lon/heading, language, model override. Distinct from `PluginCtx`. |
| **API level** | `api_level` on a plugin manifest. The running core accepts plugins with `api_level ≤ CORE_API_LEVEL` (currently `1`). |
| **AppState** | The single, cheaply-cloneable state struct holding the DB pool, config, services, plugin manager, voice clients and session token. |
| **Artifact** | A structured card (`ActionOutcome.artifact`) rendered inside its owning plugin's window; saved per user and tagged with the owning plugin. |
| **`bridged()`** | Adapter that runs a plugin tool on a plugin-owned runtime so plugin statically-linked sqlx/reqwest/tokio work does not abort the host. |
| **ConfigSnapshot** | The subset of server config handed to plugins via `PluginCtx`. |
| **Core** | The `shiny` binary (`src/`) as opposed to plugins. |
| **Contrib / contribution** | What a plugin registers: tools, routes, crons, skills markdown, persona, context lines. |
| **CronSpec** | A plugin-declared scheduled job (scheduling infrastructure is roadmap). |
| **Deactivate vs uninstall** | Deactivate keeps the install dir + tables and turns tools off; uninstall removes the library and contributions. |
| **`doc_fragment`** | Markdown a tool returns to advertise itself to the LLM. Tools with no fragment stay hidden. |
| **HUD** | The fixed top bar: clock/weather, plugin tray, host chips, workspace tabs, power menu, Settings/Plugins/Chats. |
| **Kiosk** | The full-screen native shell (`peakd`/`peakd-mac`) that renders the web UI on a dedicated machine. |
| **`PluginCtx`** | Per-plugin context carrying the manifest, config snapshot, and lazy plugin-owned DB/HTTP clients. |
| **`plugin_schema_versions`** | Core-owned table recording which plugin migration files have run. |
| **`shiny_plugin_entry`** | The C symbol a plugin cdylib exports so the loader can obtain a `*mut dyn Plugin`. |
| **Sidecar** | A separate speech process (Supertonic, faster-whisper, Qwen3-TTS) spawned by the server or by hand. |
| **Skill markdown** | Markdown concatenated into the agent's system prompt describing available tools. |
| **Studio Grid** | The modular patch inside the Studio plugin. |
| **Traveler** | Historically the whole user/domain model; now either the `traveler` plugin or the core-served travel services, depending on context. |
| **Window surface** | A plugin's `web/plugin.js` default export: `mount`/`unmount`/`getElement`/`wireEvents`/`contextMenu`. |
| **Workspace** | One Hyprland-style virtual desktop in the web shell; windows are placed per workspace. |
