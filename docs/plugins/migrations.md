# Plugin migrations

Each plugin owns its tables and applies its schema through SQL files in
`plugins/<name>/migrations/`. The core runs them **host-side** at load time with
the host pool and records what it applied.

Source: [`crates/shiny-plugin-sdk/src/migrations.rs`](../../crates/shiny-plugin-sdk/src/migrations.rs),
[`src/plugins/loader.rs`](../../src/plugins/loader.rs).

---

## How it works

`run_plugin_migrations(pool, plugin, dir)`:

1. Ensures the core-owned `plugin_schema_versions` table exists:

   ```sql
   CREATE TABLE IF NOT EXISTS plugin_schema_versions (
       plugin TEXT NOT NULL,
       file   TEXT NOT NULL,
       applied_at TEXT NOT NULL DEFAULT (datetime('now')),
       PRIMARY KEY (plugin, file)
   );
   ```

2. Lists `dir/*.sql`, sorted **lexicographically** (so number your files
   `001_`, `002_`, …).
3. For each file not already recorded for `plugin`, runs it and inserts a row.

It returns the names of the files it applied.

---

## The `plugin_schema_versions` contract

- The table is **core-owned**; plugins never manage migration state themselves.
- Re-installing a plugin runs only **new** files (already-recorded files are
  skipped).

### The immutability rule (important)

**An applied migration file is immutable.** The runner records it by *name*;
editing its contents afterwards does **not** re-run it and does **not** warn.
Two consequences have both bitten this repo:

1. A fix written into an already-applied file is silently lost on every install
   that has applied it (fresh installs get it, existing ones never do). Add a
   **new, higher-numbered** file instead.
2. A later file's `CREATE TABLE IF NOT EXISTS` is a **no-op** if an earlier file
   already created that table. If the first version of the table lacked a column
   the code now writes, the code fails at runtime against the old shape — and a
   plugin that swallows DB errors fails *silently*.

Check that replaying your files produces every column your code names.

---

## Idempotency

Migrations should be safe to replay when a partial install is retried. Use
`IF NOT EXISTS`:

```sql
-- 001_init.sql
CREATE TABLE IF NOT EXISTS tips (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    body TEXT NOT NULL
);
```

SQLite has limited `ALTER TABLE` support. For a structural change, use the
table-recreate-and-copy pattern in a new numbered file.

---

## Repairing a bad shape in code

When a repair genuinely cannot be expressed as DDL (SQLite has no conditional
`ALTER TABLE … ADD COLUMN`), do it in code from `on_load`, where a "duplicate
column" result can be treated as success. The reference is
[`plugins/browser/src/history.rs::ensure_schema`](../../plugins/browser/src/history.rs):

```rust
// Pseudocode: add a column only if missing.
let has = ctx.db().query(
    "SELECT COUNT(*) FROM pragma_table_info('peakd_history') WHERE name = 'kind'",
    &[],
)?;
if has[0][0] == DbValue::Int(0) {
    match ctx.db().execute("ALTER TABLE peakd_history ADD COLUMN kind TEXT", &[]) {
        Ok(_) => {}
        // Race / already added by a replay: treat as success.
        Err(_) => {}
    }
}
```

Do this from `on_load` (runs once per load) rather than a migration file.

---

## Uninstall

Uninstalling a plugin **leaves its tables in place** — rolling back structural
schema changes is the operator's responsibility. To clean up:

```sql
DROP TABLE my_plugin_xxx;
```

Uninstalling does not delete rows from `plugin_schema_versions`, so
re-installing the same files will skip them (correct: the tables are still
there). If you drop the tables yourself, also clear the rows for that plugin if
you intend to re-run the files.

---

## Naming

```
migrations/
├── 001_init.sql
├── 002_add_notes.sql
└── 003_indexes.sql
```

Keep them small and append-only. Never edit an applied file; always add the
next number.
