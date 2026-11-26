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
    /// Who the model plays, when it is not the character: a suspect, a
    /// witness, written by the game. Empty means the character
    /// (`personality.md`).
    #[serde(default)]
    pub persona: String,
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

/// Who the character is: its own file, to be edited without touching code.
pub const PERSONALITY: &str = include_str!("personality.md");

/// Ways to make a line funny. One is suggested for each line, at random, so
/// two moments in a row are never joked about the same way.
pub const ANGLES: &[&str] = &[
    "a tragic overreaction, as if it were the last act of an opera",
    "deadpan understatement",
    "a nature-documentary narrator describing yourself",
    "a petty grudge against the thing, as if it did it on purpose",
    "speak to the people who will watch this clip",
    "an absurd comparison",
    "desperate optimism: the bright side, however small",
    "a philosophical thought far too deep for the moment",
    "a backhanded compliment to the player",
    "a breaking-news report",
    "a callback to something that happened earlier, if anything fits",
    "make it about food",
    "a very polite complaint to the city council",
    "pretend it was all part of your plan",
    "a sports commentator, live",
    "a dramatic confession",
];

/// The angle for one line: different from one request to the next.
pub fn angle(salt: u64) -> &'static str {
    let mut z = salt.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ANGLES[((z ^ (z >> 31)) % ANGLES.len() as u64) as usize]
}

pub fn system_prompt(request: &Request) -> String {
    let actions = if request.allowed_actions.is_empty() {
        "\"action\": \"\" (there is nothing to do, only something to say)".to_string()
    } else {
        format!(
            "\"action\": one of [{}], or \"\" when none fits",
            request
                .allowed_actions
                .iter()
                .map(|a| format!("\"{a}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let who = if request.persona.is_empty() {
        PERSONALITY
    } else {
        request.persona.as_str()
    };
    format!(
        "{who}\n\
         Answer at once, without deliberating, with one JSON object and nothing else: \
         {{{actions}, \"say\": your one spoken line, in {language}, at most 18 words}}.",
        language = language(&request.lang),
    )
}

/// What the model is told about the moment: the game's description, and the
/// angle to try this time.
pub fn user_message(request: &Request, salt: u64) -> String {
    // Somebody else than the character: no comic angle, they are who they are.
    if !request.persona.is_empty() {
        return request.situation.clone();
    }
    format!("{}\nThis time, go for: {}.", request.situation, angle(salt))
}

/// The model's text as a checked answer: the first JSON object in it, an
/// action the game offered (else none), one short line.
pub fn parse(text: &str, allowed: &[String]) -> Option<Answer> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let value: serde_json::Value = serde_json::from_str(text.get(start..=end)?).ok()?;
    // Models fill a field they have no use for in their own ways (`[]`,
    // `null`, "none"): anything that is not an offered action is no action.
    let mut answer = Answer {
        action: value["action"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_lowercase(),
        say: value["say"].as_str().unwrap_or_default().to_string(),
    };
    if !allowed.contains(&answer.action) {
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

/// Where the user's assistant runs, as chosen in Settings → AI.
#[derive(Debug, PartialEq)]
pub enum Provider {
    /// The server's Ollama, with the user's model or the server's default.
    Ollama { url: String, model: String },
    /// The user's own OpenAI-compatible endpoint.
    OpenAi {
        base: String,
        key: String,
        model: String,
    },
}

impl Provider {
    pub fn model(&self) -> &str {
        match self {
            Provider::Ollama { model, .. } | Provider::OpenAi { model, .. } => model,
        }
    }
}

/// The same choice the assistant makes (`src/services/ai.rs` in the core):
/// the OpenAI-compatible endpoint when the user set one up completely and its
/// address is acceptable, Ollama otherwise.
fn provider(ctx: &PluginCtx, uid: &str) -> Provider {
    let pref = |key: &str| preference(ctx, uid, key);
    if pref("ai.provider").as_deref() == Some("openai") {
        if let (Some(base), Some(model)) = (pref("ai.openai_base_url"), pref("ai.openai_model")) {
            if safe_base_url(&base) {
                return Provider::OpenAi {
                    base: base.trim_end_matches('/').to_string(),
                    key: pref("ai.openai_api_key").unwrap_or_default(),
                    model,
                };
            }
            tracing::warn!("happywalk: refusing unsafe ai.openai_base_url; using Ollama instead");
        }
    }
    Provider::Ollama {
        url: ctx.config.ollama_url.trim_end_matches('/').to_string(),
        model: pref("ai.ollama_model").unwrap_or_else(|| ctx.config.ollama_model.clone()),
    }
}

/// The core's rule for a user-supplied endpoint: http(s), and not a
/// link-local, unspecified, broadcast or multicast address.
pub fn safe_base_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url.trim()) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    match host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
    {
        Ok(std::net::IpAddr::V4(v4)) => {
            let o = v4.octets();
            !(v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0)
        }
        Ok(std::net::IpAddr::V6(v6)) => {
            !(v6.is_unspecified() || v6.is_multicast() || (v6.segments()[0] & 0xffc0) == 0xfe80)
        }
        Err(_) => true,
    }
}

/// The shape the answer must have. Ollama constrains the model's output to
/// it (its `format`), so a small model cannot wander off into prose.
fn answer_schema(allowed: &[String]) -> serde_json::Value {
    let mut actions: Vec<&str> = allowed.iter().map(String::as_str).collect();
    actions.push("");
    json!({
        "type": "object",
        "properties": {
            "action": { "type": "string", "enum": actions },
            "say": { "type": "string" },
        },
        "required": ["action", "say"],
    })
}

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// One question, one reply: the model's text.
async fn ask(provider: &Provider, request: &Request, salt: u64) -> Result<String, String> {
    let messages = json!([
        { "role": "system", "content": system_prompt(request) },
        { "role": "user", "content": user_message(request, salt) },
    ]);
    match provider {
        Provider::Ollama { url, model } => {
            let mut body = json!({
                "model": model,
                "messages": messages,
                "stream": false,
                "format": answer_schema(&request.allowed_actions),
                // A line is short, and the moment passes quickly: no
                // reasoning trace, few tokens, the model kept loaded.
                "think": false,
                "keep_alive": "30m",
                "options": { "num_predict": 96, "temperature": 1.0, "repeat_penalty": 1.2 },
            });
            let mut reply = http()
                .post(format!("{url}/api/chat"))
                .json(&body)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if reply.status() == reqwest::StatusCode::BAD_REQUEST {
                // Some models refuse the `think` switch outright.
                body.as_object_mut().map(|b| b.remove("think"));
                reply = http()
                    .post(format!("{url}/api/chat"))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
            }
            if !reply.status().is_success() {
                return Err(format!("Ollama answered {}", reply.status()));
            }
            let data: serde_json::Value = reply.json().await.map_err(|e| e.to_string())?;
            Ok(data["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string())
        }
        Provider::OpenAi { base, key, model } => {
            // Many of these models reason before they answer, and the trace
            // counts against the limit: leave room for it. JSON mode and
            // "no thinking" are asked for first; an endpoint that does not
            // know them refuses the request, which is then sent plain.
            let plain = json!({
                "model": model,
                "messages": messages,
                "stream": false,
                "max_tokens": 700,
                "temperature": 1.0,
                // Words and turns of phrase it already used cost it.
                "frequency_penalty": 0.7,
                "presence_penalty": 0.6,
            });
            let mut asked = plain.clone();
            asked["response_format"] = json!({ "type": "json_object" });
            asked["thinking"] = json!({ "type": "disabled" });
            let send = |body: serde_json::Value| {
                let mut call = http().post(format!("{base}/chat/completions")).json(&body);
                if !key.is_empty() {
                    call = call.bearer_auth(key);
                }
                call.send()
            };
            let mut reply = send(asked).await.map_err(|e| e.to_string())?;
            if matches!(reply.status().as_u16(), 400 | 422) {
                reply = send(plain).await.map_err(|e| e.to_string())?;
            }
            if !reply.status().is_success() {
                // The endpoint's own text stays out of the log line a page reads.
                return Err(format!("the endpoint answered {}", reply.status()));
            }
            let data: serde_json::Value = reply.json().await.map_err(|e| e.to_string())?;
            let choice = &data["choices"][0];
            tracing::debug!(
                "happywalk: the endpoint finished with {} ({} reasoning characters)",
                choice["finish_reason"],
                choice["message"]["reasoning_content"]
                    .as_str()
                    .map(str::len)
                    .unwrap_or(0)
            );
            Ok(choice["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string())
        }
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
            let provider = provider(&ctx, &uid);
            // A different number every request: the angle of the line.
            let salt = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or_default();
            let name = provider.model().to_string();
            let text =
                match tokio::time::timeout(ANSWER_WITHIN, ask(&provider, &request, salt)).await {
                    Err(_) => return Ok(none("late", &name, started)),
                    Ok(Err(e)) => {
                        tracing::info!("happywalk: the brain got no answer: {e}");
                        return Ok(none("unavailable", &name, started));
                    }
                    Ok(Ok(text)) => text,
                };
            match parse(&text, &request.allowed_actions) {
                None => {
                    let shown: String = text.chars().take(200).collect();
                    tracing::info!("happywalk: the brain's answer was not usable: {shown:?}");
                    Ok(none("malformed", &name, started))
                }
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
    fn an_action_of_any_other_shape_is_no_action() {
        let answer = parse(r#"{"action": [], "say": "Terza volta, stesso muro."}"#, &[]).unwrap();
        assert_eq!(
            answer,
            Answer {
                action: String::new(),
                say: "Terza volta, stesso muro.".into()
            }
        );
        assert_eq!(
            parse(r#"{"action": null, "say": "Eh."}"#, &allowed())
                .unwrap()
                .action,
            ""
        );
    }

    #[test]
    fn with_nothing_to_do_the_prompt_offers_no_list() {
        let request = Request {
            situation: String::new(),
            allowed_actions: Vec::new(),
            lang: "it".into(),
            ..Default::default()
        };
        let prompt = system_prompt(&request);
        assert!(!prompt.contains("one of"));
        assert!(prompt.contains("nothing to do"));
    }

    #[test]
    fn nothing_usable_is_no_answer() {
        assert_eq!(parse("I would rather not.", &allowed()), None);
        assert_eq!(parse(r#"{"action": "fly"}"#, &allowed()), None);
    }

    #[test]
    fn the_schema_offers_the_actions_and_none() {
        let schema = answer_schema(&allowed());
        assert_eq!(
            schema["properties"]["action"]["enum"],
            json!(["stop", "turn_left", ""])
        );
    }

    #[test]
    fn a_user_endpoint_must_be_a_plain_web_address() {
        assert!(safe_base_url("https://api.example.com/v1"));
        assert!(safe_base_url("http://127.0.0.1:1234/v1"));
        assert!(!safe_base_url("file:///etc/passwd"));
        assert!(!safe_base_url("http://169.254.169.254/latest"));
        assert!(!safe_base_url("http://0.0.0.0/v1"));
        assert!(!safe_base_url("not a url"));
    }

    #[test]
    fn the_character_has_a_personality_and_angles_vary() {
        let prompt = system_prompt(&Request {
            situation: String::new(),
            allowed_actions: Vec::new(),
            lang: "it".into(),
            ..Default::default()
        });
        assert!(prompt.starts_with(PERSONALITY));
        assert!(prompt.contains("Italian"));
        let angles: std::collections::HashSet<&str> =
            (0..200u64).map(|i| angle(i * 7919)).collect();
        assert!(
            angles.len() > ANGLES.len() * 3 / 4,
            "{} angles of {}",
            angles.len(),
            ANGLES.len()
        );
        let message = user_message(
            &Request {
                situation: "Now: x".into(),
                allowed_actions: Vec::new(),
                lang: "it".into(),
                ..Default::default()
            },
            1,
        );
        assert!(message.starts_with("Now: x\nThis time, go for: "));
    }

    #[test]
    fn a_persona_replaces_the_character() {
        let request = Request {
            persona: "You are Gina, the baker.".into(),
            situation: "Where were you?".into(),
            lang: "it".into(),
            ..Default::default()
        };
        let prompt = system_prompt(&request);
        assert!(prompt.starts_with("You are Gina, the baker."));
        assert!(!prompt.contains(PERSONALITY));
        assert_eq!(user_message(&request, 3), "Where were you?");
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
            ..Default::default()
        });
        assert!(prompt.contains(r#"one of ["stop", "turn_left"]"#));
        assert!(prompt.contains("Italian"));
    }
}
