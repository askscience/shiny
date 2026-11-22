# The plugin builder

The assistant can build a plugin from a conversation: create it from a
template, write its sources, check them, compile the engine, pack the archive
and install it. It does all of it with a fixed set of **core commands**, never
with the terminal or the file tools: each command is a function of the server
with fixed parameters, and the assistant only chooses which one to call.

Source: [`src/services/plugin_dev/`](../../src/services/plugin_dev),
[`src/services/ask.rs`](../../src/services/ask.rs) (the cards),
[`web/skills/plugin-builder*.md`](../../web/skills) (what the assistant reads).
The agent-loop side is in [agent § Questions, plans and choices](../core/agent.md#47-questions-plans-and-choices).

## What the user decides

| Decision | How | Enforced by |
|---|---|---|
| The **language** of the plugin (Python or Rust) | the choice card of `plugin_dev_start`, once per conversation | `state::choose_language`: a chat keeps its first language; a language the model passes is ignored |
| The **plan** | the plan card of `plan_propose` (Approve / Ask for changes / Cancel) | `commands::run`: nothing is created, written, built, packed or installed until the chat's plan is approved |
| The **install** | the allow / deny card of a gated action | the approval gate of the agent loop (`describe_gated`) |

By voice there is no card: the assistant asks aloud and the next message
answers, read in code with strict rules (a plain yes approves a plan; a
language is set only by a message that names exactly one).

A language this machine cannot build in yet is shown on the card with what it
lacks and cannot be picked, by card or by voice: a conversation keeps its
language for good, so it is never bound to one that leads nowhere.

Nothing of the builder works for a remote client (Iroh, a public URL): a
plugin is code that runs on the machine.

## Modules: one language per conversation

The commands are split in modules (`plugin_dev/mod.rs`, the `COMMANDS` table):

| Module | Commands | A conversation has it when |
|---|---|---|
| entry | `plugin_dev_start` | always |
| common | `plugin_dev_new`, `plugin_dev_list`, `plugin_dev_tree`, `plugin_dev_read`, `plugin_dev_write`, `plugin_dev_patch`, `plugin_dev_delete`, `plugin_dev_check`, `plugin_dev_pack`, `plugin_dev_install`, `plugin_dev_status`, `plugin_dev_logs` | it has chosen a language |
| rust | `rust_doctor`, `rust_build` | it chose Rust |
| python | — (the Python kit is not in this version yet) | it chose Python |

`scope_refusal` refuses a command of a module the conversation does not have,
whatever the prompt says. The prompt follows the same split: a conversation
gets `web/skills/plugin-builder.md` plus its language's file, so the other
language's commands are not even named to the model. A conversation reopened
later reads its language from `chat_agent_state` and gets the same set again.

A workspace records the language it was created in. A conversation of another
language cannot work on it: the plugin has to be opened in a new chat.

A chat that builds gets 160 tool steps per turn instead of 40.

## Workspaces

A plugin's sources live in `<data>/plugin-dev/<name>/`, next to the installed
plugins and never inside them (the server reloads whatever changes there).
`workspace.rs` is the only way the assistant reads or writes one:

- a path is a plain relative path inside the workspace: no `..`, no absolute
  path, no hidden file, no symbolic link on the way;
- `target/`, `bin/` and `dist/` hold what the build produces and are not
  written by hand;
- files are text of known kinds (`rs`, `toml`, `py`, `js`, `css`, `json`, `md`,
  `sql`, `svg`, `html`, `txt`), at most 256 KB each, 300 per plugin;
- a name is 2–32 lowercase letters, digits, `-` or `_`, is not a Craft app's
  and not an installed plugin's.

`plugin_dev_new` fills the workspace from templates
(`plugin_dev/templates/`): the manifest, a first migration, the window
(`web/plugin.js`, `plugin.css`, `icon.svg`, `i18n/en.json`), the skill, and the
language's engine. The template is a working plugin (a list with two tools and
two routes) that the assistant reads and then turns into what was asked.

## Checks

`plugin_dev_check` runs static checks (`checks.rs`); `only` runs one of them.
Every finding names a file and, where it can, a line.

| Check | What it holds the plugin to |
|---|---|
| `manifest` | `plugin.toml` parses and is what the loader accepts: the workspace's name, a version, `api_level`, `runtime = "process"`, relative directories |
| `web` | `web/plugin.js` has the window's default export and sets `data-plugin`; imports are relative; nothing the Content-Security-Policy refuses (`eval`, inline scripts and handlers, a direct `fetch` to another site) |
| `i18n` | catalogues are flat JSON with `plugin.<name>.` keys; every key the window uses is in `en.json`; a translation only has the reference's keys |
| `migrations` | `NNN_name.sql`, unique numbers, not empty; a migration that is installed is neither edited nor removed |
| `conflicts` | tools are named `<name>_…` and belong to no other plugin; routes are under `/api/<name>/`; the skill names every tool |

The two naming rules make a clash with another plugin impossible by
construction. Tools and routes are read from the sources (`fn name(&self)` and
`RouteSpec::…(` in Rust).

## Build, pack, install

Each step records the fingerprint of the sources it ran on
(`Workspace::sources_digest`, in `.workspace.json`) and the next step refuses
sources it did not see:

1. `plugin_dev_check` without `only`, passed, records `checked`.
2. `rust_build` runs `cargo build --release` in the workspace (the command is
   fixed), into a target directory all workspaces share
   (`<data>/plugin-dev/.target`), and copies the executable to `bin/<name>`.
   Compiler messages about the plugin's own crate come back as
   `{file, line, column, code, message, help}`, at most 20 errors and 8
   warnings; when cargo itself cannot start the build its last lines come
   back as `detail`. The build stops with the turn and after 40 minutes.
3. `plugin_dev_pack` needs `checked` and `built` to be the current sources. It
   writes `dist/<name>-<version>.tar.gz` (`plugin.toml`, `bin/<name>`, `web/`,
   `skills/`, `migrations/`; the engine's sources are not installed) and
   records its SHA-256.
4. `plugin_dev_install` takes only that archive, checked against the recorded
   SHA-256 and the current sources, and hands it to the ordinary installer
   (`installer::install_archive`): the same staging, backup and rollback as
   an upload. The plugin is then turned on for the user.

`plugin_dev_status` says where the sources stand and whether the installed
plugin runs (version, process id, tools, routes); `plugin_dev_logs` returns
the last lines the plugin printed (`[<name>] …` in the server log) and its
install events.

## What a Rust plugin needs on the machine

`rust_doctor` and the choice card read `rust::readiness()`:

- `cargo` and `rustc`, in `PATH` or `~/.cargo/bin`;
- a C linker (`cc`) and the system SQLite's development files;
- the **SDK bundle**: a directory laid out like the repository, with
  `crates/shiny-plugin-sdk`, `crates/shiny-plugin-protocol`,
  `vendor/libsqlite3-sys` and the server's `Cargo.lock`. It is looked for in
  `SHINY_PLUGIN_SDK_DIR`, then `/usr/share/shiny/plugin-sdk`, then the
  directory the server runs in (a source checkout).

The vendored `libsqlite3-sys` is required: the template patches it in, as the
server's own build does. Without it the SDK would build SQLite's bindings with
bindgen and need libclang.

A new plugin starts from the bundle's `Cargo.lock`, so the first build needs
nothing newer from the network than the server was built with. Measured on the
development machine: 3 min 08 s for the first build (the SDK and its
dependencies), 600 MB in the shared target directory.

## Not there yet

- Python plugins: the choice card shows the language as not available yet and
  says its kit is missing; it cannot be chosen.
- Fetching what Rust needs when the machine lacks it. The card lists what is
  missing and Rust cannot be chosen until it is there; nothing is downloaded.
- Packaging the SDK bundle (`/usr/share/shiny/plugin-sdk` is looked for but no
  package installs it yet).
- Working on an installed plugin's window and skill, rolling back, the
  window's JavaScript errors, calling a route or a tool as a test.
