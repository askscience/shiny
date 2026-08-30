//! Minimal synchronous SQLite access for plugin code.
//!
//! `ctx.pool()` (async `sqlx::SqlitePool`) runs each connection on its own
//! `sqlx-sqlite-worker-*` thread while the caller drops rows on the plugin
//! runtime thread — under load that can segfault in `sqlite3_value_free`. This
//! module instead exposes a **synchronous** connection (`libsqlite3-sys`
//! directly), so prepare/bind/step/finalize all happen on the one plugin
//! runtime thread and no value ever crosses a thread boundary.
//!
//! `Db` is `Send + Sync` by construction: `rt::bridge` drives every plugin on
//! a single dedicated thread, and the `Mutex` serializes any residual access.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::Mutex;

use libsqlite3_sys::{
    sqlite3, sqlite3_stmt, SQLITE_BLOB, SQLITE_DONE, SQLITE_INTEGER, SQLITE_NULL, SQLITE_OK,
    SQLITE_ROW,
};

use crate::errors::AppError;

/// A SQL value used as a bind parameter or returned as a column value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn text(s: impl Into<String>) -> Self {
        Value::Text(s.into())
    }
    pub fn blob(b: Vec<u8>) -> Self {
        Value::Blob(b)
    }
}

/// A plugin-owned synchronous SQLite connection.
pub struct Db {
    raw: Mutex<*mut sqlite3>,
}

// SAFETY: `Db` is only ever accessed from the plugin's single dedicated runtime
// thread (see `rt::bridge`); the Mutex serializes any residual access. The
// underlying connection is therefore never used concurrently.
unsafe impl Send for Db {}
unsafe impl Sync for Db {}

fn sqlite_path(url: &str) -> String {
    if let Some(p) = url.strip_prefix("sqlite://") {
        p.to_string()
    } else if let Some(p) = url.strip_prefix("sqlite:") {
        p.to_string()
    } else {
        url.to_string()
    }
}

fn errmsg(db: *mut sqlite3) -> String {
    unsafe { CStr::from_ptr(libsqlite3_sys::sqlite3_errmsg(db)) }
        .to_string_lossy()
        .into_owned()
}

impl Db {
    pub fn open(url: &str) -> Result<Self, AppError> {
        // Explicitly initialize the (plugin's own) sqlite before opening, so
        // its global config is set up regardless of the dlopen'd copy's
        // initialization order.
        let _ = unsafe { libsqlite3_sys::sqlite3_initialize() };
        let path = sqlite_path(url);
        let cpath = CString::new(path)
            .map_err(|_| AppError::Internal("invalid database path".into()))?;
        let mut raw: *mut sqlite3 = std::ptr::null_mut();
        let rc = unsafe { libsqlite3_sys::sqlite3_open(cpath.as_ptr(), &mut raw) };
        if rc != SQLITE_OK {
            let msg = if raw.is_null() {
                "failed to open database".to_string()
            } else {
                errmsg(raw)
            };
            if !raw.is_null() {
                unsafe { libsqlite3_sys::sqlite3_close(raw) };
            }
            return Err(AppError::Internal(msg));
        }
        Ok(Db { raw: Mutex::new(raw) })
    }

    /// Run a non-SELECT statement; returns the number of affected rows.
    pub fn execute(&self, sql: &str, params: &[Value]) -> Result<usize, AppError> {
        let guard = self.raw.lock().unwrap();
        let db = *guard;
        let mut stmt: *mut sqlite3_stmt = std::ptr::null_mut();
        let csql = CString::new(sql).map_err(|_| AppError::Internal("invalid SQL".into()))?;
        let rc = unsafe {
            libsqlite3_sys::sqlite3_prepare_v2(
                db,
                csql.as_ptr(),
                -1,
                &mut stmt,
                std::ptr::null_mut(),
            )
        };
        if rc != SQLITE_OK {
            return Err(AppError::Internal(format!("sqlite: {}", errmsg(db))));
        }
        bind_all(stmt, params)?;
        let step = unsafe { libsqlite3_sys::sqlite3_step(stmt) };
        if step != SQLITE_DONE && step != SQLITE_ROW {
            let e = AppError::Internal(format!("sqlite: {}", errmsg(db)));
            unsafe { libsqlite3_sys::sqlite3_finalize(stmt) };
            return Err(e);
        }
        let changes = unsafe { libsqlite3_sys::sqlite3_changes(db) } as usize;
        unsafe { libsqlite3_sys::sqlite3_finalize(stmt) };
        Ok(changes)
    }

    /// Run a SELECT; returns rows (each row is a column vector).
    pub fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Vec<Value>>, AppError> {
        let guard = self.raw.lock().unwrap();
        let db = *guard;
        let mut stmt: *mut sqlite3_stmt = std::ptr::null_mut();
        let csql = CString::new(sql).map_err(|_| AppError::Internal("invalid SQL".into()))?;
        let rc = unsafe {
            libsqlite3_sys::sqlite3_prepare_v2(
                db,
                csql.as_ptr(),
                -1,
                &mut stmt,
                std::ptr::null_mut(),
            )
        };
        if rc != SQLITE_OK {
            return Err(AppError::Internal(format!("sqlite: {}", errmsg(db))));
        }
        bind_all(stmt, params)?;

        let mut rows = Vec::new();
        loop {
            let step = unsafe { libsqlite3_sys::sqlite3_step(stmt) };
            match step {
                SQLITE_ROW => {
                    let count = unsafe { libsqlite3_sys::sqlite3_column_count(stmt) };
                    let mut row = Vec::with_capacity(count as usize);
                    for i in 0..count {
                        row.push(column(stmt, i));
                    }
                    rows.push(row);
                }
                SQLITE_DONE => break,
                _ => {
                    let e = AppError::Internal(format!("sqlite: {}", errmsg(db)));
                    unsafe { libsqlite3_sys::sqlite3_finalize(stmt) };
                    return Err(e);
                }
            }
        }
        unsafe { libsqlite3_sys::sqlite3_finalize(stmt) };
        Ok(rows)
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        let raw = *self.raw.get_mut().unwrap();
        if !raw.is_null() {
            unsafe { libsqlite3_sys::sqlite3_close(raw) };
        }
    }
}

fn bind_all(stmt: *mut sqlite3_stmt, params: &[Value]) -> Result<(), AppError> {
    for (i, p) in params.iter().enumerate() {
        let idx = (i + 1) as c_int;
        let rc = match p {
            Value::Null => unsafe { libsqlite3_sys::sqlite3_bind_null(stmt, idx) },
            Value::Int(n) => unsafe { libsqlite3_sys::sqlite3_bind_int64(stmt, idx, *n) },
            Value::Text(s) => {
                let cs = CString::new(s.as_str())
                    .map_err(|_| AppError::Internal("invalid text parameter".into()))?;
                unsafe {
                    libsqlite3_sys::sqlite3_bind_text(
                        stmt,
                        idx,
                        cs.as_ptr(),
                        -1,
                        libsqlite3_sys::SQLITE_TRANSIENT(),
                    )
                }
            }
            Value::Blob(b) => unsafe {
                libsqlite3_sys::sqlite3_bind_blob(
                    stmt,
                    idx,
                    b.as_ptr() as *const c_void,
                    b.len() as c_int,
                    libsqlite3_sys::SQLITE_TRANSIENT(),
                )
            },
        };
        if rc != SQLITE_OK {
            return Err(AppError::Internal(format!("sqlite bind failed at {}", idx)));
        }
    }
    Ok(())
}

fn column(stmt: *mut sqlite3_stmt, i: c_int) -> Value {
    let ty = unsafe { libsqlite3_sys::sqlite3_column_type(stmt, i) };
    match ty {
        SQLITE_NULL => Value::Null,
        SQLITE_INTEGER => Value::Int(unsafe { libsqlite3_sys::sqlite3_column_int64(stmt, i) }),
        SQLITE_BLOB => {
            let ptr = unsafe { libsqlite3_sys::sqlite3_column_blob(stmt, i) } as *const u8;
            let len = unsafe { libsqlite3_sys::sqlite3_column_bytes(stmt, i) } as usize;
            if ptr.is_null() || len == 0 {
                Value::Blob(Vec::new())
            } else {
                Value::Blob(unsafe { std::slice::from_raw_parts(ptr, len).to_vec() })
            }
        }
        _ => {
            // TEXT (and fallback for anything else).
            let ptr = unsafe { libsqlite3_sys::sqlite3_column_text(stmt, i) };
            if ptr.is_null() {
                Value::Null
            } else {
                let cstr = unsafe { CStr::from_ptr(ptr as *const c_char) };
                Value::Text(cstr.to_string_lossy().into_owned())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_and_query_file() {
        let tmp = std::env::temp_dir().join("shiny_dbtest.db");
        let _ = std::fs::remove_file(&tmp);
        let db = Db::open(&format!("sqlite:{}", tmp.display())).unwrap();
        db.execute("CREATE TABLE t (id INTEGER, name TEXT)", &[]).unwrap();
        db.execute("INSERT INTO t VALUES (?1, ?2)", &[Value::Int(1), Value::text("hi")]).unwrap();
        assert_eq!(db.query("SELECT name FROM t", &[]).unwrap()[0][0], Value::Text("hi".into()));
        drop(db);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn open_and_query_roundtrip() {
        let db = Db::open("sqlite::memory:").unwrap();
        db.execute("CREATE TABLE t (id INTEGER, name TEXT, data BLOB)", &[]).unwrap();
        db.execute(
            "INSERT INTO t (id, name, data) VALUES (?1, ?2, ?3)",
            &[Value::Int(1), Value::text("hello"), Value::blob(vec![1, 2, 3])],
        )
        .unwrap();
        let rows = db.query("SELECT id, name, data FROM t", &[]).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], Value::Int(1));
        assert_eq!(rows[0][1], Value::Text("hello".into()));
        assert_eq!(rows[0][2], Value::Blob(vec![1, 2, 3]));
    }
}
