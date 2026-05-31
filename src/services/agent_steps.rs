//! Step-by-step agent conversation — small prompts per Ollama call.

use serde_json::Value;

const MAX_STEP_NOTES: usize = 6;

pub fn build_planning_messages(
    full_system: &str,
    user_message: &str,
) -> Vec<(String, String)> {
    vec![
        ("system".to_string(), full_system.to_string()),
        ("user".to_string(), user_message.to_string()),
    ]
}

/// After the first tool: tiny system + user request + recent step notes only.
pub fn build_continuation_messages(
    ai_name: &str,
    lang: &str,
    user_message: &str,
    completed_steps: &[String],
) -> Vec<(String, String)> {
    let system = format!(
        "You are {ai_name}, a travel navigator. Language: {lang}.\n\
         Call exactly ONE tool per turn (raw JSON line, no markdown) or reply in plain language if done.\n\
         Format: {{\"action\":\"tool_name\",\"params\":{{...}}}}"
    );

    let mut messages = vec![
        ("system".to_string(), system),
        ("user".to_string(), user_message.to_string()),
    ];

    for step in recent_steps(completed_steps) {
        messages.push(("user".to_string(), format!("[Done] {step}")));
    }

    messages
}

/// Final spoken reply — no tool docs, only step outcomes.
pub fn build_final_reply_messages(
    ai_name: &str,
    lang: &str,
    user_message: &str,
    completed_steps: &[String],
) -> Vec<(String, String)> {
    let steps = recent_steps(completed_steps).join("\n");
    let system = format!(
        "You are {ai_name}. Reply in language '{lang}' with 1-2 short spoken sentences.\n\
         Summarize what was accomplished for the user. Do not call tools. Do not mention JSON."
    );
    let user = format!(
        "User asked: {user_message}\n\nSteps completed:\n{steps}\n\nReply to the user now."
    );
    vec![
        ("system".to_string(), system),
        ("user".to_string(), user),
    ]
}

fn recent_steps(steps: &[String]) -> Vec<String> {
    steps
        .iter()
        .rev()
        .take(MAX_STEP_NOTES)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn describe_tool_step(action: &str, result: &str, data: &Value) -> String {
    if result == "error" {
        let err = data
            .get("error")
            .or_else(|| data.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return format!("{action} failed: {err}");
    }
    if result != "ok" {
        return format!("{action}: {result}");
    }

    match action {
        "plan_trip" => {
            let dest = data
                .pointer("/destination/name")
                .and_then(|v| v.as_str())
                .unwrap_or("destination");
            let guides = data
                .get("guides_created")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let route = data.get("route").and_then(|r| {
                let km = r.get("distance_km")?.as_f64()?;
                Some(format!("{km:.0} km drive"))
            });
            match route {
                Some(r) => format!("Planned trip to {dest} — {guides} guides ({r})"),
                None => format!("Planned trip to {dest} — {guides} guides"),
            }
        }
        "navigate_to" => {
            let dest = data
                .pointer("/navigator/destination")
                .and_then(|v| v.as_str())
                .unwrap_or("destination");
            format!("Navigating to {dest}")
        }
        "create_trip" | "start_trip" | "end_trip" => {
            let name = data
                .pointer("/trip/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if name.is_empty() {
                format!("{action} complete")
            } else {
                format!("{action}: {name}")
            }
        }
        "list_trips" => {
            let n = data
                .get("trips")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            format!("Listed {n} trip(s)")
        }
        "web_search" => "Saved search results".into(),
        "show_artifact" | "update_artifact" => {
            let title = data
                .pointer("/artifact/title")
                .and_then(|v| v.as_str())
                .unwrap_or("card");
            format!("Updated {title}")
        }
        "generate_diary" => "Diary entry saved".into(),
        _ => format!("{action} complete"),
    }
}

pub fn step_label_for_action(action: &str) -> &'static str {
    match action {
        "plan_trip" => "Planning your trip…",
        "navigate_to" => "Starting navigation…",
        "web_search" => "Searching the web…",
        "create_trip" => "Creating trip…",
        "start_trip" => "Starting trip…",
        "end_trip" => "Ending trip…",
        "list_trips" => "Loading trips…",
        "generate_diary" => "Writing diary…",
        "show_artifact" | "update_artifact" => "Updating guide…",
        _ => "Working…",
    }
}

pub fn messages_char_count(messages: &[(String, String)]) -> usize {
    messages.iter().map(|(_, c)| c.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn continuation_omits_full_skill_doc() {
        let msgs = build_continuation_messages(
            "Shiny",
            "en",
            "Plan Rome",
            &["Step 1".into()],
        );
        assert!(msgs[0].1.len() < 500);
        assert!(msgs.iter().any(|(_, c)| c.contains("[Done]")));
    }

    #[test]
    fn plan_trip_step_mentions_guides_not_geometry() {
        let data = json!({
            "destination": { "name": "Rome" },
            "guides_created": 4,
            "route": { "distance_km": 120.0, "geometry_points": 9000 }
        });
        let step = describe_tool_step("plan_trip", "ok", &data);
        assert!(step.contains("4 guides"));
        assert!(!step.contains("9000"));
    }
}
