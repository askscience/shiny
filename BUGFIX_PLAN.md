# Shiny — Bug-Fix & Cleanup Plan

> **Status (session 3):** ✅ Phase 0, ✅ Phase 1 (1.1–1.34), ✅ Phase 2,
> and ✅ Phase 3 are complete. Phase 3 resolutions: ① admin gating kept
> as-is (documented in PLUGINS.md §14); ② argon2 (done in Phase 0);
> ③ cookie hardening — `HttpOnly` + a server-side `/api/auth/logout`
> (token invalidation + cookie clear); ④ radio `/nowplaying` SSRF — reject
> non-global hosts (with unit tests); ⑤ timezone model — recommendation
> documented below (no behavior change). Verified:
> `cargo check --workspace --all-targets` exit 0 (only the 15 design-inherent
> `improper_ctypes_definitions` notes) and `cargo test --workspace` green.

Consolidated from a full review of core (`src/`), the plugin SDK
(`crates/shiny-plugin-sdk/`), all 14 plugins, and the web frontend (`web/`),
cross-checked with `cargo check --workspace --all-targets`. Every P0/P1 item
below was verified by reading the code; line numbers refer to the current
working tree (which includes uncommitted WIP — fixes apply on top of it).

Guiding rules: **no functionality removed** (dead code deleted only when
provably unreachable), WIP intent preserved, each phase ends with
`cargo check --workspace --all-targets` + `cargo test --workspace` green.

---

## Phase 0 — Crashes & security (P0)

| # | Fix | Files |
|---|-----|-------|
| 0.1 | **Use-after-free on plugin uninstall/reinstall.** Add `ToolRegistry::uninstall_plugin(name)` (remove every key owned by `name`, aliases included) and call it from `PluginManager::uninstall` **and** at the top of re-installs in `install_dir_static`. Never `dlclose` a library whose tools may still be referenced: move dropped `Library` handles into a process-lifetime graveyard `Vec<Library>` (leak-by-design, documented) instead of dropping them in `Loader::retain`. Fix the stale `unload()` doc comment. | `src/plugins/registry.rs`, `src/plugins/manager.rs`, `src/plugins/loader.rs` |
| 0.2 | **Path traversal via `manifest.name` on install.** Validate the name is a single safe path component (non-empty, no `/`, `\`, NUL, not `.`/`..`) before any `join`; reject with 400 otherwise. | `src/plugins/installer.rs` |
| 0.3 | **Arbitrary `remove_dir_all` via uninstall name.** Same safe-component validation for `body.name` in the uninstall handler (and activate/deactivate for consistency). | `src/plugins/admin_api.rs` |
| 0.4 | **Path traversal via diary `date`.** Validate `^\d{4}-\d{2}-\d{2}$` before `generate_for_date` writes `diaries/<date>.md`. | `src/services/diary_gen.rs` (or `src/api/diary.rs`) |
| 0.5 | **UTF-8 slice panic in diary summary** — `&content[..min(len,200)]` panics on multi-byte boundary. Use `content.chars().take(200).collect::<String>()`. Two copies: core and traveler plugin. | `src/services/diary_gen.rs:100`, `plugins/traveler/src/diary.rs:88` |
| 0.6 | **Plugin panic kills the shared plugin runtime.** Wrap `job.await` in `catch_unwind(AssertUnwindSafe(..))` inside the rt worker loop; keep the loop alive, return a panic error to the caller instead of hanging future calls. Correct the "one sender per plugin" comment (it is process-global by design). | `crates/shiny-plugin-sdk/src/rt.rs` |
| 0.7 | **Daily diary cron never fires.** `interval(3600s)` is anchored to process start, so `%H:%M == target` only matches if the server started in that minute. Compute the duration to the next target time and `sleep` → run → repeat (daily). | `src/main.rs` `spawn_diary_cron` |
| 0.8 | **Sphere long-press release never fires** (and the next tap is eaten). Track `longPressFired`; on release with a fired long press, call `onLongPressEnd` and reset `conversationMode`/state. | `web/js/sphere.js` |

## Phase 1 — Data loss & correctness (P1)

### Mail (in-flight WIP area — surgical fixes only)
| # | Fix | Files |
|---|-----|-------|
| 1.1 | **Re-sync wipes cached bodies.** In the envelope loop, skip the full `INSERT OR REPLACE` for already-cached UIDs (targeted `UPDATE … SET seen` instead); only new UIDs insert full rows. | `plugins/mail/src/mail.rs`, `plugins/mail/src/cache.rs` |
| 1.2 | **IMAP paging off-by-one** — io-email pages are 1-based; start `page = 1` (oldest mail currently never synced; newest fetched twice). | `plugins/mail/src/mail.rs:490` |
| 1.3 | **One bad message aborts the whole folder sync.** `fetch_bodies`: `continue` on `get_message`/`parse_message` errors (count skips). | `plugins/mail/src/mail.rs` |
| 1.4 | Open-message race guard (bail if selection changed); include account/user + html in the send-dedup fingerprint; add `user_id` filter to `cached_uids`/`sync_state`; update `seen` in cache on `set_seen`; read `sync_state` to stop re-syncing empty folders. | `plugins/mail/src/*`, `plugins/mail/web/plugin.js` |

### Office & documents
| # | Fix | Files |
|---|-----|-------|
| 1.5 | **Calc drops terms on mixed `+`/`-`** — merge the two while-loops into one left-associative loop. | `plugins/calc/web/plugin.js` `parseExpr` |
| 1.6 | **ODS sparse-row collapse** — iterate `1..=max_row`, emit `<table:table-row/>` for gaps so rows don't shift on round-trip. | `crates/shiny-plugin-sdk/src/ods.rs` |
| 1.7 | **Word markdown→ODT corrupts on `<`** — escape text nodes (`<`, `>`, `&`); remove the dead `'<' => "&lt;"` arm in `escape_and_collapse` by making it reachable/correct. | `crates/shiny-plugin-sdk/src/odt.rs`, `plugins/word/src/tools/mod.rs` |
| 1.8 | `calc_create` stringifies non-string cell values (numbers/bools currently make the whole sheet unreadable) + applies the same cell-ref/limit validation as `calc_write`. | `plugins/calc/src/tools/mod.rs` |
| 1.9 | Calc JS: case-insensitive function names (`=sum(...)` works); aggregates skip empty/text cells (COUNT/AVERAGE/MIN/MAX). | `plugins/calc/web/plugin.js` |
| 1.10 | Word window: reset `tileEl` in `unmountWordTile`; remove the `selectionchange` listener on unmount. Same pattern fix for radio (`unmountRadioTile` nulls refs) and PDF (remove body-level popups + their document listeners on unmount). | `plugins/word|radio|pdf/web/plugin.js` |

### Image
| # | Fix | Files |
|---|-----|-------|
| 1.11 | **Preview adjustments compound** — keep a committed baseline in `Session`; non-commit applies compute `baseline + ops` and never mutate the baseline; commit applies the same ops to the baseline and persists. | `plugins/image/src/session.rs`, `routes.rs` |
| 1.12 | Bounds/validation: clamp resize/crop dims to a sane max before `as u32`; validate `x/y` and finite `angle`; use `split('.').next()` for the title stem. | `plugins/image/src/ops.rs`, `routes.rs` |

### Core agent/chat/artifacts
| # | Fix | Files |
|---|-----|-------|
| 1.13 | **`/api/chat` sends the user message twice** and reads the oldest-20 window. Drop the duplicate append; select recent history `ORDER BY timestamp DESC LIMIT n` then reverse. | `src/api/chat.rs` |
| 1.14 | **`urlencoding` mangles non-ASCII** (`c as u8` truncates UTF-8) — four copies (core osm, SDK services, traveler osm, radio_browser). Fix each to encode `s.as_bytes()`; keep the SDK's `+`-for-space variant via a flag. | `src/services/osm.rs`, `crates/shiny-plugin-sdk/src/services.rs`, `plugins/traveler/src/osm.rs`, `plugins/radio/src/radio_browser.rs` |
| 1.15 | `merge_update` wipes `trip_id` — preserve the existing value like `existing_plugin_key` does. | `src/services/artifacts.rs` |
| 1.16 | Chat-memory ordering: `ORDER BY timestamp, rowid` (user/assistant rows share second-resolution timestamps) + index on `chat_messages(conversation_id, timestamp)`. | `src/services/chat_memory.rs`, migration |
| 1.17 | **Tools get a manifest-less `PluginCtx` and open a fresh DB pool per call.** Store the per-plugin `Arc<PluginCtx>` in the registry at `install_owned` time; `invoke` uses the owner's ctx (fixes `ctx.manifest.name == ""` artifact tagging and connection churn). `LoadedPlugin.ctx` stops being write-only. | `src/plugins/registry.rs`, `manager.rs`, `loader.rs` |
| 1.18 | **`on_load`/`on_unload` documented but never called** (no plugin implements them today → zero behavior change). Call `on_load` after successful registration; `on_unload` before removing a plugin (uninstall + reinstall). | `src/plugins/loader.rs`, `manager.rs` |
| 1.19 | Trip stats: count the last point's speed in `avg_speed`; same fix in the traveler plugin's `trip_stats`. | `src/api/trips.rs`, `plugins/traveler/src/tools/trips.rs` |
| 1.20 | Installer hardening: hold a real cross-process advisory lock (`fs2` or `flock` via libc) plus an in-process async mutex; restore `<name>.bak` when registration fails; skip `_staging-*` dirs in `discover_and_install`. | `src/plugins/installer.rs`, `manager.rs` |
| 1.21 | `/api/plugins` `enabled` flags disagree with `/api/plugins/active` for `session.remember=false` users — make `list` use `session_active_set`, delete the duplicated `active_plugin_set` helper. | `src/plugins/admin_api.rs` |
| 1.22 | `studio_update` returns `has_audio: false` while stale audio remains — report the true state (and clear `wav`/`duration_ms` when config changes). Sort automation points by beat in `parse_arrangement`. Fix the mono WAV writer (header/loop mismatch) for correctness. Align JS `DEFAULT_LEVEL` with Rust `default_level` (clap/sub/drumkit). | `plugins/studio/src/*`, `web/plugin.js` |
| 1.23 | Traveler: scope the artifact upsert conflict to the owner (`WHERE saved_artifacts.traveler_id = excluded.traveler_id`). | `plugins/traveler/src/artifact_store.rs` |
| 1.24 | SDK `db.rs`: finalize the prepared statement when `bind_all` fails (leak); avoid `unwrap` on a poisoned mutex in `Drop`. | `crates/shiny-plugin-sdk/src/db.rs` |

### Frontend (desktop shell)
| # | Fix | Files |
|---|-----|-------|
| 1.25 | Stop GPS tracking (+ clear active trip id) when the traveler plugin deactivates. | `web/js/app.js`, `gps.js` |
| 1.26 | Voice model-download card is appended to a `hidden` container — unhide on append; don't wipe it on insight re-render. | `web/js/voice.js`, `insights/insightCards.js` |
| 1.27 | Context-menu submenus can't reopen after a sibling closes them — null `subEl`/`__openSub` on close (or check `isConnected`). | `web/js/contextMenu.js` |
| 1.28 | Stale reply timers hide newer replies — generation counter for `#reply-text` timeouts. Same pattern for insight-fetch and chat-history-view races. | `web/js/agent.js`, `insights/insightCards.js`, `chatHistory.js` |
| 1.29 | `loadActiveRoute` null-map early return (silent TypeError every 60 s in chat-only mode). | `web/js/map.js` |
| 1.30 | Preferences flush: keep failed keys in `dirty` (currently lost forever on a transient error). | `web/js/preferences.js` |
| 1.31 | Cap AI-driven workspace index (unbounded `pushWorkspace` loop). | `web/js/desktop.js` |
| 1.32 | Calendar `renderGrid` null guard; mail `openMessage` stale-fetch guard (1.4); calculator keypad: every digit resets `lastResult`. | `plugins/calendar/web/plugin.js`, `plugins/calculator/web/plugin.js` |
| 1.33 | `youtube_play` dispatches `plugin:focus` before the `tileEl` guard (or store a pending video played on mount) so "activate + play" works in one turn. | `plugins/youtube/web/plugin.js` |
| 1.34 | Radio `metaint + 4080` → `saturating_add`. | `plugins/radio/src/routes.rs` |

## Phase 2 — Dead code, warnings, duplication (P2)

- **Compiler warnings** (verified via `cargo check`): unused imports (`registry.rs` ParamHelpers, `installer.rs` Read, `admin_api.rs` IntoResponse, `agent_tools.rs` SdkActionOutcome, plus test-only ones), `let mut manifest` in `loader.rs`, dead `prev_was_tag` writes in `odt.rs:591-650`, unused `k`/`ctx` in studio, `RelatedTopic` gets `#[allow(non_snake_case)]` (serde matches DuckDuckGo's PascalCase JSON — do **not** rename).
- **Dead functions to delete** (grep-proven unused): `parse_event_cards` (move under `#[cfg(test)]`), gpsd `get_current_position`/`to_location` + unused `_connected`, `ToolRegistry::install`, mail `list_envelopes`/`search_envelopes`/`email_err`/`presets`, `cache::sync_state` (or wire it per 1.4), pdf `ops::extract`, calendar `date::today()`, SDK `entry_symbol()`, studio dead ParamDef tables/`fx_defaults`/`GRID_MODULES`/`CompiledGrid.modulations`/`Rendered.frames` (keep `channels` for tests), frontend dead exports (`initHudLeft`, `changeLanguage`, `getListenMode`, `setOrbCaption`, `clearAgentUI`, `revealNow`, `getMapTileElement`, `isKeyboardActive`, `getActiveTripId`, `getCurrentArtifact`, `isConversationMode` import), `#nav-banner` line + `.ui-nav-banner` CSS (3 copies).
- **Manifest dead fields** (`entry_symbol`, `skills_dir`, `signature`): keep for forward-compat (documented in PLUGINS.md), read `entry_symbol` in the loader when present (restores documented meaning), `#[allow(dead_code)]` + comment on the rest.
- **Unused workspace deps**: remove `walkdir`, `tokio-util` from the root `Cargo.toml`.
- **Dedupe**: one `log_event` (installer + admin_api copies), one `find_cdylib` (loader + installer copies), shared `validate_username`/`validate_avatar` (auth.rs vs travelers.rs — keep travelers' allow-empty semantics).
- **Stale comments**: loader `unload` doc (fixed in 0.1), `TeeMakeWriter` "independent handle" → "cloned append-mode handle", admin_api "enabled by default" comment (fixed in 1.21).

## Phase 3 — Decision items (behavior changes — need owner sign-off)

1. **Plugin install/uninstall admin gating** — ✅ *Resolved: keep as-is.*
   Any logged-in user can install native code (documented in PLUGINS.md §14
   as intended). Path-traversal is closed by 0.2–0.3 regardless.
2. **Password hashing** — ✅ *Resolved: argon2id* (Phase 0), with rehash-on-login
   for legacy SHA-256 accounts.
3. **Session cookie hardening** — ✅ *Implemented.* `shiny_token` is now
   `HttpOnly`; since JS can't clear an `HttpOnly` cookie, a server-side
   `POST /api/auth/logout` (nulls `auth_token`, returns `Set-Cookie: …Max-Age=0`)
   was added and both logout paths (auth page + settings page) call it. The
   localStorage bearer token is unchanged — the cookie remains the reload
   fallback.
4. **Radio `/nowplaying` SSRF** — ✅ *Implemented.* The proxy now resolves the
   stream host and rejects any non-globally-routable address (loopback,
   private, link-local, CGNAT 100.64/10, documentation, benchmarking,
   reserved, IPv6 link-local/ULA/mapped-IPv4). Unit-tested. Known residual:
   DNS rebinding between the check and the fetch is out of scope.
5. **Calendar/diary timezone model** — 📝 *Recommendation (no code change yet).*
   Diary (`diary_gen.rs`, `api/diary.rs`) and calendar (`date.rs`) use
   `chrono::Local::now()` for "today", so date boundaries depend on the
   server's timezone. Recommended follow-up: store `YYYY-MM-DD` in UTC and
   convert to the browser's timezone for display; accept the boundary
   explicitly as UTC to keep "today" deterministic across restarts and
   deployments.

Deliberately **not** in scope (roadmap items in PLUGINS.md §20): ed25519 signature enforcement, cron scheduling (`CronSpec`), per-plugin runtime isolation, cross-allocator plugin ABI.

## Verification

- After each phase: `cargo check --workspace --all-targets` (zero new warnings), `cargo test --workspace`, `cargo build --release` at the end.
- Rust fixes with logic worth pinning get unit tests: diary summary truncation, urlencoding, ODS sparse rows, calc validation, image op bounds, WAV header mono, trip avg speed, chat history ordering.
- Manual smoke: boot the server (Ollama absent → graceful degradation), hit `/api/voice/languages`, register/login, plugin list/activate; browser pass for the frontend gesture/menu/calc fixes.

## Effort estimate

Phase 0 ≈ 1 focused session; Phase 1 ≈ 2–3 sessions (mail + image + frontend batches); Phase 2 ≈ 1 session (mostly mechanical); Phase 3 per decision.
