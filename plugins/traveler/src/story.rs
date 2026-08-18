//! Prose pipeline behind `plan_trip` — research → fact extraction → Ollama
//! narrative writing, with soft-fail fallbacks when Ollama or search is
//! unavailable. Ported from core's agent_tools helpers; behavior preserved.

use serde_json::Value;
use uuid::Uuid;

use shiny_plugin_sdk::artifacts::{Artifact, ArtifactSection, Coordinates, RouteMeta};
use shiny_plugin_sdk::services::{PluginCtx, SearchResult};

/// Shared tone for artifact prose — practical first, lightly descriptive.
const ARTIFACT_WRITER_TONE: &str = "Write in a warm but practical tone. Name real places, neighborhoods, times, and logistics. \
    A little atmosphere is fine; avoid flowery or overly poetic language.";

async fn extract_travel_facts(
    ctx: &PluginCtx,
    destination: &str,
    lang: &str,
    results: &[SearchResult],
    model: Option<&str>,
) -> Vec<String> {
    let mut facts = Vec::new();
    for row in results.iter().take(5) {
        let prompt = format!(
            "Extract one practical travel fact about {destination} for language '{lang}' from:\n- {}: {}\n\
             Focus on what to do, where, when, or how — not poetry. Reply with one sentence only.",
            row.title, row.snippet
        );
        if let Ok(line) = ctx.ollama().await.generate(&prompt, None, model).await {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                facts.push(trimmed.to_string());
            }
        }
    }
    facts
}

async fn extract_lodging_facts(
    ctx: &PluginCtx,
    destination: &str,
    lang: &str,
    results: &[SearchResult],
    model: Option<&str>,
) -> Vec<String> {
    let mut facts = Vec::new();
    for row in results.iter().take(5) {
        let prompt = format!(
            "Extract one practical lodging fact for {destination} (language '{lang}') from:\n- {}: {}\n\
             Hotels, neighborhoods to stay, B&Bs, or overnight road stops — be specific when possible. \
             One sentence only.",
            row.title, row.snippet
        );
        if let Ok(line) = ctx.ollama().await.generate(&prompt, None, model).await {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                facts.push(trimmed.to_string());
            }
        }
    }
    facts
}

pub async fn gather_lodging_facts(
    ctx: &PluginCtx,
    destination: &str,
    num_days: u32,
    route_meta: Option<&RouteMeta>,
    lang: &str,
    model: Option<&str>,
) -> Vec<String> {
    let needs_stay = num_days > 1;
    let long_drive = route_meta.map(|r| r.distance_km > 400.0).unwrap_or(false);
    if !needs_stay && !long_drive {
        return vec![];
    }

    let mut rows = Vec::new();
    if needs_stay {
        if let Ok(found) = ctx
            .search()
            .await
            .search(&format!("{destination} hotels where to stay best neighborhoods"))
            .await
        {
            rows.extend(found);
        }
    }
    if long_drive {
        if let Ok(found) = ctx
            .search()
            .await
            .search(&format!("drive to {destination} overnight stop hotels motels"))
            .await
        {
            rows.extend(found);
        }
    }

    if rows.is_empty() || !ctx.ollama().await.is_available().await {
        return vec![];
    }

    extract_lodging_facts(ctx, destination, lang, &rows, model).await
}

pub async fn build_overview_story(
    ctx: &PluginCtx,
    destination: &str,
    num_days: u32,
    lang: &str,
    results: &[SearchResult],
    lodging_facts: &[String],
    model: Option<&str>,
) -> (String, Vec<ArtifactSection>) {
    if ctx.ollama().await.is_available().await {
        let facts = extract_travel_facts(ctx, destination, lang, results, model).await;
        if !facts.is_empty() {
            let facts_text = facts.join("\n");
            let lodging_block = if lodging_facts.is_empty() {
                String::new()
            } else {
                format!(
                    "\n\nLodging notes (use for nightly stays):\n{}",
                    lodging_facts.join("\n")
                )
            };
            let day_schema = if num_days > 1 {
                r#"{"title":"Day 1 — short theme","story":"One paragraph: morning/afternoon plan with named places and rough times","stay":"Where to sleep that night — neighborhood or hotel from lodging notes"}"#
            } else {
                r#"{"title":"Day 1 — short theme","story":"One paragraph: what to see and do with named places and rough times"}"#
            };
            let stay_rule = if num_days > 1 {
                "\n- Each day MUST include \"stay\" with a concrete place or neighborhood to sleep.\n\
                 - Use the same base hotel across nights when that makes sense."
            } else {
                ""
            };
            let prompt = format!(
                "{ARTIFACT_WRITER_TONE}\n\
                 Write for language '{lang}' about visiting {destination} for {num_days} days.\n\
                 Research notes (one fact per line):\n{facts_text}{lodging_block}\n\n\
                 Reply with ONLY valid JSON (no markdown):\n\
                 {{\"intro\":\"1-2 short paragraphs: trip overview with practical tips\",\"days\":[{day_schema}]}}\n\
                 Include exactly {num_days} day objects.{stay_rule}\n\
                 No bullet lists inside story text.",
            );
            if let Ok(raw) = ctx.ollama().await.generate(&prompt, None, model).await {
                if let Ok(v) = serde_json::from_str::<Value>(&extract_json_object(&raw)) {
                    let intro = v
                        .get("intro")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let sections: Vec<ArtifactSection> = v
                        .get("days")
                        .and_then(|d| d.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|item| {
                                    let title = item
                                        .get("title")
                                        .and_then(|t| t.as_str())
                                        .unwrap_or("Day")
                                        .to_string();
                                    let story = item
                                        .get("story")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("")
                                        .trim();
                                    let stay = item
                                        .get("stay")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("")
                                        .trim();
                                    let value = if stay.is_empty() {
                                        story.to_string()
                                    } else {
                                        format!("{story}\n\nWhere to sleep: {stay}")
                                    };
                                    if value.is_empty() {
                                        return None;
                                    }
                                    Some(ArtifactSection { label: title, value })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if !intro.is_empty() {
                        return (intro, sections);
                    }
                }
            }
        }
    }

    let snippets: Vec<String> = results
        .iter()
        .take(8)
        .map(|r| format!("- {}: {}", r.title, r.snippet))
        .collect();
    let intro = format!(
        "{destination} over {num_days} days — a practical base for exploring. \
         Key pointers from research: {}",
        snippets.join(" ")
    );
    let sections = (1..=num_days)
        .map(|d| {
            let stay = if num_days > 1 {
                let hint = lodging_facts
                    .first()
                    .map(|s| format!("\n\nWhere to sleep: {s}"))
                    .unwrap_or_else(|| {
                        "\n\nWhere to sleep: look for hotels in the city centre or main tourist district."
                            .to_string()
                    });
                format!(
                    "Day {d}: pick 2–3 sights, allow time for lunch, and keep the evening open.{hint}"
                )
            } else {
                format!("Day {d}: pick 2–3 sights and allow time for lunch — no need to rush.")
            };
            ArtifactSection {
                label: format!("Day {}", d),
                value: stay,
            }
        })
        .collect();
    (intro, sections)
}

pub async fn build_theme_guide(
    ctx: &PluginCtx,
    destination: &str,
    lang: &str,
    theme: &str,
    artifact_type: &str,
    results: &[SearchResult],
    lat: f64,
    lon: f64,
    model: Option<&str>,
) -> Artifact {
    let theme_label = match theme {
        "nightlife" => "after dark",
        "food" => "food & drink",
        "culture" => "culture & art",
        _ => theme,
    };

    let fallback_copy = || {
        let snippets: Vec<String> = results
            .iter()
            .take(6)
            .map(|r| format!("- {}: {}", r.title, r.snippet))
            .collect();
        (
            theme.to_string(),
            format!("Explore {theme_label} in {destination}. {}", snippets.join(" ")),
        )
    };

    let (title, narrative) = if ctx.ollama().await.is_available().await {
        let facts = extract_travel_facts(ctx, destination, lang, results, model).await;
        if !facts.is_empty() {
            let prompt = format!(
                "{ARTIFACT_WRITER_TONE}\n\
                 Write a practical tourist guide about {theme_label} in {destination} for language '{lang}'.\n\
                 Research notes:\n{notes}\n\n\
                 Reply with ONLY JSON: {{\"title\":\"short clear title\",\"narrative\":\"2 short paragraphs: specific places, times, and tips — light color ok, no purple prose\"}}",
                notes = facts.join("\n")
            );
            if let Ok(raw) = ctx.ollama().await.generate(&prompt, None, model).await {
                if let Ok(v) = serde_json::from_str::<Value>(&extract_json_object(&raw)) {
                    let t = v.get("title").and_then(|x| x.as_str()).unwrap_or(theme).to_string();
                    let n = v
                        .get("narrative")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !n.is_empty() {
                        (t, n)
                    } else {
                        (theme.to_string(), facts.join(" "))
                    }
                } else {
                    (theme.to_string(), facts.join(" "))
                }
            } else {
                (theme.to_string(), facts.join(" "))
            }
        } else {
            fallback_copy()
        }
    } else {
        fallback_copy()
    };

    Artifact {
        id: Uuid::new_v4().to_string(),
        artifact_type: artifact_type.to_string(),
        theme: Some(theme.to_string()),
        destination: Some(destination.to_string()),
        title,
        subtitle: Some(destination.to_string()),
        coordinates: Some(Coordinates { lat, lon }),
        narrative: Some(narrative),
        sections: vec![],
        days: vec![],
        route: None,
        geometry: vec![],
        actions: vec![],
    }
}

fn extract_json_object(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return trimmed[start..=end].to_string();
        }
    }
    trimmed.to_string()
}
