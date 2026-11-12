//! The Oorbit window's backend for previews outside LiteAura (driven by
//! `plugins/oorbit/scripts/preview-server.py`): the same view the plugin
//! returns, from a planet kept in a local JSON file.
//!
//!   preview STATE_FILE NOW_MS catalog
//!   preview STATE_FILE NOW_MS state
//!   preview STATE_FILE NOW_MS found CLIMATE
//!   preview STATE_FILE NOW_MS action '{"type":"research","tech":"fire"}'
//!
//! Prints `{"success":true,"data":VIEW}` or `{"success":false,"error":...}`.
use oorbit_sim::view::planet_view;
use oorbit_sim::{advance, apply, Action, Catalog, Planet};
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let reply = run(&args).unwrap_or_else(|e| json!({ "success": false, "error": e }));
    println!("{reply}");
}

fn run(args: &[String]) -> Result<serde_json::Value, String> {
    let [_, file, now, command, rest @ ..] = args else {
        return Err("usage: preview STATE_FILE NOW_MS state|found|action [ARG]".into());
    };
    let now: i64 = now.parse().map_err(|_| "NOW_MS must be a number")?;
    let catalog = Catalog::builtin();
    if command == "catalog" {
        return Ok(json!({ "success": true, "data": catalog }));
    }
    let saved = std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str::<Planet>(&t).ok());
    let mut planet = match (command.as_str(), saved) {
        ("found", Some(_)) => return Err("this planet is already founded".into()),
        ("found", None) => {
            let climate = rest.first().ok_or("found needs a climate")?;
            Planet::new(now as u64 ^ 0x5EED, climate, now, &catalog)?
        }
        (_, None) => {
            return Ok(json!({ "success": true, "data": { "now_ms": now, "planet": null } }))
        }
        (_, Some(planet)) => planet,
    };
    advance(&mut planet, &catalog, now);
    let mut error = None;
    if command == "action" {
        let action: Action = serde_json::from_str(rest.first().ok_or("action needs JSON")?)
            .map_err(|e| format!("invalid action: {e}"))?;
        if let Err(e) = apply(&mut planet, &catalog, now, &action) {
            error = Some(e.to_string());
        }
    }
    std::fs::write(file, serde_json::to_string(&planet).unwrap()).map_err(|e| e.to_string())?;
    if let Some(e) = error {
        return Err(e);
    }
    Ok(json!({ "success": true, "data": planet_view(&planet, &catalog, now) }))
}
