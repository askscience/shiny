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
| common | `plugin_dev_new`, `plugin_dev_list`, `plugin_dev_tree`, `plugin_dev_read`, `plugin_dev_write`, `plugin_dev_patch`, `plugin_dev_delete`, `check_all`, `check_manifest`, `check_web`, `check_i18n`, `check_migrations`, `check_conflicts`, `plugin_dev_pack`, `plugin_dev_install`, `plugin_dev_status`, `plugin_dev_logs` | it has chosen a language |
| rust | `rust_doctor`, `rust_build`, `rust_test` | it chose Rust |
| python | `py_check`, `py_test` | it chose Python |

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

`check_all` runs the static checks (`checks.rs`); `check_manifest`,
`check_web`, `check_i18n`, `check_migrations` and `check_conflicts` run one.
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

1. `check_all`, passed, records `checked`.
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

## Python plugins

A Python plugin needs only `python3`, which the LiteAura package depends on:
nothing is compiled and nothing is downloaded.

- `engine/plugin.py` is the engine: tools declared with `@plugin.tool(…)`,
  routes with `@plugin.route("GET", "/api/<name>/…")`, data through
  `ctx.query` / `ctx.execute` (the standard `sqlite3` module on the
  application database).
- `engine/shiny_plugin.py` is the **kit**
  (`plugin_dev/templates/python/engine/shiny_plugin.py`): the plugin side of
  the [process protocol](process-plugins.md#the-protocol), version 1, in the
  standard library only. It answers the handshake, the lifecycle calls, tool
  calls and forwarded route requests, serves one call at a time, and exits
  when the server goes away or takes its socket. It is copied into every
  plugin, so an installed plugin carries the kit it was written against; the
  builder refuses to edit it and `py_check` restores it if it changed.
- `py_check` parses every file of `engine/` (nothing is executed) and reports
  syntax errors and imports of anything but the standard library and the
  plugin's own files, with file and line. When it passes it writes the
  launcher `bin/<name>` (`exec python3 -B -s engine/plugin.py`), which is what
  makes the plugin packable, as a build does for Rust.
- `py_test` (and `rust_test`) start the engine as the server would, without
  installing it, and report whether it answers the handshake and which tools
  and routes it declares. Nothing of it is called and it is stopped at once.

The kit is a second implementation of the protocol next to the Rust SDK's:
a change to the protocol has to reach both (`tests/agent_ask.rs` runs a
Python plugin through the real server on every test run).

## What a Rust plugin needs on the machine

`rust_doctor` and the choice card read `rust::readiness()`:

| Needed | Where it comes from | When it is missing |
|---|---|---|
| The **SDK bundle**: `crates/shiny-plugin-sdk`, `crates/shiny-plugin-protocol`, `vendor/libsqlite3-sys` and the server's `Cargo.lock`, laid out like the repository | the LiteAura package, in `/usr/share/shiny/plugin-sdk` (`scripts/stage-plugin-sdk.sh`, 2 MB); a source checkout is one too; `SHINY_PLUGIN_SDK_DIR` names another | Rust cannot be chosen: the installation is incomplete |
| The system **SQLite** library | the LiteAura package depends on it | Rust cannot be chosen |
| `cargo` and `rustc`, in `PATH` or `~/.cargo/bin` | installed when Rust is chosen | see below |
| A C linker (`cc`) | the package recommends `gcc` and `libc6-dev`; installed when Rust is chosen | see below |

The vendored `libsqlite3-sys` is required: the template patches it in, as the
server's own build does. Without it the SDK would build SQLite's bindings with
bindgen and need libclang. SQLite's development package is not needed: where
only the runtime library exists, the build links it through a symbolic link of
its own (`<data>/plugin-dev/.lib/libsqlite3.so`, on `LIBRARY_PATH`).

A new plugin starts from the bundle's `Cargo.lock`, so the first build needs
nothing newer from the network than the server was built with. Measured on the
development machine: 3 min for the first build (the SDK and its
dependencies), 600 MB in the shared target directory.

### Choosing Rust installs what is missing

On the choice card a language is `ready`, `setup` (choosing it installs what
the card lists) or `unavailable` (it lacks something that cannot be installed
from here, and cannot be picked: a conversation keeps its language for good,
so it is never bound to one that leads nowhere).

Choosing a `setup` language runs `Machine::install` (`rust_setup.rs`) before
the language becomes the conversation's; if it fails the chat is free to
choose again. The commands are fixed:

- **The C linker**: `apt-get install -y --no-install-recommends gcc libc6-dev`
  through `sudo -S -k`. When `sudo` needs a password, a second card asks for
  it in a hidden field. It is piped to `sudo`'s standard input and dropped: it
  is in no argument, environment variable, log line, stored conversation row
  or prompt. Other package managers are not driven: the card's failure says
  to install a C compiler by hand.
- **The compiler**: `rustup-init` from `static.rust-lang.org`
  (`SHINY_RUSTUP_DIST_URL` overrides it), checked against the SHA-256
  published next to it, then run as the session user with
  `-y --profile minimal --default-toolchain stable --no-modify-path`. It
  installs into the user's home; no administrator rights.

By voice there is no card to agree on: a language that needs setup is not
chosen, and the assistant says it is set up from the text chat.

## Not there yet

- Working on an installed plugin's window and skill, rolling back, the
  window's JavaScript errors, calling a route or a tool as a test.
