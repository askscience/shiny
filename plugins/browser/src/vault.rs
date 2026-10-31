//! The Browser's password vault: site logins sealed under the user's own
//! login password.
//!
//! # Keys
//!
//! - The **data key** is 32 random bytes made when the vault is created. Every
//!   entry is sealed under it with AES-256-GCM.
//! - The **key-encryption key** is the account's login password stretched with
//!   Argon2id (per-account salt, OWASP's 19 MiB / 2 passes / 1 lane). It seals
//!   the data key, so a changed login password re-seals 32 bytes instead of
//!   every entry.
//! - Each sealed value is bound to its owner and slot through the AEAD's
//!   associated data, so a row copied to another account or id never opens.
//!
//! The vault is created only with a password core confirms is the account's
//! login password (`POST /api/auth/verify-password`, which covers PAM accounts
//! too); after that, opening it needs nothing but the password itself: a wrong
//! one simply fails to open the data key.
//!
//! # Unlocked state
//!
//! An open vault keeps its data key in this process's memory only (zeroized on
//! drop), per account, and locks itself after [`IDLE_LOCK`] without use. A
//! restart of the server or the plugin locks every vault.
//!
//! Unlike history and bookmarks this module is **not** best-effort: a login
//! that silently failed to save is worse than an error, so database failures
//! surface to the window.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::services::PluginCtx;

/// The vault's key material and cost, one row per account.
pub const VAULT_TABLE: &str = "browser_vault";
/// The sealed entries.
pub const ENTRY_TABLE: &str = "browser_passwords";

/// An open vault locks itself after this long without use.
pub const IDLE_LOCK: Duration = Duration::from_secs(15 * 60);

/// Argon2id cost for new vaults (OWASP's recommended minimum). Stored per row,
/// so raising it later leaves existing vaults readable.
const KDF_M_COST: u32 = 19_456;
const KDF_T_COST: u32 = 2;
const KDF_P_COST: u32 = 1;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const SALT_LEN: usize = 16;

type Key = Zeroizing<[u8; KEY_LEN]>;

/// Ensure both tables exist on an install whose migration has not run.
pub fn ensure_schema(ctx: &PluginCtx) {
    let _ = ctx.db().execute(
        "CREATE TABLE IF NOT EXISTS browser_vault (\
             traveler_id TEXT PRIMARY KEY, \
             salt TEXT NOT NULL, \
             m_cost INTEGER NOT NULL, \
             t_cost INTEGER NOT NULL, \
             p_cost INTEGER NOT NULL, \
             wrapped_key TEXT NOT NULL, \
             created_at TEXT NOT NULL DEFAULT (datetime('now')), \
             updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        &[],
    );
    let _ = ctx.db().execute(
        "CREATE TABLE IF NOT EXISTS browser_passwords (\
             id TEXT PRIMARY KEY, \
             traveler_id TEXT NOT NULL, \
             sealed TEXT NOT NULL, \
             created_at TEXT NOT NULL DEFAULT (datetime('now')), \
             updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        &[],
    );
}

/* ── Crypto ──────────────────────────────────────────────────────── */

fn random<const N: usize>() -> Result<[u8; N], AppError> {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf)
        .map_err(|e| AppError::Internal(format!("no system randomness: {e}")))?;
    Ok(buf)
}

/// Stretch `password` into a key-encryption key.
fn derive(password: &str, salt: &[u8], cost: KdfCost) -> Result<Key, AppError> {
    let params = Params::new(cost.m, cost.t, cost.p, Some(KEY_LEN))
        .map_err(|e| AppError::Internal(format!("vault cost: {e}")))?;
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|e| AppError::Internal(format!("vault key: {e}")))?;
    Ok(key)
}

/// AES-256-GCM, returned as base64 `nonce || ciphertext`.
fn seal(key: &[u8; KEY_LEN], plaintext: &[u8], aad: &str) -> Result<String, AppError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| AppError::Internal("vault key length".into()))?;
    let nonce = random::<NONCE_LEN>()?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: aad.as_bytes() })
        .map_err(|_| AppError::Internal("could not seal".into()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(B64.encode(out))
}

/// The inverse of [`seal`]; `None` for a wrong key, a tampered value or a
/// value sealed for another slot.
fn open(key: &[u8; KEY_LEN], sealed: &str, aad: &str) -> Option<Zeroizing<Vec<u8>>> {
    let raw = B64.decode(sealed).ok()?;
    if raw.len() <= NONCE_LEN {
        return None;
    }
    let (nonce, ct) = raw.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    cipher
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: aad.as_bytes() })
        .ok()
        .map(Zeroizing::new)
}

fn key_aad(user_id: &str) -> String {
    format!("browser-vault:v1:key:{user_id}")
}

fn entry_aad(user_id: &str, id: &str) -> String {
    format!("browser-vault:v1:entry:{user_id}:{id}")
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct KdfCost {
    m: u32,
    t: u32,
    p: u32,
}

const DEFAULT_COST: KdfCost = KdfCost { m: KDF_M_COST, t: KDF_T_COST, p: KDF_P_COST };

/// The stored half of a vault: everything but the password.
#[derive(Debug, Clone)]
struct Sealed {
    salt: Vec<u8>,
    cost: KdfCost,
    wrapped_key: String,
}

impl Sealed {
    /// A new vault for `user_id`, its data key sealed under `password`.
    fn create(user_id: &str, password: &str) -> Result<(Self, Key), AppError> {
        let data_key = Zeroizing::new(random::<KEY_LEN>()?);
        let sealed = Self::wrap(user_id, password, &data_key, DEFAULT_COST)?;
        Ok((sealed, data_key))
    }

    /// Seal `data_key` under `password` with a fresh salt.
    fn wrap(user_id: &str, password: &str, data_key: &Key, cost: KdfCost) -> Result<Self, AppError> {
        let salt = random::<SALT_LEN>()?.to_vec();
        let kek = derive(password, &salt, cost)?;
        let wrapped_key = seal(&kek, data_key.as_ref(), &key_aad(user_id))?;
        Ok(Self { salt, cost, wrapped_key })
    }

    /// The data key, or `None` when `password` is not the one it was sealed with.
    fn unwrap(&self, user_id: &str, password: &str) -> Result<Option<Key>, AppError> {
        let kek = derive(password, &self.salt, self.cost)?;
        Ok(open(&kek, &self.wrapped_key, &key_aad(user_id)).and_then(|raw| {
            let bytes: [u8; KEY_LEN] = raw.as_slice().try_into().ok()?;
            Some(Zeroizing::new(bytes))
        }))
    }
}

/* ── Unlocked vaults ─────────────────────────────────────────────── */

struct OpenVault {
    key: Key,
    last_used: Instant,
}

static OPEN: LazyLock<Mutex<HashMap<String, OpenVault>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The open vault's data key, refreshing its idle timer; `None` when locked
/// (or idle for longer than [`IDLE_LOCK`], which locks it now).
fn data_key(user_id: &str) -> Option<Key> {
    let mut open = OPEN.lock();
    let expired = open
        .get(user_id)
        .is_some_and(|v| v.last_used.elapsed() >= IDLE_LOCK);
    if expired {
        open.remove(user_id);
        return None;
    }
    let vault = open.get_mut(user_id)?;
    vault.last_used = Instant::now();
    Some(Zeroizing::new(*vault.key))
}

fn keep_open(user_id: &str, key: Key) {
    OPEN.lock().insert(user_id.to_string(), OpenVault { key, last_used: Instant::now() });
}

/// Forget the data key (the window's lock button, a reset).
pub fn lock(user_id: &str) {
    OPEN.lock().remove(user_id);
}

pub fn is_unlocked(user_id: &str) -> bool {
    let open = OPEN.lock();
    open.get(user_id).is_some_and(|v| v.last_used.elapsed() < IDLE_LOCK)
}

/// Failed unlocks per account inside [`ATTEMPT_WINDOW`] before a cool-down.
const MAX_ATTEMPTS: usize = 8;
const ATTEMPT_WINDOW: Duration = Duration::from_secs(5 * 60);

static ATTEMPTS: LazyLock<Mutex<HashMap<String, Vec<Instant>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Whether another unlock attempt is allowed right now. Each one costs an
/// Argon2 derivation (19 MiB), so this is also what keeps the endpoint from
/// being a memory-exhaustion primitive.
fn attempt_allowed(user_id: &str) -> bool {
    let mut attempts = ATTEMPTS.lock();
    let list = attempts.entry(user_id.to_string()).or_default();
    list.retain(|t| t.elapsed() < ATTEMPT_WINDOW);
    list.len() < MAX_ATTEMPTS
}

fn record_failure(user_id: &str) {
    ATTEMPTS.lock().entry(user_id.to_string()).or_default().push(Instant::now());
}

fn clear_failures(user_id: &str) {
    ATTEMPTS.lock().remove(user_id);
}

/* ── Storage ─────────────────────────────────────────────────────── */

fn load_sealed(ctx: &PluginCtx, user_id: &str) -> Result<Option<Sealed>, AppError> {
    let rows = ctx.db().query(
        &format!(
            "SELECT salt, m_cost, t_cost, p_cost, wrapped_key FROM {VAULT_TABLE} \
             WHERE traveler_id = ?1 LIMIT 1"
        ),
        &[Value::text(user_id.to_string())],
    )?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let mut it = row.into_iter();
    let salt = match it.next() {
        Some(Value::Text(s)) => B64
            .decode(s)
            .map_err(|_| AppError::Internal("vault salt is corrupt".into()))?,
        _ => return Err(AppError::Internal("vault salt is missing".into())),
    };
    let int = |v: Option<Value>| match v {
        Some(Value::Int(n)) => u32::try_from(n).ok(),
        _ => None,
    };
    let cost = KdfCost {
        m: int(it.next()).unwrap_or(KDF_M_COST),
        t: int(it.next()).unwrap_or(KDF_T_COST),
        p: int(it.next()).unwrap_or(KDF_P_COST),
    };
    let wrapped_key = match it.next() {
        Some(Value::Text(s)) => s,
        _ => return Err(AppError::Internal("vault key is missing".into())),
    };
    Ok(Some(Sealed { salt, cost, wrapped_key }))
}

fn store_sealed(ctx: &PluginCtx, user_id: &str, sealed: &Sealed) -> Result<(), AppError> {
    ctx.db().execute(
        &format!(
            "INSERT INTO {VAULT_TABLE} (traveler_id, salt, m_cost, t_cost, p_cost, wrapped_key, \
                 created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now')) \
             ON CONFLICT(traveler_id) DO UPDATE SET salt = excluded.salt, \
                 m_cost = excluded.m_cost, t_cost = excluded.t_cost, p_cost = excluded.p_cost, \
                 wrapped_key = excluded.wrapped_key, updated_at = datetime('now')"
        ),
        &[
            Value::text(user_id.to_string()),
            Value::text(B64.encode(&sealed.salt)),
            Value::Int(i64::from(sealed.cost.m)),
            Value::Int(i64::from(sealed.cost.t)),
            Value::Int(i64::from(sealed.cost.p)),
            Value::text(sealed.wrapped_key.clone()),
        ],
    )?;
    Ok(())
}

pub fn exists(ctx: &PluginCtx, user_id: &str) -> Result<bool, AppError> {
    Ok(load_sealed(ctx, user_id)?.is_some())
}

/* ── Unlock ──────────────────────────────────────────────────────── */

/// What an unlock attempt came to. Never an HTTP 401: the app treats that
/// status as an expired session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unlock {
    /// The vault is open (and was created if it did not exist).
    Unlocked,
    /// Not the password this vault, or this account, uses.
    WrongPassword,
    /// The typed password is the account's *current* login password, but the
    /// vault was sealed under an earlier one: ask for that once to re-seal.
    PasswordChanged,
    /// Too many failed attempts; try again in a few minutes.
    TooManyAttempts,
}

/// Open (or first create) `user_id`'s vault with `password`.
///
/// `confirm` asks core whether a password is the account's login password; it
/// is consulted to create a vault, to tell a changed login password from a
/// typo, and before re-sealing under a new one. `previous` is the earlier
/// login password, sent once after a [`Unlock::PasswordChanged`].
pub async fn unlock<F, Fut>(
    ctx: &PluginCtx,
    user_id: &str,
    password: &str,
    previous: Option<&str>,
    confirm: F,
) -> Result<Unlock, AppError>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<bool, AppError>>,
{
    if !attempt_allowed(user_id) {
        return Ok(Unlock::TooManyAttempts);
    }
    let Some(sealed) = load_sealed(ctx, user_id)? else {
        if !confirm(password.to_string()).await? {
            record_failure(user_id);
            return Ok(Unlock::WrongPassword);
        }
        let (sealed, key) = Sealed::create(user_id, password)?;
        store_sealed(ctx, user_id, &sealed)?;
        clear_failures(user_id);
        keep_open(user_id, key);
        return Ok(Unlock::Unlocked);
    };

    if let Some(key) = sealed.unwrap(user_id, password)? {
        clear_failures(user_id);
        keep_open(user_id, key);
        return Ok(Unlock::Unlocked);
    }

    if let Some(previous) = previous {
        let Some(key) = sealed.unwrap(user_id, previous)? else {
            record_failure(user_id);
            return Ok(Unlock::WrongPassword);
        };
        if !confirm(password.to_string()).await? {
            record_failure(user_id);
            return Ok(Unlock::WrongPassword);
        }
        let resealed = Sealed::wrap(user_id, password, &key, DEFAULT_COST)?;
        store_sealed(ctx, user_id, &resealed)?;
        clear_failures(user_id);
        keep_open(user_id, key);
        return Ok(Unlock::Unlocked);
    }

    record_failure(user_id);
    if confirm(password.to_string()).await? {
        return Ok(Unlock::PasswordChanged);
    }
    Ok(Unlock::WrongPassword)
}

/// Delete the vault and every entry (a forgotten earlier password). The caller
/// has already confirmed the login password.
pub fn reset(ctx: &PluginCtx, user_id: &str) -> Result<(), AppError> {
    lock(user_id);
    ctx.db().execute(
        &format!("DELETE FROM {ENTRY_TABLE} WHERE traveler_id = ?1"),
        &[Value::text(user_id.to_string())],
    )?;
    ctx.db().execute(
        &format!("DELETE FROM {VAULT_TABLE} WHERE traveler_id = ?1"),
        &[Value::text(user_id.to_string())],
    )?;
    Ok(())
}

/* ── Entries ─────────────────────────────────────────────────────── */

/// What is sealed for one login.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Secret {
    origin: String,
    username: String,
    password: String,
    #[serde(default)]
    note: String,
}

impl Drop for Secret {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.password.zeroize();
    }
}

/// One login as the window lists it: everything but the password.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub id: String,
    /// `scheme://host[:port]` the login belongs to.
    pub origin: String,
    pub username: String,
    pub note: String,
    pub updated_at: String,
}

/// The error a locked vault answers with; the window turns it into the unlock
/// prompt.
fn locked() -> AppError {
    AppError::BadRequest("vault_locked".into())
}

/// `scheme://host[:port]` for a page URL or a bare host the user typed.
pub fn origin_of(input: &str) -> Option<String> {
    let input = input.trim();
    let parsed = url::Url::parse(input)
        .ok()
        .filter(|u| u.has_host())
        .or_else(|| url::Url::parse(&format!("https://{input}")).ok())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.trim_start_matches("www.").to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(match parsed.port() {
        Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
        None => format!("{}://{host}", parsed.scheme()),
    })
}

/// Whether a login saved for `saved` may be offered on `page` (both origins).
///
/// The same host, or a page on a subdomain of the saved host (a login saved on
/// `example.com` is offered on `accounts.example.com`, never the reverse). A
/// login saved over HTTPS is never offered to a plain-HTTP page.
pub fn origin_matches(saved: &str, page: &str) -> bool {
    let (Ok(saved), Ok(page)) = (url::Url::parse(saved), url::Url::parse(page)) else {
        return false;
    };
    if saved.scheme() == "https" && page.scheme() != "https" {
        return false;
    }
    // Explicit ports only, so an http login is still offered once the site
    // upgrades to https (80 and 443 are both "no port").
    if saved.port() != page.port() {
        return false;
    }
    let (Some(s), Some(p)) = (saved.host_str(), page.host_str()) else {
        return false;
    };
    let s = s.trim_start_matches("www.");
    let p = p.trim_start_matches("www.");
    p.eq_ignore_ascii_case(s)
        || (s.contains('.') && p.to_ascii_lowercase().ends_with(&format!(".{}", s.to_ascii_lowercase())))
}

fn load_entries(ctx: &PluginCtx, user_id: &str, key: &Key) -> Result<Vec<(Entry, Secret)>, AppError> {
    let rows = ctx.db().query(
        &format!(
            "SELECT id, sealed, updated_at FROM {ENTRY_TABLE} WHERE traveler_id = ?1 \
             ORDER BY updated_at DESC, rowid DESC"
        ),
        &[Value::text(user_id.to_string())],
    )?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut it = row.into_iter();
        let (Some(Value::Text(id)), Some(Value::Text(sealed))) = (it.next(), it.next()) else {
            continue;
        };
        let updated_at = match it.next() {
            Some(Value::Text(t)) => t,
            _ => String::new(),
        };
        let Some(plain) = open(key, &sealed, &entry_aad(user_id, &id)) else {
            tracing::warn!("browser: a vault entry did not open and was skipped");
            continue;
        };
        let Ok(secret) = serde_json::from_slice::<Secret>(&plain) else {
            continue;
        };
        let entry = Entry {
            id,
            origin: secret.origin.clone(),
            username: secret.username.clone(),
            note: secret.note.clone(),
            updated_at,
        };
        out.push((entry, secret));
    }
    Ok(out)
}

/// The user's logins, newest first; only those for `page` when given.
pub fn list(ctx: &PluginCtx, user_id: &str, page: Option<&str>) -> Result<Vec<Entry>, AppError> {
    let key = data_key(user_id).ok_or_else(locked)?;
    let page = page.and_then(origin_of);
    Ok(load_entries(ctx, user_id, &key)?
        .into_iter()
        .filter(|(e, _)| page.as_deref().is_none_or(|p| origin_matches(&e.origin, p)))
        .map(|(e, _)| e)
        .collect())
}

/// One login's username and password, for filling or showing.
pub fn reveal(ctx: &PluginCtx, user_id: &str, id: &str) -> Result<(String, Zeroizing<String>), AppError> {
    let key = data_key(user_id).ok_or_else(locked)?;
    let rows = ctx.db().query(
        &format!("SELECT sealed FROM {ENTRY_TABLE} WHERE traveler_id = ?1 AND id = ?2 LIMIT 1"),
        &[Value::text(user_id.to_string()), Value::text(id.to_string())],
    )?;
    let Some(Some(Value::Text(sealed))) = rows.into_iter().next().map(|r| r.into_iter().next()) else {
        return Err(AppError::NotFound("no such login".into()));
    };
    let plain = open(&key, &sealed, &entry_aad(user_id, id))
        .ok_or_else(|| AppError::Internal("this login could not be opened".into()))?;
    let secret: Secret = serde_json::from_slice(&plain)
        .map_err(|_| AppError::Internal("this login is corrupt".into()))?;
    Ok((secret.username.clone(), Zeroizing::new(secret.password.clone())))
}

/// What the window sends to save a login.
#[derive(Debug, Deserialize)]
pub struct SaveRequest {
    /// Update this entry; absent to add (or update the one with the same
    /// origin and username).
    #[serde(default)]
    pub id: Option<String>,
    /// The page URL or a host the user typed.
    pub site: String,
    #[serde(default)]
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub note: String,
}

impl Drop for SaveRequest {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.password.zeroize();
    }
}

/// Add or update one login and return it.
pub fn save(ctx: &PluginCtx, user_id: &str, req: &SaveRequest) -> Result<Entry, AppError> {
    let key = data_key(user_id).ok_or_else(locked)?;
    let origin = origin_of(&req.site)
        .ok_or_else(|| AppError::BadRequest("not a web address".into()))?;
    if req.password.is_empty() {
        return Err(AppError::BadRequest("the password is empty".into()));
    }
    let username = req.username.trim().to_string();

    let existing = load_entries(ctx, user_id, &key)?;
    let id = match req.id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) if existing.iter().any(|(e, _)| e.id == id) => id.to_string(),
        Some(_) => return Err(AppError::NotFound("no such login".into())),
        None => existing
            .iter()
            .find(|(e, _)| e.origin == origin && e.username == username)
            .map(|(e, _)| e.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
    };

    let secret = Secret {
        origin: origin.clone(),
        username: username.clone(),
        password: req.password.clone(),
        note: req.note.trim().to_string(),
    };
    let plain = Zeroizing::new(
        serde_json::to_vec(&secret).map_err(|e| AppError::Internal(e.to_string()))?,
    );
    let sealed = seal(&key, &plain, &entry_aad(user_id, &id))?;
    ctx.db().execute(
        &format!(
            "INSERT INTO {ENTRY_TABLE} (id, traveler_id, sealed, created_at, updated_at) \
             VALUES (?1, ?2, ?3, datetime('now'), datetime('now')) \
             ON CONFLICT(id) DO UPDATE SET sealed = excluded.sealed, updated_at = datetime('now') \
             WHERE browser_passwords.traveler_id = excluded.traveler_id"
        ),
        &[Value::text(id.clone()), Value::text(user_id.to_string()), Value::text(sealed)],
    )?;
    Ok(Entry { id, origin, username, note: secret.note.clone(), updated_at: String::new() })
}

/// Drop one login. Needs the vault open, like every other change.
pub fn remove(ctx: &PluginCtx, user_id: &str, id: &str) -> Result<bool, AppError> {
    data_key(user_id).ok_or_else(locked)?;
    let n = ctx.db().execute(
        &format!("DELETE FROM {ENTRY_TABLE} WHERE traveler_id = ?1 AND id = ?2"),
        &[Value::text(user_id.to_string()), Value::text(id.to_string())],
    )?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cheap cost so the tests do not spend 19 MiB per derivation.
    const TEST_COST: KdfCost = KdfCost { m: 64, t: 1, p: 1 };

    #[test]
    fn seal_round_trips_and_binds_its_slot() {
        let key = Zeroizing::new([7u8; KEY_LEN]);
        let sealed = seal(&key, b"secret", "slot-a").unwrap();
        assert_eq!(open(&key, &sealed, "slot-a").unwrap().as_slice(), b"secret");
        assert!(open(&key, &sealed, "slot-b").is_none(), "another slot must not open it");
        let other = Zeroizing::new([8u8; KEY_LEN]);
        assert!(open(&other, &sealed, "slot-a").is_none(), "another key must not open it");
    }

    #[test]
    fn nonces_are_fresh() {
        let key = Zeroizing::new([1u8; KEY_LEN]);
        assert_ne!(seal(&key, b"x", "a").unwrap(), seal(&key, b"x", "a").unwrap());
    }

    #[test]
    fn the_data_key_opens_only_with_its_password_and_owner() {
        let data_key = Zeroizing::new([9u8; KEY_LEN]);
        let sealed = Sealed::wrap("alice", "hunter2", &data_key, TEST_COST).unwrap();
        assert_eq!(*sealed.unwrap("alice", "hunter2").unwrap().unwrap(), *data_key);
        assert!(sealed.unwrap("alice", "hunter3").unwrap().is_none());
        assert!(sealed.unwrap("bob", "hunter2").unwrap().is_none());
    }

    #[test]
    fn rewrapping_keeps_the_data_key() {
        let data_key = Zeroizing::new([3u8; KEY_LEN]);
        let old = Sealed::wrap("u", "old", &data_key, TEST_COST).unwrap();
        let key = old.unwrap("u", "old").unwrap().unwrap();
        let new = Sealed::wrap("u", "new", &key, TEST_COST).unwrap();
        assert_ne!(old.salt, new.salt);
        assert_eq!(*new.unwrap("u", "new").unwrap().unwrap(), *data_key);
        assert!(new.unwrap("u", "old").unwrap().is_none());
    }

    #[test]
    fn origins_are_normalized() {
        assert_eq!(origin_of("https://www.Example.com/login?x=1").as_deref(), Some("https://example.com"));
        assert_eq!(origin_of("example.com").as_deref(), Some("https://example.com"));
        assert_eq!(origin_of("http://localhost:8080/a").as_deref(), Some("http://localhost:8080"));
        assert_eq!(origin_of("ftp://example.com"), None);
        assert_eq!(origin_of(""), None);
    }

    #[test]
    fn logins_are_offered_to_the_same_site_and_its_subdomains_only() {
        assert!(origin_matches("https://example.com", "https://example.com"));
        assert!(origin_matches("https://example.com", "https://accounts.example.com"));
        assert!(!origin_matches("https://accounts.example.com", "https://example.com"));
        assert!(!origin_matches("https://example.com", "https://evil-example.com"));
        assert!(!origin_matches("https://example.com", "https://example.com.evil.net"));
        assert!(!origin_matches("https://example.com", "http://example.com"), "never downgrade to http");
        assert!(origin_matches("http://example.com", "https://example.com"));
        assert!(!origin_matches("http://localhost:8080", "http://localhost:9090"));
    }

    #[test]
    fn failed_unlocks_are_throttled() {
        let user = "throttle-test-user";
        clear_failures(user);
        for _ in 0..MAX_ATTEMPTS {
            assert!(attempt_allowed(user));
            record_failure(user);
        }
        assert!(!attempt_allowed(user));
        clear_failures(user);
        assert!(attempt_allowed(user));
    }

    #[test]
    fn an_idle_vault_locks_itself() {
        let user = "idle-test-user";
        keep_open(user, Zeroizing::new([1u8; KEY_LEN]));
        assert!(is_unlocked(user));
        OPEN.lock().get_mut(user).unwrap().last_used = Instant::now() - IDLE_LOCK;
        assert!(data_key(user).is_none());
        assert!(!is_unlocked(user));
    }
}
