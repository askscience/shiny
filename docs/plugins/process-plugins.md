# Process plugins and the plugin catalogue

A plugin can run in two ways:

| `runtime` in `plugin.toml` | What ships | Runs |
|---|---|---|
| `dylib` (default) | `libshiny_<name>_plugin.so` | inside the server, loaded with `dlopen` |
| `process` | `bin/<name>` | as its own process, started by the server |

A library plugin shares the server's memory and its Rust ABI (see
[runtime-abi.md](runtime-abi.md)): it only works with the exact server build it
was compiled with, and a crash in it takes the server down. A process plugin
talks to the server over a private Unix socket with a versioned protocol, so:

- a plugin built for an earlier release keeps working with later servers, as
  long as they still speak its protocol version;
- a crash costs only that plugin, which the server restarts;
- it can be published, installed, updated and removed on its own.

Every plugin is a process plugin except **terminal**, **updates** and
**installer**: they manage the server itself, ship in the Debian package and
stay libraries.

Source: [`crates/shiny-plugin-protocol`](../../crates/shiny-plugin-protocol/src/lib.rs)
(the contract), [`crates/shiny-plugin-sdk/src/process.rs`](../../crates/shiny-plugin-sdk/src/process.rs)
(plugin side), [`src/plugins/process.rs`](../../src/plugins/process.rs) (server
side), [`src/plugins/catalog.rs`](../../src/plugins/catalog.rs) (catalogue).

## Writing one

Nothing changes in the plugin's code: the same `impl Plugin` serves both
runtimes. A process plugin crate adds a binary target and a one-line `main`:

```toml
# Cargo.toml
[lib]
name = "shiny_word_plugin"
crate-type = ["rlib"]

[[bin]]
name = "shiny-plugin-word"
path = "src/main.rs"
```

```rust
// src/main.rs
fn main() {
    shiny_plugin_sdk::process::run(shiny_word_plugin::WordPlugin::new());
}
```

```toml
# plugin.toml
runtime = "process"
```

`scripts/plugin-to-process.py <name> <crate::Type> [--idle]` does all three.
`--idle` uses `process::run_with` with `idle_stop`: the server stops the
process after ten idle minutes and starts it again on the next request. Leave
it out when `on_load` starts background work (mail sync, a scheduler, audio).

The process runs in the **server's working directory** (relative paths keep
their meaning) with these variables:

| Variable | Value |
|---|---|
| `SHINY_PLUGIN_SOCKET` | the socket to listen on (the SDK does it) |
| `SHINY_PLUGIN_NAME` | the plugin's name |
| `SHINY_PLUGIN_DIR` | its install directory |
| `SHINY_HOST_EXE` | the server executable (to find helpers installed next to it) |

Its stdout and stderr go to the server log, prefixed with `[<name>]`.

## The protocol

HTTP/1.1 on the Unix socket. Two kinds of request reach the plugin:

- **Control calls**, `POST` with JSON under `/_shiny/`: `hello` (handshake:
  the server sends its protocol range and config, the plugin answers with the
  chosen version and its tools, routes, skills, persona and context lines),
  `load`, `unload`, `user-registered`, `tool`.
- **Route requests**: the user's request, forwarded unchanged with the route's
  tag in `x-shiny-route-tag`. Identity and path parameters travel in the same
  `x-shiny-*` headers a library plugin reads.

Compatibility rules (written in the protocol crate, which is the promise):

- each side speaks a range of versions; the handshake picks the highest shared
  one, and a plugin with no shared version is not loaded;
- additive changes keep the version: new optional fields with
  `#[serde(default)]`, new control paths a peer may answer with 404; unknown
  fields are ignored on both sides;
- a breaking change raises `PROTOCOL_MAX`, and the server keeps the earlier
  versions for several releases before raising `PROTOCOL_MIN`.

## Lifecycle

The server's supervisor (`src/plugins/process.rs`):

- starts the process when the plugin loads, waits for its socket and says
  hello; tools and routes register from the answer;
- restarts it after a crash (backoff 1 s, 2 s, 4 s...), replaying `on_load`;
  more than three crashes in a minute and it stays down until reinstalled;
- stops it when idle, if the plugin asked for that;
- on uninstall or replacement, calls `unload`, then SIGTERM, then SIGKILL;
- stops every plugin process before the server exits or re-executes itself.

A plugin also exits on its own when its parent changes or its socket is
removed, so none outlives the server.

Measured on the development machine (release build): about 5 MB of memory per
idle process, 13 ms from start to ready, under 1 ms per call.

## Packaging

`scripts/stage-plugin.sh <name> <dest>` lays out one plugin as the server
loads it. `scripts/build-plugin-archives.sh` makes one
`<name>-<version>-linux-<arch>.tar.gz` per process plugin and `index.json`,
the catalogue. On a release tag the publish workflow:

1. uploads the archives to the GitHub Release;
2. signs `index.json` with the LiteAura archive key (`index.json.sig`);
3. publishes both on Pages at `https://askscience.github.io/shiny/plugins/`.

The Debian package ships only terminal, updates and installer, plus
`/usr/share/shiny/default-plugins`, the list every account gets.

## The catalogue

The Plugin manager's **Apps** section lists what the catalogue offers for
this server: matching architecture and a shared protocol version. **Get**
and **Update** download the archive, check it against the SHA-256 in the
signed index and install it with the ordinary installer. **Remove** uninstalls
it.

The index is trusted only after `gpgv` checks its signature against
`/usr/share/keyrings/liteaura-archive.gpg`, the key APT already trusts.
Archives come only from the release hosts over HTTPS.

In the background (20 s after start, then every six hours; two minutes after
a failure) the server installs the default plugins this account never had and
updates installed ones the catalogue has a newer version of. A default the
user removed is not installed again (`.catalog-seeded.json` in the plugins
directory).

| Variable | Default | Use |
|---|---|---|
| `SHINY_PLUGIN_INDEX_URL` | the Pages URL | another catalogue; a `file://` one needs no signature (a local build) |
| `SHINY_PLUGIN_KEYRING` | `/usr/share/keyrings/liteaura-archive.gpg` | the key the index is checked against |
| `SHINY_DEFAULT_PLUGINS_FILE` | `/usr/share/shiny/default-plugins` | the default list |
| `SHINY_PLUGIN_AUTO_UPDATE` | on | `0` stops the background installs (a machine running its own builds) |

## Development

```sh
cargo build --release -p shiny-word-plugin
scripts/install-plugin-dev.sh word        # copies bin/word and the rest
```

To move a development machine from library plugins to process plugins in
one step (install every rebuilt plugin, turn off catalogue auto-updates,
restart, report what started), run `scripts/switch-to-process-plugins.sh`
from a terminal outside LiteAura after a release build.

A running server loads the new executable when it rescans the plugin
(`POST /api/plugins/rescan`, or the next start). To try the whole catalogue
locally:

```sh
LITEAURA_VERSION=0.2.8 PLUGIN_BASE_URL=file://$PWD/out scripts/build-plugin-archives.sh out
SHINY_PLUGIN_INDEX_URL=file://$PWD/out/index.json shiny
```
