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

A process plugin need not be written in Rust: the plugin builder's Python
kit speaks the same protocol ([builder.md](builder.md#python-plugins)).

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

Inside the process, all plugin code (tools, routes, hooks, SQLite) runs on the
plugin's **single** runtime thread, as it does for a library plugin
([runtime-abi.md](runtime-abi.md)). The Tokio runtime `run` builds only accepts
connections and hands requests to that thread, so it has two workers, not one
per CPU core. CPU-heavy work belongs in `tokio::task::spawn_blocking`, which
has its own pool.

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

## Packaging and publication

`scripts/stage-plugin.sh <name> <dest>` lays out one plugin as the server
loads it. `scripts/build-plugin-archives.sh` makes one
`<name>-<version>-linux-<arch>.tar.gz` per process plugin (all of them, or
those named in `PLUGIN_NAMES`) and an `index.json` of what it built.

**A plugin is published when its `version` in `plugin.toml` goes up on
`main`**, not with a server release. Raising the version is the act of
publishing, so:

- work in progress goes to `main` **without** touching the version: nothing
  is published and nobody receives it;
- when the plugin is good to ship, raise its version (and note it in the
  CHANGELOG): the push publishes it and every server picks it up within six
  hours;
- a plugin that only needs rebuilding (the SDK it is built with changed) is
  published the same way: raise its version.
- a **new** plugin has no published version, so its first push would publish
  it: while it is in development, put `publish = false` in its `plugin.toml`.
  It then lives on `main` without being offered or packed
  (`scripts/plugin-catalog.py`, `scripts/build-plugin-archives.sh`); remove
  the line when it is ready.

On such a push `.github/workflows/publish-plugins.yml`:

1. reads the published catalogue and finds the plugins whose version in the
   repository is newer (`scripts/plugin-catalog.py pending`);
2. builds only those, in the image the server is built in, and makes their
   archives;
3. merges them into the catalogue (`plugin-catalog.py merge`) and signs it
   with the LiteAura archive key (`index.json.sig`);
4. uploads archives, `index.json` and `index.json.sig` to the rolling GitHub
   release **`plugins`**:
   `https://github.com/askscience/shiny/releases/download/plugins/index.json`.

A server release (`publish.yml`, on a `v*` tag) no longer builds plugins.
The Debian package ships only terminal, updates and installer, plus
`/usr/share/shiny/default-plugins`, the list every account gets.

### `min_host`: a plugin that needs a newer server

A plugin's executable only needs the protocol, but its window is built from
the server's UI modules (`../../ui/index.js`, `web/js`, the core
stylesheets). A plugin version that uses something added to the server after
its last release must say so in `plugin.toml`:

```toml
min_host = "0.3.1"   # the first server release that has what this version uses
```

The catalogue then offers that version only to servers at 0.3.1 or newer.
It keeps the previous version for the others: an index may list several
versions of one plugin, each server takes the newest it can run
(`Index::offers`), and `merge` drops a version once no server would pick it.
A plugin without `min_host` is offered to every server that speaks its
protocol.

Servers up to 0.2.9 do not know `min_host`. They read
`https://askscience.github.io/shiny/plugins/index.json`, which every release
republishes unchanged from `packaging/plugins-legacy/` (the signed 0.2.9
catalogue): they keep the plugins of their release until the server is
updated.

## The catalogue

The Plugin manager's **Apps** section lists what the catalogue offers for
this server: matching architecture, a shared protocol version and a server
new enough (`min_host`). **Get** and **Update** download the archive, check
it against the SHA-256 in the signed index and install it with the ordinary
installer. **Remove** uninstalls it.

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
| `SHINY_PLUGIN_INDEX_URL` | the `plugins` release URL | another catalogue; a `file://` one needs no signature (a local build) |
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
from a terminal outside LiteAura after a release build. It is a one-time
step: its catalogue settings go in the systemd drop-in
`~/.config/systemd/user/shiny.service.d/30-plugin-catalog.conf`, which
survives logins (`~/.config/shiny/env` belongs to the session script, which
sets `SERVER_PORT` there at every login). Later plugin builds need only
`scripts/install-plugin-dev.sh <name>` and a rescan, no restart.

A running server loads the new executable when it rescans the plugin
(`POST /api/plugins/rescan`, or the next start). To try the whole catalogue
locally:

```sh
LITEAURA_VERSION=0.3.0 PLUGIN_BASE_URL=file://$PWD/out scripts/build-plugin-archives.sh out
SHINY_PLUGIN_INDEX_URL=file://$PWD/out/index.json shiny
```

`python3 scripts/test-plugin-catalog.py` tests the catalogue script (what is
pending, how versions are merged and dropped).
