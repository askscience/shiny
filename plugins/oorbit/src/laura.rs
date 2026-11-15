//! Laura, the chronicler: a few lines retelling what happened on the planet
//! lately, written by the AI in a voice the player picks.
//!
//! The game never waits for her. The route answers at once with what she
//! last wrote (or nothing) and, when the Chronicle has moved on since, asks
//! the model in the background; the window asks again a little later. When
//! the AI is switched off for the user, or does not answer, the Chronicle
//! still reads from the catalog's own sentences.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use oorbit_sim::{Catalog, Planet};
use serde::Deserialize;
use serde_json::json;
use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, ok, user_id, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

use crate::store::{self, catalog};

/// How many of the latest Chronicle entries Laura retells.
const ENTRIES: usize = 20;

/// The voices she can take.
pub const PERSONAS: [&str; 4] = ["historian", "gossip", "grandpa", "scientist"];

#[derive(Clone)]
struct Told {
    key: String,
    text: String,
}

fn told() -> &'static Mutex<HashMap<String, Told>> {
    static TOLD: OnceLock<Mutex<HashMap<String, Told>>> = OnceLock::new();
    TOLD.get_or_init(|| Mutex::new(HashMap::new()))
}

fn writing() -> &'static Mutex<HashSet<String>> {
    static WRITING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    WRITING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Is the AI switched on for this user? (The core's switch: the
/// `ai.enabled` preference, on unless set to false.)
fn ai_enabled(ctx: &PluginCtx, uid: &str) -> bool {
    let rows = ctx.db().query(
        "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = 'ai.enabled'",
        &[Value::text(uid)],
    );
    !matches!(
        rows.ok().and_then(|r| r.into_iter().next()).and_then(|row| row.into_iter().next()),
        Some(Value::Text(v)) if v == "false" || v == "0"
    )
}

fn persona_voice(persona: &str) -> &'static str {
    match persona {
        "gossip" => "the village gossip: warm, chatty, a little nosy, delighted by small news",
        "grandpa" => "a grumpy old grandpa: grumbles, compares everything to the old days, secretly proud",
        "scientist" => "a keen young scientist: curious, enthusiastic, notices causes and numbers",
        _ => "a solemn historian: measured, a little grand, as if writing for the ages",
    }
}

fn language(lang: &str) -> &'static str {
    match lang {
        "it" => "Italian",
        "fr" => "French",
        "de" => "German",
        "es" => "Spanish",
        _ => "English",
    }
}

/// A catalog id's English name, or the id itself.
fn name_of(catalog: &Catalog, id: &str) -> String {
    catalog
        .tech(id)
        .map(|t| t.name.clone())
        .or_else(|| catalog.building(id).map(|b| b.name.clone()))
        .or_else(|| catalog.resource(id).map(|r| r.name.clone()))
        .or_else(|| catalog.eras.iter().find(|e| e.id == id).map(|e| e.name.clone()))
        .unwrap_or_else(|| id.replace('_', " "))
}

/// The prompt: who Laura is, how the planet stands, and the latest
/// entries (oldest first) with how long ago each happened.
pub fn prompt(planet: &Planet, catalog: &Catalog, persona: &str, lang: &str) -> (String, String) {
    let system = format!(
        "You are Laura, the chronicler of a tiny planet in the game Oorbit. Speak as {voice}. \
         Write in {language}. Write 3 to 5 short sentences of plain prose: no lists, no headings, \
         no emoji, no quotation marks around the whole text, no dashes as punctuation. Retell only \
         what the chronicle entries say; you may add colour, feeling and a little opinion, but never \
         invent events, names, places or numbers. Mention the newest things first or build up to them.",
        voice = persona_voice(persona),
        language = language(lang),
    );
    let era = catalog
        .era(planet.era)
        .map(|e| e.name.clone())
        .unwrap_or_default();
    let cities: Vec<String> = planet.cities.iter().map(|c| c.name.clone()).collect();
    let mut lines = Vec::new();
    let start = planet.chronicle.len().saturating_sub(ENTRIES);
    for e in &planet.chronicle[start..] {
        let hours = (planet.step - e.step) as f64 / 6.0;
        let ago = if hours < 1.0 {
            "just now".to_string()
        } else if hours < 48.0 {
            format!("{} hours ago", hours.round())
        } else {
            format!("{} days ago", (hours / 24.0).round())
        };
        let params: Vec<String> = e
            .params
            .iter()
            .map(|(k, v)| format!("{k}: {}", name_of(catalog, v)))
            .collect();
        lines.push(format!("- {ago}: {} ({})", e.key.replace('_', " "), params.join(", ")));
    }
    let user = format!(
        "The planet is in the {era}, with {people} people (mood {mood}%) in {count} cities: {cities}.\n\
         Chronicle entries, oldest first (planet time):\n{lines}\n\n\
         Tell the player what has happened lately.",
        people = planet.population.round(),
        mood = (planet.mood * 100.0).round(),
        count = cities.len(),
        cities = cities.join(", "),
        lines = lines.join("\n"),
    );
    (system, user)
}

/// `POST /api/oorbit/chronicle` with `{ lang, persona }`: what Laura has to
/// say. `status` is `ready` (the text is about the latest entries),
/// `writing` (a new text is on its way; `text` is the previous one, if
/// any), `off` (the AI is switched off) or `empty` (nothing has happened).
pub fn chronicle(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        lang: String,
        #[serde(default)]
        persona: String,
    }
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = <axum::Json<Body> as axum::extract::FromRequest<()>>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let persona = if PERSONAS.contains(&body.persona.as_str()) { body.persona } else { "historian".into() };
            let lang = if body.lang.is_empty() { "en".to_string() } else { body.lang };
            if !ai_enabled(&ctx, &uid) {
                return Ok(ok(json!({ "status": "off", "text": "" })));
            }
            let planet = {
                let _guard = store::lock();
                store::load(&ctx, &uid)?
            };
            let Some(planet) = planet else {
                return Ok(ok(json!({ "status": "empty", "text": "" })));
            };
            if planet.chronicle.is_empty() {
                return Ok(ok(json!({ "status": "empty", "text": "" })));
            }
            let last = planet.chronicle.last().map(|e| e.step).unwrap_or(0);
            let key = format!("{last}:{}:{persona}:{lang}", planet.chronicle.len());
            let previous = told().lock().ok().and_then(|t| t.get(&uid).cloned());
            if let Some(t) = &previous {
                if t.key == key {
                    return Ok(ok(json!({ "status": "ready", "text": t.text, "persona": persona })));
                }
            }
            let old_text = previous.map(|t| t.text).unwrap_or_default();
            let start = writing().lock().map(|mut w| w.insert(uid.clone())).unwrap_or(false);
            if start {
                let (system, user) = prompt(&planet, catalog(), &persona, &lang);
                let ctx = ctx.clone();
                let uid = uid.clone();
                shiny_plugin_sdk::rt::spawn(async move {
                    let client = ctx.ollama().await;
                    let reply = if client.is_available().await {
                        client
                            .chat(vec![("system".into(), system), ("user".into(), user)], None)
                            .await
                            .ok()
                    } else {
                        None
                    };
                    if let Some(text) = reply.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
                        if let Ok(mut told) = told().lock() {
                            told.insert(uid.clone(), Told { key, text });
                        }
                    } else {
                        tracing::info!("oorbit: Laura got no answer from the AI; the Chronicle stays as written");
                    }
                    if let Ok(mut w) = writing().lock() {
                        w.remove(&uid);
                    }
                });
            }
            Ok(ok(json!({ "status": "writing", "text": old_text, "persona": persona })))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_names_things_and_keeps_to_the_entries() {
        let catalog = Catalog::builtin();
        let mut planet = Planet::new(3, "temperate", 1_794_470_400_000, &catalog).unwrap();
        planet.log(&catalog, planet.step, "tech_done", &[("tech", "fire")]);
        let (system, user) = prompt(&planet, &catalog, "gossip", "it");
        assert!(system.contains("Italian"));
        assert!(system.contains("gossip"));
        assert!(user.contains("Fire"), "ids become names: {user}");
        assert!(user.contains("tech done"));
    }
}
