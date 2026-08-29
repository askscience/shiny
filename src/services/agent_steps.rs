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
/// `plugins_hint` keeps the plugin windows catalog visible so the model can
/// still call `show_plugin` on later turns.
pub fn build_continuation_messages(
    ai_name: &str,
    lang: &str,
    mode: &str,
    user_message: &str,
    completed_steps: &[String],
    plugins_hint: &str,
) -> Vec<(String, String)> {
    let plugins_line = if plugins_hint.is_empty() {
        String::new()
    } else {
        format!(
            "Plugin windows: {plugins_hint}. If the request belongs to one of them and you \
             haven't shown its window yet, call show_plugin with its name before your final reply. \
             An (inactive) plugin needs {{\"action\":\"plugin_activate\",\"params\":{{\"name\":\"<plugin>\"}}}} \
             before you can use its tools.\n"
        )
    };
    let system = format!(
        "You are {ai_name}. Language: {lang}. Mode: {mode} — keep spoken replies to 1-2 short sentences.\n\
         Call exactly ONE tool per turn (raw JSON line, no markdown) or reply in plain language if done.\n\
         Format: {{\"action\":\"tool_name\",\"params\":{{...}}}}\n\
         {plugins_line}"
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
        "web_search" => {
            // The final reply is generated from these step notes — carry the
            // actual summary + top hits or the model has nothing to answer with.
            let mut note = String::from("Web search completed");
            if let Some(summary) = data
                .get("summary")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                note.push_str(&format!(": {summary}"));
            }
            if let Some(results) = data.get("results").and_then(|v| v.as_array()) {
                for r in results.iter().take(4) {
                    let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let snippet = r.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
                    if title.is_empty() && snippet.is_empty() {
                        continue;
                    }
                    note.push_str(&format!("\n- {title}: {snippet}"));
                }
            }
            note
        }
        "show_artifact" | "update_artifact" => {
            let title = data
                .pointer("/artifact/title")
                .and_then(|v| v.as_str())
                .unwrap_or("card");
            format!("Updated {title}")
        }
        "generate_diary" => "Diary entry saved".into(),
        "show_plugin" => {
            let name = data
                .get("plugin")
                .and_then(|v| v.as_str())
                .unwrap_or("plugin");
            format!("Showing {name}")
        }
        "plugin_activate" => {
            let name = data.get("plugin").and_then(|v| v.as_str()).unwrap_or("plugin");
            if data.get("already").and_then(|v| v.as_bool()).unwrap_or(false) {
                format!("Plugin {name} was already active")
            } else if let Some(err) = data.get("error").and_then(|v| v.as_str()) {
                format!("Could not activate plugin {name}: {err}")
            } else {
                format!("Activated plugin {name}")
            }
        }
        "plugin_deactivate" => {
            let name = data.get("plugin").and_then(|v| v.as_str()).unwrap_or("plugin");
            if data.get("already").and_then(|v| v.as_bool()).unwrap_or(false) {
                format!("Plugin {name} was already inactive")
            } else if let Some(err) = data.get("error").and_then(|v| v.as_str()) {
                format!("Could not deactivate plugin {name}: {err}")
            } else {
                format!("Deactivated plugin {name}")
            }
        }
        "list_plugins" => {
            let lines: Vec<String> = data
                .get("plugins")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .map(|p| {
                            let n = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                            let d = p.get("description").and_then(|v| v.as_str()).unwrap_or("");
                            let a = p.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
                            format!("{n} ({}) — {d}", if a { "active" } else { "inactive" })
                        })
                        .collect()
                })
                .unwrap_or_default();
            if lines.is_empty() {
                "No plugins installed".into()
            } else {
                format!("Plugins:\n{}", lines.join("\n"))
            }
        }
        "youtube_search" => {
            // Carry the top hits into the conversation like web_search does.
            let mut note = String::from("YouTube results");
            if let Some(results) = data.get("results").and_then(|v| v.as_array()) {
                for r in results.iter().take(4) {
                    let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let channel = r.get("channel").and_then(|v| v.as_str()).unwrap_or("");
                    if title.is_empty() {
                        continue;
                    }
                    note.push_str(&format!("\n- {title} — {channel}"));
                }
            }
            note
        }
        "youtube_play" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("video");
            format!("Playing {title} on YouTube")
        }
        _ => {
            // Unknown/plugin tools: the outcome DATA is the result — hand it
            // to the model (truncated), or it has nothing to answer with.
            let ser = serde_json::to_string(data).unwrap_or_default();
            let trimmed: String = ser.chars().take(2000).collect();
            if trimmed.is_empty() || trimmed == "{}" {
                format!("{action} complete")
            } else {
                format!("{action} complete: {trimmed}")
            }
        }
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
        "show_plugin" => "Opening plugin…",
        "plugin_activate" => "Activating plugin…",
        "plugin_deactivate" => "Deactivating plugin…",
        "list_plugins" => "Checking plugins…",
        "youtube_search" => "Searching YouTube…",
        "youtube_play" => "Playing on YouTube…",
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
            "single",
            "Plan Rome",
            &["Step 1".into()],
            "traveler: Trip tracking",
        );
        assert!(msgs[0].1.len() < 700);
        assert!(msgs.iter().any(|(_, c)| c.contains("[Done]")));
    }

    #[test]
    fn continuation_without_plugins_omits_hint() {
        let msgs = build_continuation_messages("Shiny", "en", "single", "Hi", &[], "");
        assert!(!msgs[0].1.contains("Plugin windows"));
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

    #[test]
    fn web_search_step_carries_summary_and_top_hits() {
        let data = json!({
            "summary": "Paris is the capital of France.",
            "results": [
                { "title": "Paris - Wikipedia", "snippet": "The capital and largest city of France." },
                { "title": "Britannica", "snippet": "Capital city of France." },
                { "title": "Mappr", "snippet": "What is the capital of France?" }
            ]
        });
        let step = describe_tool_step("web_search", "ok", &data);
        assert!(step.contains("Paris is the capital of France."), "summary missing: {step}");
        assert!(step.contains("Paris - Wikipedia"), "top hit missing: {step}");
        assert!(step.contains("Britannica"), "second hit missing: {step}");
        assert!(step.contains("What is the capital of France?"), "third hit missing: {step}");
    }

    #[test]
    fn plugin_activate_step_mentions_plugin_name() {
        let step = describe_tool_step("plugin_activate", "ok", &json!({ "plugin": "radio", "enabled": true, "already": false }));
        assert!(step.contains("Activated plugin radio"), "activate note: {step}");

        let step = describe_tool_step("plugin_activate", "ok", &json!({ "plugin": "radio", "already": true }));
        assert!(step.contains("already active"), "already-active note: {step}");

        let step = describe_tool_step("plugin_activate", "error", &json!({ "error": "plugin 'nope' is not installed" }));
        assert!(step.contains("not installed"), "error note: {step}");
    }

    #[test]
    fn plugin_deactivate_and_list_steps() {
        let step = describe_tool_step("plugin_deactivate", "ok", &json!({ "plugin": "word", "enabled": false, "already": false }));
        assert!(step.contains("Deactivated plugin word"), "deactivate note: {step}");

        let step = describe_tool_step(
            "list_plugins",
            "ok",
            &json!({ "plugins": [
                { "name": "radio", "active": true, "description": "Internet radio" },
                { "name": "hello", "active": false, "description": "Demo plugin" }
            ]}),
        );
        assert!(step.contains("radio (active)"), "list note: {step}");
        assert!(step.contains("hello (inactive)"), "list note: {step}");
    }

    #[test]
    fn continuation_hint_teaches_activation() {
        let msgs = build_continuation_messages(
            "Shiny",
            "en",
            "single",
            "Play radio",
            &[],
            "radio: Internet radio (inactive)",
        );
        assert!(msgs[0].1.contains("plugin_activate"), "hint should teach activation: {}", msgs[0].1);
    }

    #[test]
    fn youtube_search_step_carries_top_hits() {
        let data = json!({
            "results": [
                { "title": "Never Gonna Give You Up", "channel": "Rick Astley" },
                { "title": "Rick Astley live", "channel": "Rick Astley Official" }
            ]
        });
        let step = describe_tool_step("youtube_search", "ok", &data);
        assert!(step.contains("YouTube results"), "missing header: {step}");
        assert!(step.contains("Never Gonna Give You Up"), "missing hit: {step}");
        assert!(step.contains("Rick Astley"), "missing channel: {step}");
    }

    #[test]
    fn youtube_play_step_mentions_title() {
        let step = describe_tool_step("youtube_play", "ok", &json!({ "video_id": "abc", "title": "Rick Astley - Never Gonna Give You Up" }));
        assert!(step.contains("Playing Rick Astley"), "play note: {step}");
        assert_eq!(step_label_for_action("youtube_play"), "Playing on YouTube…");
    }
}
