//! Tool cards: the compact record of one finished tool call that the chat
//! renders as a small rich card (the weather/route/mail cards under the user's
//! message).
//!
//! The record is streamed live as a `tool` event and stored as a `tool` row of
//! the conversation, so reopening a chat rebuilds the same cards. A tool's raw
//! payload can be large (a page's text, a route's geometry, a directory of
//! thousands of entries) and can carry fields nobody should see in a chat log
//! (an account's password, an API token), so every record goes through
//! [`card_record`]: credential-looking keys are dropped, heavy keys removed,
//! strings and arrays capped, and the whole record kept well under ~64 KB.
//!
//! The rows are UI-only: the agent's memory window reads `user`, `assistant`
//! and `system` rows, never `tool` ones (see `chat_memory::recent_history`).

use serde_json::{Map, Value};

/// Serialized size the capped `data` must stay under. The params budget is
/// separate and much smaller, so a whole event stays below ~64 KB.
const MAX_DATA_BYTES: usize = 48 * 1024;
const MAX_PARAMS_BYTES: usize = 6 * 1024;

/// Keys whose value is removed outright: credentials of any kind.
const SECRET_MARKERS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "api_key",
    "apikey",
    "authorization",
    "cookie",
    "credential",
    "private_key",
    "access_key",
];

/// Keys that never help a card and are heavy or unsafe to keep: a route's
/// polyline, a mail's HTML body, raw bytes.
const HEAVY_KEYS: &[&str] = &["geometry", "html", "raw", "bytes_base64", "base64", "headers"];

/// Keys that hold prose or command output: they get the longer string cap.
const TEXT_KEYS: &[&str] = &["output", "text", "content", "body", "snippet", "description", "error"];

/// A data URI longer than this is dropped rather than cut (a cut one is
/// garbage anyway).
const MAX_DATA_URI: usize = 256;

#[derive(Clone, Copy)]
struct Limits {
    string: usize,
    text: usize,
    items: usize,
    keys: usize,
    depth: usize,
}

const DATA_PASSES: [Limits; 3] = [
    Limits { string: 600, text: 4000, items: 12, keys: 40, depth: 6 },
    Limits { string: 240, text: 1200, items: 6, keys: 24, depth: 4 },
    Limits { string: 120, text: 400, items: 3, keys: 16, depth: 3 },
];

const PARAMS_PASSES: [Limits; 2] = [
    Limits { string: 300, text: 600, items: 6, keys: 20, depth: 3 },
    Limits { string: 80, text: 120, items: 3, keys: 10, depth: 2 },
];

/// Is this object key a credential? Matched case-insensitively on the
/// normalised key (`X-Api-Key`, `apiKey`, `refresh_token` all match).
pub fn is_secret_key(key: &str) -> bool {
    let normalised: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .map(|c| if c == '-' { '_' } else { c.to_ascii_lowercase() })
        .collect();
    let squashed = normalised.replace('_', "");
    SECRET_MARKERS
        .iter()
        .any(|m| normalised.contains(m) || squashed.contains(&m.replace('_', "")))
}

fn cut(s: &str, max: usize, capped: &mut bool) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    *capped = true;
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out = out.trim_end().to_string();
    out.push('…');
    out
}

fn cap(value: &Value, key: Option<&str>, limits: Limits, depth: usize, capped: &mut bool) -> Option<Value> {
    match value {
        Value::String(s) => {
            if s.starts_with("data:") && s.len() > MAX_DATA_URI {
                *capped = true;
                return None;
            }
            let max = if key.is_some_and(|k| TEXT_KEYS.contains(&k)) { limits.text } else { limits.string };
            Some(Value::String(cut(s, max, capped)))
        }
        Value::Array(items) => {
            if depth >= limits.depth {
                *capped = true;
                return None;
            }
            if items.len() > limits.items {
                *capped = true;
            }
            Some(Value::Array(
                items
                    .iter()
                    .take(limits.items)
                    .filter_map(|item| cap(item, key, limits, depth + 1, capped))
                    .collect(),
            ))
        }
        Value::Object(map) => {
            if depth >= limits.depth {
                *capped = true;
                return None;
            }
            let mut out = Map::new();
            for (k, v) in map {
                if is_secret_key(k) || HEAVY_KEYS.contains(&k.as_str()) {
                    *capped = true;
                    continue;
                }
                if out.len() >= limits.keys {
                    *capped = true;
                    break;
                }
                if let Some(v) = cap(v, Some(k.as_str()), limits, depth + 1, capped) {
                    out.insert(k.clone(), v);
                }
            }
            Some(Value::Object(out))
        }
        other => Some(other.clone()),
    }
}

/// Sanitise and cap a payload, tightening the limits until it fits `budget`
/// bytes. Returns the value and whether anything was cut or removed.
fn fit(value: &Value, passes: &[Limits], budget: usize) -> (Option<Value>, bool) {
    let mut capped = false;
    for limits in passes {
        let mut cut_here = false;
        let out = cap(value, None, *limits, 0, &mut cut_here);
        capped |= cut_here;
        let size = out
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .map_or(0, |s| s.len());
        if size <= budget {
            return (out, capped);
        }
        capped = true;
    }
    (None, true)
}

/// A payload capped and sanitised for a card (exposed for tests and callers
/// that need the data half alone).
pub fn cap_payload(value: &Value) -> (Option<Value>, bool) {
    fit(value, &DATA_PASSES, MAX_DATA_BYTES)
}

/// The card record of one finished tool call:
/// `{ action, result: "ok"|"error", plugin?, data?, params?, capped? }`.
///
/// `result` is folded to `ok` / `error` (a tool's own non-`ok` result, such as
/// a refusal, is an error card). `plugin` is the owning plugin, so the card's
/// open arrow knows which window to focus.
pub fn card_record(
    action: &str,
    result: &str,
    plugin: Option<&str>,
    data: Option<&Value>,
    params: &Value,
) -> Value {
    let mut record = Map::new();
    record.insert("action".into(), Value::String(action.to_string()));
    let result = if result == "ok" { "ok" } else { "error" };
    record.insert("result".into(), Value::String(result.into()));
    if let Some(plugin) = plugin.filter(|p| !p.is_empty() && *p != "core") {
        record.insert("plugin".into(), Value::String(plugin.to_string()));
    }

    let mut capped = false;
    if let Some(data) = data.filter(|d| !d.is_null()) {
        let (value, cut) = cap_payload(data);
        capped |= cut;
        if let Some(value) = value {
            record.insert("data".into(), value);
        }
    }
    if params.as_object().is_some_and(|m| !m.is_empty()) {
        let (value, cut) = fit(params, &PARAMS_PASSES, MAX_PARAMS_BYTES);
        capped |= cut;
        if let Some(value) = value {
            record.insert("params".into(), value);
        }
    }
    if capped {
        record.insert("capped".into(), Value::Bool(true));
    }
    Value::Object(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secret_keys_are_recognised() {
        for key in ["password", "smtp_password", "apiKey", "X-Api-Key", "refresh_token", "Authorization", "cookie", "client_secret"] {
            assert!(is_secret_key(key), "{key} should be secret");
        }
        for key in ["title", "station", "count", "command", "output", "url"] {
            assert!(!is_secret_key(key), "{key} should not be secret");
        }
    }

    #[test]
    fn credentials_are_stripped_at_any_depth() {
        let data = json!({
            "accounts": [{ "email": "a@b.c", "password": "hunter2", "smtp": { "api_key": "k" } }],
            "token": "abc",
        });
        let record = card_record("mail_status", "ok", Some("mail"), Some(&data), &json!({}));
        let text = record.to_string();
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("\"k\""));
        assert!(!text.contains("abc"));
        assert_eq!(record["data"]["accounts"][0]["email"], "a@b.c");
        assert_eq!(record["capped"], true);
    }

    #[test]
    fn heavy_keys_and_data_uris_are_dropped() {
        let geometry = vec![[1.0, 2.0]; 500];
        let data = json!({
            "route": { "total_duration_seconds": 2880.0, "geometry": geometry },
            "html": "<p>hi</p>",
            "thumb": format!("data:image/png;base64,{}", "A".repeat(4000)),
        });
        let record = card_record("map_route", "ok", Some("traveler"), Some(&data), &json!({}));
        assert!(record["data"]["route"].get("geometry").is_none());
        assert!(record["data"].get("html").is_none());
        assert!(record["data"].get("thumb").is_none());
        assert_eq!(record["data"]["route"]["total_duration_seconds"], 2880.0);
    }

    #[test]
    fn long_strings_and_arrays_are_capped() {
        let data = json!({
            "output": "x".repeat(20_000),
            "title": "t".repeat(5_000),
            "results": (0..100).map(|i| json!({ "n": i })).collect::<Vec<_>>(),
        });
        let (value, capped) = cap_payload(&data);
        let value = value.unwrap();
        assert!(capped);
        assert!(value["output"].as_str().unwrap().chars().count() <= 4000);
        assert!(value["output"].as_str().unwrap().ends_with('…'));
        assert!(value["title"].as_str().unwrap().chars().count() <= 600);
        assert_eq!(value["results"].as_array().unwrap().len(), 12);
    }

    #[test]
    fn a_huge_payload_still_fits_the_event_budget() {
        // Many wide, deep entries: the first pass is too big, later ones fit.
        let entry = json!({
            "a": "y".repeat(600), "b": "y".repeat(600), "c": "y".repeat(600),
            "d": "y".repeat(600), "e": "y".repeat(600), "f": "y".repeat(600),
            "g": "y".repeat(600), "h": "y".repeat(600), "i": "y".repeat(600),
        });
        let data = json!({
            "rows": (0..12).map(|_| json!({ "cells": vec![entry.clone(); 12] })).collect::<Vec<_>>(),
        });
        let params = json!({ "query": "q".repeat(50_000) });
        let record = card_record("calc_read", "ok", Some("calc"), Some(&data), &params);
        let size = record.to_string().len();
        assert!(size < 64 * 1024, "record is {size} bytes");
        assert_eq!(record["capped"], true);
    }

    #[test]
    fn small_payloads_pass_untouched() {
        let data = json!({ "expression": "18% * 2340", "result": 421.2, "result_text": "421.2" });
        let params = json!({ "expression": "18% * 2340" });
        let record = card_record("calculator_eval", "ok", Some("calculator"), Some(&data), &params);
        assert_eq!(record["data"], data);
        assert_eq!(record["params"], params);
        assert!(record.get("capped").is_none());
        assert_eq!(record["plugin"], "calculator");
    }

    #[test]
    fn results_fold_to_ok_or_error_and_core_has_no_plugin() {
        let record = card_record("web_search", "denied", Some("core"), None, &Value::Null);
        assert_eq!(record["result"], "error");
        assert!(record.get("plugin").is_none());
        assert!(record.get("data").is_none());
        assert!(record.get("params").is_none());
    }
}
