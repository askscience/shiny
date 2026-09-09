//! Step-by-step agent conversation — small prompts per Ollama call.

use serde_json::{json, Value};

pub fn build_planning_messages(
    full_system: &str,
    user_message: &str,
) -> Vec<(String, String)> {
    vec![
        ("system".to_string(), full_system.to_string()),
        ("user".to_string(), user_message.to_string()),
    ]
}

/// After the first tool: tiny system + user request + conversation history +
/// completed step notes. `plugins_hint` keeps the plugin windows catalog
/// visible so the model can still call `show_plugin` on later turns.
/// `history` carries the earlier turns of this conversation so the model never
/// "forgets" what was being discussed while it is mid-tool-loop.
pub fn build_continuation_messages(
    ai_name: &str,
    lang: &str,
    mode: &str,
    user_message: &str,
    completed_steps: &[String],
    plugins_hint: &str,
    history: &[(String, String)],
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
    let history_line = if history.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = history
            .iter()
            .map(|(role, content)| format!("{role}: {content}"))
            .collect();
        format!(
            "\n## Conversation history (remember earlier turns)\n{}\n",
            lines.join("\n")
        )
    };
    let system = format!(
        "You are {ai_name}. Language: {lang}. Mode: {mode} — answer fully and clearly; be concise for simple questions but give detail when helpful.\n\
         Call exactly ONE tool per turn (raw JSON line, no markdown) or reply in plain language if done.\n\
         Format: {{\"action\":\"tool_name\",\"params\":{{...}}}}\n\
         {plugins_line}\
         {history_line}"
    );

    let mut messages = vec![
        ("system".to_string(), system),
        ("user".to_string(), user_message.to_string()),
    ];

    for step in completed_steps {
        messages.push(("user".to_string(), format!("[Done] {step}")));
    }

    messages
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
        "plugin_deactivate_all" => {
            let n = data.get("deactivated").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            format!("Deactivated {n} plugin(s)")
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
        "calc_read" => {
            // Carry the cell values into the conversation as readable lines so
            // the model can compute from them without re-serializing JSON.
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("spreadsheet");
            let mut note = format!("Spreadsheet \"{title}\"");
            if let Some(cells) = data.get("cells").and_then(|v| v.as_object()) {
                let mut entries: Vec<(String, String)> = cells
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect();
                entries.sort_by(|a, b| cell_ref_order(&a.0).cmp(&cell_ref_order(&b.0)));
                for (cell, value) in entries.iter() {
                    note.push_str(&format!("\n{cell}: {value}"));
                }
            }
            note
        }
        "calc_write" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("spreadsheet");
            let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("Wrote {n} cells to \"{title}\"")
        }
        "calc_clear" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("spreadsheet");
            let n = data.get("cleared").and_then(|v| v.as_u64()).unwrap_or(0);
            format!("Cleared {n} cells from \"{title}\"")
        }
        "calc_create" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
            format!("Created spreadsheet \"{title}\"")
        }
        "calc_delete" => "Spreadsheet deleted".into(),
        "desktop_fullscreen" => {
            let name = data.get("plugin").and_then(|v| v.as_str()).unwrap_or("window");
            if data.get("fullscreen").and_then(|v| v.as_bool()).unwrap_or(true) {
                format!("Fullscreened {name}")
            } else {
                format!("Restored {name} from fullscreen")
            }
        }
        "desktop_focus" => {
            let name = data.get("plugin").and_then(|v| v.as_str()).unwrap_or("window");
            format!("Focused {name}")
        }
        "workspace_create" => "Created a new workspace".into(),
        "workspace_remove" => "Removed the current workspace".into(),
        "workspace_switch" => {
            let to = data.get("workspace").and_then(|v| v.as_str()).unwrap_or("next");
            if to == "next" || to == "prev" {
                format!("Switched to the {to} workspace")
            } else {
                format!("Switched to workspace {}", workspace_display_number(to))
            }
        }
        "workspace_move" => {
            let name = data.get("plugin").and_then(|v| v.as_str()).unwrap_or("window");
            let to = data.get("workspace").and_then(|v| v.as_str()).unwrap_or("new");
            if to == "new" {
                format!("Moved {name} to a new workspace")
            } else {
                format!("Moved {name} to workspace {}", workspace_display_number(to))
            }
        }
        // ── Mail ─────────────────────────────────────────────────────────
        "mail_read" => {
            let subject = data.get("subject").and_then(|v| v.as_str()).unwrap_or("(no subject)");
            let from = data.get("from").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            let date = data.get("date").and_then(|v| v.as_str()).unwrap_or("");
            let text = data.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let html = data.get("html").and_then(|v| v.as_str()).unwrap_or("");
            let attachments = data.get("attachments").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.get("filename").and_then(|f| f.as_str())).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            let body = if !text.trim().is_empty() { text } else { html };
            let mut note = format!("Email from {from} ({date}): {subject}\n\n{body}");
            if !attachments.is_empty() {
                note.push_str(&format!("\n\nAttachments: {attachments}"));
            }
            note
        }
        "mail_list" => {
            let folder = data.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");
            let mut note = format!("Messages in {folder}:");
            if let Some(msgs) = data.get("messages").and_then(|v| v.as_array()) {
                for m in msgs {
                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let subject = m.get("subject").and_then(|v| v.as_str()).unwrap_or("(no subject)");
                    let from = m.get("from").and_then(|v| v.as_str()).unwrap_or("");
                    let date = m.get("date").and_then(|v| v.as_str()).unwrap_or("");
                    note.push_str(&format!("\n- id={id} | {subject} | {from} | {date}"));
                }
            }
            note
        }
        "mail_status" => {
            let accounts = data.get("accounts").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.get("email").and_then(|e| e.as_str())).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            format!("Mail accounts: {accounts}")
        }
        "mail_search" => {
            let query = data.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let folder = data.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");
            let mut note = format!("Search results for \"{query}\" in {folder}:");
            if let Some(msgs) = data.get("messages").and_then(|v| v.as_array()) {
                for m in msgs {
                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let subject = m.get("subject").and_then(|v| v.as_str()).unwrap_or("(no subject)");
                    let from = m.get("from").and_then(|v| v.as_str()).unwrap_or("");
                    let date = m.get("date").and_then(|v| v.as_str()).unwrap_or("");
                    note.push_str(&format!("\n- id={id} | {subject} | {from} | {date}"));
                }
            }
            note
        }
        // ── Word ─────────────────────────────────────────────────────────
        "doc_read" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("document");
            let doc_id = data.get("doc_id").and_then(|v| v.as_str()).unwrap_or("");
            let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
            format!("Document \"{title}\" (id={doc_id}):\n{content}")
        }
        "doc_list" => {
            let mut note = String::from("Documents:");
            if let Some(docs) = data.get("documents").and_then(|v| v.as_array()) {
                for d in docs {
                    let id = d.get("doc_id").and_then(|v| v.as_str()).unwrap_or("");
                    let title = d.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let updated = d.get("updated_at").and_then(|v| v.as_str()).unwrap_or("");
                    note.push_str(&format!("\n- id={id} | {title} | {updated}"));
                }
            }
            note
        }
        "doc_create" | "doc_write" | "doc_edit" | "doc_append" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
            let doc_id = data.get("doc_id").and_then(|v| v.as_str()).unwrap_or("");
            format!("{action} document \"{title}\" (id={doc_id})")
        }
        // ── Impress ──────────────────────────────────────────────────────
        "slide_read" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("presentation");
            let deck_id = data.get("deck_id").and_then(|v| v.as_str()).unwrap_or("");
            let slides = data.get("slides").cloned().unwrap_or_else(|| json!({}));
            format!("Presentation \"{title}\" (id={deck_id}):\n{slides}")
        }
        "slide_list" => {
            let mut note = String::from("Presentations:");
            if let Some(decks) = data.get("presentations").and_then(|v| v.as_array()) {
                for d in decks {
                    let id = d.get("deck_id").and_then(|v| v.as_str()).unwrap_or("");
                    let title = d.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let count = d.get("slide_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    note.push_str(&format!("\n- id={id} | {title} | {count} slides"));
                }
            }
            note
        }
        "slide_create" | "slide_write" | "slide_edit" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
            let deck_id = data.get("deck_id").and_then(|v| v.as_str()).unwrap_or("");
            format!("{action} presentation \"{title}\" (id={deck_id})")
        }
        // ── Studio ───────────────────────────────────────────────────────
        "studio_get" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("track");
            let track_id = data.get("track_id").and_then(|v| v.as_str()).unwrap_or("");
            let config = data.get("config").cloned().unwrap_or_else(|| json!({}));
            format!("Track \"{title}\" (id={track_id}):\n{config}")
        }
        "studio_list" => {
            let mut note = String::from("Studio tracks:");
            if let Some(tracks) = data.get("tracks").and_then(|v| v.as_array()) {
                for t in tracks {
                    let id = t.get("track_id").and_then(|v| v.as_str()).unwrap_or("");
                    let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let bpm = t.get("bpm").and_then(|v| v.as_u64()).unwrap_or(0);
                    note.push_str(&format!("\n- id={id} | {title} | {bpm} BPM"));
                }
            }
            note
        }
        "studio_create" | "studio_render" | "studio_update" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("track");
            let track_id = data.get("track_id").and_then(|v| v.as_str()).unwrap_or("");
            format!("{action} track \"{title}\" (id={track_id})")
        }
        "studio_arrangement_get" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("arrangement");
            let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
            format!(
                "Arrangement \"{title}\" (id={id}):\n{}",
                serde_json::to_string(data).unwrap_or_default()
            )
        }
        "studio_arrangement_list" => {
            let mut note = String::from("Arrangements:");
            if let Some(arrs) = data.get("arrangements").and_then(|v| v.as_array()) {
                for a in arrs {
                    let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let title = a.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    note.push_str(&format!("\n- id={id} | {title}"));
                }
            }
            note
        }
        "studio_arrangement_save" => {
            let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("arrangement");
            let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
            format!("Saved arrangement \"{title}\" (id={id})")
        }
        "studio_preset_list" => {
            let mut note = String::from("Presets:");
            if let Some(presets) = data.get("presets").and_then(|v| v.as_array()) {
                for p in presets {
                    let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let kind = p.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    note.push_str(&format!("\n- id={id} | {kind} | {name}"));
                }
            }
            note
        }
        _ => {
            // Unknown/plugin tools: the outcome DATA is the result — hand the
            // FULL payload to the model so nothing is lost.
            let ser = serde_json::to_string(data).unwrap_or_default();
            if ser.is_empty() || ser == "{}" {
                format!("{action} complete")
            } else {
                format!("{action} complete: {ser}")
            }
        }
    }
}

/// Workspace indices travel 0-based in tool data; show them 1-based to the user.
fn workspace_display_number(raw: &str) -> u32 {
    raw.parse::<u32>().map(|n| n + 1).unwrap_or(1)
}

/// Sort key for A1-style cell refs: row-major (A1, B1, … A2, B2, …).
fn cell_ref_order(cell_ref: &str) -> (u32, u32) {
    let bytes = cell_ref.as_bytes();
    let mut i = 0;
    let mut col: u32 = 0;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        col = col * 26 + (bytes[i].to_ascii_uppercase() - b'A' + 1) as u32;
        i += 1;
    }
    let row: u32 = cell_ref[i..].parse().unwrap_or(0);
    (row, col)
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
        "plugin_deactivate_all" => "Closing all plugins…",
        "list_plugins" => "Checking plugins…",
        "desktop_fullscreen" => "Fullscreening…",
        "desktop_focus" => "Focusing window…",
        "workspace_create" => "New workspace…",
        "workspace_remove" => "Removing workspace…",
        "workspace_switch" => "Switching workspace…",
        "workspace_move" => "Moving window…",
        "youtube_search" => "Searching YouTube…",
        "youtube_play" => "Playing on YouTube…",
        "calc_create" => "Creating spreadsheet…",
        "calc_write" => "Writing cells…",
        "calc_clear" => "Clearing spreadsheet…",
        "calc_read" => "Reading spreadsheet…",
        "calc_list" => "Listing spreadsheets…",
        "calc_delete" => "Deleting spreadsheet…",
        "slide_create" => "Building slides…",
        "slide_write" => "Writing slides…",
        "slide_edit" => "Editing slides…",
        "slide_read" => "Reading slides…",
        "slide_list" => "Listing presentations…",
        "slide_delete" => "Deleting presentation…",
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
            &[],
        );
        assert!(msgs[0].1.len() < 700);
        assert!(msgs.iter().any(|(_, c)| c.contains("[Done]")));
    }

    #[test]
    fn continuation_without_plugins_omits_hint() {
        let msgs = build_continuation_messages("Shiny", "en", "single", "Hi", &[], "", &[]);
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
            &[],
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

    #[test]
    fn desktop_workspace_steps_are_human_and_1_based() {
        // Move-to-new is the "organize the desktop" path.
        let step = describe_tool_step("workspace_move", "ok", &json!({ "plugin": "radio", "workspace": "new" }));
        assert!(step.contains("Moved radio to a new workspace"), "new-workspace note: {step}");

        // Numeric workspace ids are 0-based in data but shown 1-based.
        let step = describe_tool_step("workspace_move", "ok", &json!({ "plugin": "calc", "workspace": "0" }));
        assert!(step.contains("workspace 1"), "1-based note: {step}");

        let step = describe_tool_step("workspace_switch", "ok", &json!({ "workspace": "1" }));
        assert!(step.contains("workspace 2"), "switch note: {step}");

        let step = describe_tool_step("workspace_switch", "ok", &json!({ "workspace": "next" }));
        assert!(step.contains("next workspace"), "next note: {step}");
    }
}
