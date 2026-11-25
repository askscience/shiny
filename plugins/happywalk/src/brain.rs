//! The character's brain: one question to the user's language model, one
//! structured answer.
//!
//! The game sends what just happened and the actions it would accept; the
//! model answers `{"action": …, "say": …}`. The answer is checked here, so
//! the game only ever sees an action it offered and a short line. Anything
//! else (the AI switched off, no model, a late or malformed answer) is "no
//! answer": the game then uses a line of its own, and never waits.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;
use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::routes::{bridged_route, ok, user_id, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

/// Longer than this and the moment the line was for has passed.
const ANSWER_WITHIN: Duration = Duration::from_secs(6);
const LINE_MAX_CHARS: usize = 160;

#[derive(Deserialize, Debug, Default)]
pub struct Request {
    #[serde(default)]
    pub situation: String,
    #[serde(default)]
    pub allowed_actions: Vec<String>,
    #[serde(default)]
    pub lang: String,
}

#[derive(Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct Answer {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub say: String,
}

fn language(lang: &str) -> &'static str {
    match lang.split('-').next().unwrap_or("") {
        "it" => "Italian",
        "fr" => "French",
        "de" => "German",
        "es" => "Spanish",
        "pt" => "Portuguese",
        _ => "English",
    }
}

pub fn system_prompt(request: &Request) -> String {
    format!(
        "You are the character of a video game, walking through a small pastel city. \
         A player steers you with their voice, and you know it. You are ironic and warm, \
         never mean, never rude; you tease the player when their orders go wrong. \
         Answer with one JSON object and nothing else: \
         {{\"action\": one of [{actions}], \"say\": a single short spoken line in {language}, at most 15 words}}. \
         Use an empty action when none fits.",
        actions = request
            .allowed_actions
            .iter()
            .map(|a| format!("\"{a}\""))
            .collect::<Vec<_>>()
            .join(", "),
        language = language(&request.lang),
    )
}

/// The model's text as a checked answer: the first JSON object in it, an
/// action the game offered (else none), one short line.
pub fn parse(text: &str, allowed: &[String]) -> Option<Answer> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let mut answer: Answer = serde_json::from_str(text.get(start..=end)?).ok()?;
    answer.action = answer.action.trim().to_lowercase();
    if !allowed.iter().any(|a| *a == answer.action) {
        answer.action.clear();
    }
    let line = answer.say.split_whitespace().collect::<Vec<_>>().join(" ");
    answer.say = line.chars().take(LINE_MAX_CHARS).collect();
    (!answer.action.is_empty() || !answer.say.is_empty()).then_some(answer)
}

fn preference(ctx: &PluginCtx, uid: &str, key: &str) -> Option<String> {
    let rows = ctx
        .db()
        .query(
            "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = ?2",
            &[Value::text(uid), Value::text(key)],
        )
        .ok()?;
    match rows.into_iter().next()?.into_iter().next()? {
        Value::Text(v) => Some(v.trim().trim_matches('"').to_string()).filter(|v| !v.is_empty()),
        _ => None,
    }
}

/// `POST /api/happywalk/brain` → `{answer: {action, say} | null, reason, model, ms}`.
pub fn route(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let request: Request = crate::routes::json_body(req).await?;
            let started = Instant::now();
            let none = |reason: &str, model: &str, started: Instant| {
                ok(json!({
                    "answer": null,
                    "reason": reason,
                    "model": model,
                    "ms": started.elapsed().as_millis() as u64,
                }))
            };
            if matches!(
                preference(&ctx, &uid, "ai.enabled").as_deref(),
                Some("false" | "0")
            ) {
                return Ok(none("ai-off", "", started));
            }
            // The model the user chose in Settings, else the server's default.
            let model = preference(&ctx, &uid, "ai.ollama_model");
            let client = ctx.ollama().await;
            let name = model
                .clone()
                .unwrap_or_else(|| client.default_model().to_string());
            let messages = vec![
                ("system".to_string(), system_prompt(&request)),
                ("user".to_string(), request.situation.clone()),
            ];
            let reply =
                tokio::time::timeout(ANSWER_WITHIN, client.chat(messages, model.as_deref())).await;
            let text = match reply {
                Err(_) => return Ok(none("late", &name, started)),
                Ok(Err(e)) => {
                    tracing::info!("happywalk: the brain got no answer: {e}");
                    return Ok(none("unavailable", &name, started));
                }
                Ok(Ok(text)) => text,
            };
            match parse(&text, &request.allowed_actions) {
                None => Ok(none("malformed", &name, started)),
                Some(answer) => Ok(ok(json!({
                    "answer": answer,
                    "model": name,
                    "ms": started.elapsed().as_millis() as u64,
                }))),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed() -> Vec<String> {
        vec!["stop".into(), "turn_left".into()]
    }

    #[test]
    fn keeps_an_offered_action_and_the_line() {
        let answer = parse(
            r#"Sure! {"action": "Stop", "say": "Fermo,  fermo."} "#,
            &allowed(),
        )
        .unwrap();
        assert_eq!(
            answer,
            Answer {
                action: "stop".into(),
                say: "Fermo, fermo.".into()
            }
        );
    }

    #[test]
    fn drops_an_action_the_game_did_not_offer() {
        let answer = parse(r#"{"action": "fly", "say": "Magari."}"#, &allowed()).unwrap();
        assert_eq!(answer.action, "");
        assert_eq!(answer.say, "Magari.");
    }

    #[test]
    fn nothing_usable_is_no_answer() {
        assert_eq!(parse("I would rather not.", &allowed()), None);
        assert_eq!(parse(r#"{"action": "fly"}"#, &allowed()), None);
    }

    #[test]
    fn a_long_line_is_cut() {
        let long = "a".repeat(400);
        let answer = parse(&format!(r#"{{"say": "{long}"}}"#), &allowed()).unwrap();
        assert_eq!(answer.say.chars().count(), LINE_MAX_CHARS);
    }

    #[test]
    fn the_prompt_names_the_actions_and_the_language() {
        let prompt = system_prompt(&Request {
            situation: String::new(),
            allowed_actions: allowed(),
            lang: "it".into(),
        });
        assert!(prompt.contains(r#"["stop", "turn_left"]"#));
        assert!(prompt.contains("Italian"));
    }
}
