# Live update

An update shows without a restart. Four pieces take part; each one is small
and can be read on its own.

## What changes on disk, and who notices

| Changed on disk | Who notices | What happens |
|---|---|---|
| A plugin's code (`plugin.toml`, the cdylib, `migrations/`, `skills/`) | `application::plugin_watch` every `PLUGIN_WATCH_SECS` (10), or `POST /api/plugins/rescan` | The plugin is registered again: migrations run, the old library is retired (kept mapped, never `dlclose`d), its tools and routes are replaced, the router is rebuilt. |
| A plugin's `web/` | The page, through the `digest` in `GET /api/plugins` and `/api/system/version` | The window's module URL changes (`plugin.js?v=<digest>`), so the next import loads the new code. A visible window is rebuilt in place only when its surface has `saveState()`/`restoreState()`; otherwise on close and reopen. |
| The core `web/` tree | The page, through `web` in `/api/system/version` | The desktop is stashed in `sessionStorage` and the page reloads itself once; every window comes back, also with "Remember my workspace" off. |
| `/usr/bin/shiny` (the server) | The Updates window after apt (`binaryReplaced` in `/api/system/version`) | `POST /api/system/reexec`: the server drains, closes its pool and `exec`s the new binary in the same process with the listening socket inherited (`SHINY_LISTEN_FD`). The pid does not change; the port never closes. |
| `/usr/bin/liteaura` (the kiosk shell) | The shell itself (`liteaura_core::binary`) | It exits with code 44 when the page posts `liteaura:locked`; the session script relaunches it at once, behind the locked screen. |

## Fingerprints

`src/plugins/digest.rs` hashes relative path, size and mtime of every regular
file — no content is read, so a walk of a thousand files costs a few
milliseconds. A plugin has a *code* digest and a *web* digest, combined into
the `digest` the page sees. The server binary's fingerprint (`build` in
`/api/system/version`) is taken once at startup; the file on disk is compared
with it to tell a replaced binary (`binaryReplaced`).

## The page: `web/js/liveUpdate.js`

- Every 20 s, and when the document becomes visible, it reads
  `/api/system/version` and compares with the last answer.
- `api.js` dispatches `server:unreachable` when a request cannot reach the
  server at all. The module then shows one "Reconnecting…" toast and polls
  every second; when the server answers again it dispatches
  `server:reconnected` (streams that need it reconnect on their own) and
  compares at once.
- A changed plugin digest → `invalidatePluginCatalog()` and `plugins:changed`
  with `detail.updated = [names]`; `tiles.js` does the rest.
- A changed `web` digest → `stashDesktopForReload()` then `location.reload()`.
  `preferences.getWorkspaces()` returns the stash exactly once in fresh mode,
  and `app.js` skips the fresh-mode plugin reset for that boot.
- A changed `build` alone changes nothing on the page.

## The server swap

`application::reexec`:

1. `POST /api/system/reexec` (loopback only, never from a remote client)
   sets `AppState::reexec`; `main` waits until no assistant turn is running
   (`TurnRegistry::running`, at most ten minutes).
2. The ordinary shutdown path runs: stop accepting, drain for up to 10 s,
   stop workers, close the pool.
3. The listening socket, dup'd at startup, loses close-on-exec and the
   process `exec`s `exe_path()` (the `/proc/self/exe` link with its
   ` (deleted)` suffix stripped) with `SHINY_LISTEN_FD=<fd>`.
4. The new `main` finds the variable, takes the socket with
   `inherited_listener()` instead of binding, and continues. Connections
   that arrived during the swap waited in the kernel's accept queue.

Child processes (speech sidecars) survive the `exec`; the new server finds
them on their ports as it does at any start.

## The Updates window

After a successful apt job `plugins/updates/web/plugin.js` calls
`/api/plugins/rescan`, then `/api/system/reexec` when the `liteaura` package
was upgraded or the server reports `binaryReplaced`, and writes each outcome
to the job log: which plugins were loaded again, which are shadowed by a copy
of the user's own (`overridden`), that the server switches when the assistant
is idle, that the shell relaunches at the next lock.

## Limits

- A plugin library must be **replaced, not rewritten**: write the new file
  under another name and rename it into place (dpkg, the plugin installer and
  `scripts/install-plugin-dev.sh` all do). `cp new.so old.so` rewrites the
  file the server has mapped and crashes it (reproduced: segmentation fault on
  the next shutdown).

- A plugin with background work must stop it in `on_unload`, or two copies run
  after a reload (the loader keeps the old library mapped).
- A reload `dlopen`s the new library under a fresh name, a hard link
  `.lib<name>.so.<nanos>` removed right after (`reload_path` in
  `src/plugins/loader.rs`). glibc hands back the library it already loaded for
  a name it has seen, so reopening the same path after an upgrade kept running
  the old code while the digest reported the new one. The first load of a path
  opens it directly.
- The loader keeps at most 100 retired libraries per process; past that a
  restart is needed (the next server swap does it).
- A window without `saveState()` shows the old code until it is reopened.
- An assistant answer in progress is never cut: the swap waits for it.
