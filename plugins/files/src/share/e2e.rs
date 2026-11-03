//! End-to-end encrypted share links, turned on headlessly.
//!
//! With Gokapi's encryption level 5 ("end-to-end"), `gokapi-cli upload`
//! without `-x` encrypts the file locally with a fresh per-file key and
//! appends `#<key>` to the link. The recipient's browser decrypts it
//! (`main.wasm`); the server only ever stores ciphertext. Uploads with `-x`
//! keep working unencrypted, exactly as on level 0
//! (`storage.isEncryptionRequested`), so E2E stays a per-link choice.
//!
//! The CLI also keeps an encrypted registry of its E2E files on the server
//! (`/api/e2e/get|set`), sealed with the account's E2E key. Gokapi's web setup
//! page normally creates the first, empty registry; Shiny creates it instead
//! ([`enable`]):
//!
//! 1. a random 32-byte E2E key;
//! 2. the empty registry: [`EMPTY_REGISTRY_GOB`], AES-256-GCM sealed with that
//!    key under a random 12-byte nonce (`end2end.EncryptData`), posted to
//!    `/api/e2e/set` and read back through `/api/e2e/get`;
//! 3. the key into `gokapi-cli.json` (`E2Ekey`);
//! 4. `Encryption.Level = 5` in `config.json`, then a server restart.
//!
//! The registry's plaintext is Go `gob`, which Rust cannot produce, so the
//! empty value is a constant generated once with Go 1.24 from Gokapi's
//! `models.E2EFile` shape and checked there to decode and to accept an append
//! (see the test vectors below).

use std::time::Duration;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use serde_json::{json, Value as Json};

use super::server::{self, Instance};
use super::{cli_config_path, gokapi_config_path, write_private};

/// `gob.Encode([]models.E2EFile{})`, generated with Go 1.24.4 from a struct
/// identical to Gokapi 2.2.4's `models.E2EFile` (Uuid, Id, Filename string;
/// Cipher []byte). Verified there: decodes to an empty slice, and a decode →
/// append → encode round trip (what the CLI does per upload) works.
pub const EMPTY_REGISTRY_GOB: &str = "0dff81020102ff820001ff8000003c7f0301010745324546696c6501ff80000104010455756964010c0001024964010c00010846696c656e616d65010c000106436970686572010a00000004ff820000";

/// Gokapi's `encryption.EndToEndEncryption`.
const LEVEL_E2E: i64 = 5;
/// `end2end.e2eVersion`.
const REGISTRY_VERSION: i64 = 1;

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex constant"))
        .collect()
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// AES-256-GCM seal, output `ciphertext || tag` like Go's `gcm.Seal`.
pub fn seal(key: &[u8; 32], nonce: &[u8; 12], plain: &[u8]) -> Vec<u8> {
    let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).expect("32-byte key"));
    let mut buf = plain.to_vec();
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(*nonce), Aad::empty(), &mut buf)
        .expect("AES-GCM seal");
    buf
}

/// The `/api/e2e/set` body for an empty registry sealed with `key`.
pub fn registry_body(key: &[u8; 32], nonce: &[u8; 12]) -> Json {
    let sealed = seal(key, nonce, &hex_decode(EMPTY_REGISTRY_GOB));
    let info = json!({
        "version": REGISTRY_VERSION,
        "nonce": b64(nonce),
        "content": b64(&sealed),
        "availablefiles": null,
    });
    json!({ "content": b64(info.to_string().as_bytes()) })
}

fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|e| format!("random source: {e}"))?;
    Ok(b)
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())
}

/// Read a JSON file, apply `edit`, write it back owner-only.
fn edit_json(path: &std::path::Path, edit: impl FnOnce(&mut Json)) -> Result<(), String> {
    let raw = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut v: Json =
        serde_json::from_slice(&raw).map_err(|e| format!("parse {}: {e}", path.display()))?;
    edit(&mut v);
    write_private(path, &serde_json::to_vec_pretty(&v).unwrap())
        .map_err(|e| format!("write {}: {e}", path.display()))
}

/// Turn end-to-end encryption on for `inst` (once). The server must be
/// running; it is restarted at the end. Updates and saves `inst`.
pub async fn enable(inst: &mut Instance) -> Result<(), String> {
    if inst.e2e {
        return Ok(());
    }
    let key = random::<32>()?;
    let nonce = random::<12>()?;
    let client = http()?;

    // Never overwrite a registry that already holds files: their keys would
    // be lost for the CLI (the links themselves would keep working).
    let current: Json = client
        .get(format!("{}/api/e2e/get", inst.local_url()))
        .header("apikey", &inst.api_key)
        .send()
        .await
        .map_err(|e| format!("gokapi e2e/get: {e}"))?
        .json()
        .await
        .map_err(|e| format!("gokapi e2e/get: {e}"))?;
    if current.get("version").and_then(Json::as_i64).unwrap_or(0) != 0 {
        return Err("Gokapi already has end-to-end encryption set up with another key".into());
    }

    let resp = client
        .post(format!("{}/api/e2e/set", inst.local_url()))
        .header("apikey", &inst.api_key)
        .json(&registry_body(&key, &nonce))
        .send()
        .await
        .map_err(|e| format!("gokapi e2e/set: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("gokapi e2e/set: HTTP {}", resp.status()));
    }
    let stored: Json = client
        .get(format!("{}/api/e2e/get", inst.local_url()))
        .header("apikey", &inst.api_key)
        .send()
        .await
        .map_err(|e| format!("gokapi e2e/get: {e}"))?
        .json()
        .await
        .map_err(|e| format!("gokapi e2e/get: {e}"))?;
    if stored.get("version").and_then(Json::as_i64) != Some(REGISTRY_VERSION)
        || stored.get("nonce").and_then(Json::as_str) != Some(b64(&nonce).as_str())
    {
        return Err("gokapi did not store the end-to-end registry".into());
    }

    let key_b64 = b64(&key);
    edit_json(&cli_config_path(), |v| v["E2Ekey"] = Json::String(key_b64))?;
    edit_json(&gokapi_config_path(), |v| {
        v["Encryption"]["Level"] = json!(LEVEL_E2E)
    })?;
    inst.e2e = true;
    server::save_instance(inst)?;

    // The level is read at start.
    server::stop_server().await;
    server::ensure_running(inst).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Go 1.24.4: `gcm.Seal(nil, nonce, EMPTY_REGISTRY_GOB, nil)` with
    /// key = 0x00..0x1f and nonce = 0x64..0x6f.
    const GO_VECTOR: &str = "45e45f6478eba91c3e63a068da65568241c3070dce5eb634cebdc9490423a54990e84495397bff05f1f3793db426cbc31b5a90a1492624469b9cc9d7e381a48c4be5ab66b3393a0f8b6fed47ee0831e616d5c8187fd9b3c37dc120b628c4579f";

    fn vector_key() -> ([u8; 32], [u8; 12]) {
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut nonce = [0u8; 12];
        for (i, b) in nonce.iter_mut().enumerate() {
            *b = 100 + i as u8;
        }
        (key, nonce)
    }

    #[test]
    fn seal_matches_go_gcm() {
        let (key, nonce) = vector_key();
        let ct = seal(&key, &nonce, &hex_decode(EMPTY_REGISTRY_GOB));
        let hex: String = ct.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, GO_VECTOR);
    }

    #[test]
    fn registry_body_has_gokapi_shape() {
        use base64::Engine;
        let (key, nonce) = vector_key();
        let body = registry_body(&key, &nonce);
        let inner = base64::engine::general_purpose::STANDARD
            .decode(body["content"].as_str().unwrap())
            .unwrap();
        let info: Json = serde_json::from_slice(&inner).unwrap();
        assert_eq!(info["version"], 1);
        assert_eq!(info["nonce"], b64(&nonce));
        let content = base64::engine::general_purpose::STANDARD
            .decode(info["content"].as_str().unwrap())
            .unwrap();
        assert_eq!(content, hex_decode(GO_VECTOR));
    }

    #[test]
    fn empty_registry_names_gokapi_type() {
        // gob carries the struct and field names Gokapi decodes by.
        let gob = hex_decode(EMPTY_REGISTRY_GOB);
        let text = String::from_utf8_lossy(&gob);
        for name in ["E2EFile", "Uuid", "Id", "Filename", "Cipher"] {
            assert!(text.contains(name), "{name}");
        }
    }
}
