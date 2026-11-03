//! `file_shares`: which Shiny account owns each Gokapi file id
//! (migration `001_file_shares.sql`).

use serde::Serialize;
use shiny_plugin_sdk::db::{Db, Value};
use shiny_plugin_sdk::errors::AppError;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Share {
    pub id: String,
    pub path: String,
    pub name: String,
    pub url: String,
    pub size_bytes: i64,
    /// Unix seconds; 0 = no time limit.
    pub expires_at: i64,
    /// 0 = unlimited.
    pub max_downloads: i64,
    pub has_password: bool,
    pub created_at: String,
}

/// Create the table on an install whose migration has not run (same pattern
/// as the browser plugin's `settings::ensure_schema`).
pub fn ensure_schema(db: &Db) {
    let _ = db.execute(
        "CREATE TABLE IF NOT EXISTS file_shares (\
             id TEXT PRIMARY KEY, owner TEXT NOT NULL, path TEXT NOT NULL, name TEXT NOT NULL, \
             url TEXT NOT NULL, size_bytes INTEGER NOT NULL DEFAULT 0, \
             expires_at INTEGER NOT NULL DEFAULT 0, max_downloads INTEGER NOT NULL DEFAULT 0, \
             has_password INTEGER NOT NULL DEFAULT 0, \
             created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        &[],
    );
    let _ = db.execute(
        "CREATE INDEX IF NOT EXISTS file_shares_owner ON file_shares(owner)",
        &[],
    );
}

pub fn insert(db: &Db, owner: &str, s: &Share) -> Result<(), AppError> {
    db.execute(
        "INSERT OR REPLACE INTO file_shares \
         (id, owner, path, name, url, size_bytes, expires_at, max_downloads, has_password) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            Value::text(s.id.clone()),
            Value::text(owner),
            Value::text(s.path.clone()),
            Value::text(s.name.clone()),
            Value::text(s.url.clone()),
            Value::Int(s.size_bytes),
            Value::Int(s.expires_at),
            Value::Int(s.max_downloads),
            Value::Int(s.has_password as i64),
        ],
    )
    .map(|_| ())
}

fn text(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        _ => String::new(),
    }
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Int(i) => *i,
        Value::Text(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

/// The owner's shares, newest first.
pub fn list_for(db: &Db, owner: &str) -> Result<Vec<Share>, AppError> {
    let rows = db.query(
        "SELECT id, path, name, url, size_bytes, expires_at, max_downloads, has_password, created_at \
         FROM file_shares WHERE owner = ? ORDER BY created_at DESC, rowid DESC",
        &[Value::text(owner)],
    )?;
    Ok(rows
        .iter()
        .filter(|r| r.len() >= 9)
        .map(|r| Share {
            id: text(&r[0]),
            path: text(&r[1]),
            name: text(&r[2]),
            url: text(&r[3]),
            size_bytes: int(&r[4]),
            expires_at: int(&r[5]),
            max_downloads: int(&r[6]),
            has_password: int(&r[7]) != 0,
            created_at: text(&r[8]),
        })
        .collect())
}

/// True when `owner` owns share `id`.
pub fn owns(db: &Db, owner: &str, id: &str) -> Result<bool, AppError> {
    let rows = db.query(
        "SELECT 1 FROM file_shares WHERE id = ? AND owner = ?",
        &[Value::text(id), Value::text(owner)],
    )?;
    Ok(!rows.is_empty())
}

pub fn delete(db: &Db, owner: &str, id: &str) -> Result<(), AppError> {
    db.execute(
        "DELETE FROM file_shares WHERE id = ? AND owner = ?",
        &[Value::text(id), Value::text(owner)],
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (Db, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "shiny-shares-{}.sqlite",
            super::super::random_alnum(8)
        ));
        let db = Db::open(&format!("sqlite://{}", path.display())).unwrap();
        ensure_schema(&db);
        (db, path)
    }

    fn share(id: &str) -> Share {
        Share {
            id: id.into(),
            path: "Documents/a.txt".into(),
            name: "a.txt".into(),
            url: format!("https://h/d?id={id}"),
            size_bytes: 10,
            expires_at: 0,
            max_downloads: 3,
            has_password: true,
            created_at: String::new(),
        }
    }

    #[test]
    fn shares_are_scoped_to_their_owner() {
        let (db, path) = temp_db();
        insert(&db, "alice", &share("a1")).unwrap();
        insert(&db, "bob", &share("b1")).unwrap();

        let mine = list_for(&db, "alice").unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].id, "a1");
        assert!(mine[0].has_password);
        assert_eq!(mine[0].max_downloads, 3);

        assert!(owns(&db, "alice", "a1").unwrap());
        assert!(!owns(&db, "alice", "b1").unwrap());

        // Bob cannot delete Alice's row.
        delete(&db, "bob", "a1").unwrap();
        assert_eq!(list_for(&db, "alice").unwrap().len(), 1);
        delete(&db, "alice", "a1").unwrap();
        assert!(list_for(&db, "alice").unwrap().is_empty());
        let _ = std::fs::remove_file(path);
    }
}
