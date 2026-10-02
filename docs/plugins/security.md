# Plugin security

Plugins are native code. This page states the trust model honestly, then
describes the install audit trail and the (currently inactive) signature hook.

---

## Trust model

- Plugins are full Rust `cdylib`s loaded into the server process. They have **full
  process privileges** — they can read SQLite rows, make HTTP calls, spawn
  threads and write files. **Do not install plugins you do not trust.**
- Plugin management (install/uninstall/activate/deactivate) is gated only by a
  **logged-in user token** — there is currently **no admin role**. The
  `ADMIN_TOKEN` env var and the `travelers.is_admin` column are read but **not
  enforced** by core routing. For a single-tenant server that is acceptable;
  treat every account as able to load native code.
- **Activation** is per-user (`user_plugin_states`); **installation** is
  server-wide (the shared cdylib). A user enabling a plugin enables it for
  themselves only, but anyone can install one for the whole server.
- The install log enables post-incident forensics but does not prevent a
  malicious plugin.

For multi-tenant hosting, plan to (a) require signature validation and (b)
sandbox long-running plugins behind an IPC process boundary. Both are roadmap;
the current system is single-tenant.

---

## Install log

Every install attempt — successful or failed — is appended to
**`PLUGINS_DIR/install.log`** (default `data/plugins/install.log`). It is the
audit trail for diagnosing failures offline and is exposed at
`GET /api/plugins/install.log` (last 200 lines).

Format: `[YYYY-MM-DD HH:MM:SS] <event-tag> key=value…`

```
[2026-07-12 20:33:50] install-begin bytes=1326462
[2026-07-12 20:33:50] format-detected format=TarGz
[2026-07-12 20:33:51] manifest-ok name=hello version=0.1.0 api_level=1
[2026-07-12 20:33:51] install-ok name=hello
[2026-07-12 20:33:52] install-begin bytes=15
[2026-07-12 20:33:52] reject-format unknown-bytes
[2026-07-12 20:35:01] uninstall-ok name=hello
```

Event tags:

| Tag | Meaning |
|---|---|
| `install-begin` | Upload received; logging starts. |
| `format-detected` | Magic-byte sniff succeeded. |
| `reject-format` | Unrecognised archive bytes. |
| `manifest-ok` | `plugin.toml` parsed. |
| `manifest-read-failed` | No `plugin.toml` at archive root. |
| `manifest-parse-failed` | `toml::from_str` error. |
| `reject api_level` | Plugin requires a newer API than the running core. |
| `cdylib-missing` | Archive has no `.so`/`.dylib`/`.dll`. |
| `rename-failed` | Couldn't move staging into place. |
| `extract-failed` | zip/tar unpack error. |
| `register-failed` | dlopen, missing symbol, migration, or `register()` error. |
| `install-ok` | Plugin loaded and registered. |
| `uninstall-ok` | Plugin name was installed and is now absent. |
| `uninstall-missing` | Uninstall for a plugin that wasn't installed. |

The lock file for installs is `PLUGINS_DIR/.install.lock`.

---

## Signature verification (optional, not enforced)

The manifest has an optional `signature` field: a hex-encoded ed25519 signature
of `plugin.toml + lib<name>.{so|dylib|dll}`. Intended flow:

1. Generate a keypair; publish the public key in the running server's
   `TRUSTED_PUBKEYS` env (semicolon-separated hex).
2. The author signs `plugin.toml` plus the cdylib, storing the hex in
   `signature`.
3. The installer rejects a plugin whose signature is present but doesn't verify,
   unless `--insecure` is passed via the `X-Shiny-Insecure` header.

> This is **off by default**: the decoder is not yet wired into the loader. The
> field exists so plugins can prepare; enforcement is expected to flip on at
> `api_level 2`.

---

## Remote clients

A remote client (Iroh/Tailscale) cannot change host state; see
[remote access](../deployment/remote-access.md). Plugin routes are mounted with
auth middleware unless the spec declares `public`; the Terminal plugin is
additionally refused for remote clients unless explicitly allowed.

---

## Practical guidance

- Install only plugins you built or trust the author of.
- Read `install.log` after any install that misbehaves.
- Prefer per-user activation; remember it does not isolate the native code.
- Treat `ADMIN_TOKEN`/`is_admin` as informational, not a security boundary.
