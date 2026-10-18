use serde_json::{json, Value};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::{Traveler, Trip};
use crate::services::artifacts::{self, Artifact};

// `AgentContext` now lives in the SDK so plugins can receive the same type.
// Re-export keeps existing call sites (`crate::services::agent_tools::AgentContext`)
// in the binary working.
pub use shiny_plugin_sdk::context::AgentContext;

#[derive(Debug, Clone)]
pub struct ActionOutcome {
    pub action: String,
    pub result: String,
    pub data: Value,
    pub artifact: Option<Artifact>,
    pub extra_artifacts: Vec<Artifact>,
    /// Plugin that produced this outcome ("" / None = core). Used to tag
    /// saved artifacts so the UI can group surfaces by plugin.
    pub owner: Option<String>,
}

/// Dispatch an LLM-emitted action. Core is a simple AI assistant: the only
/// built-in tool is `web_search`. Everything else (trips, GPS, maps, diary,
/// navigation, artifact cards) is contributed by plugins — the traveler
/// plugin owns that domain.
pub async fn execute_action(
    state: &AppState,
    traveler: &Traveler,
    ctx: &AgentContext,
    action: &str,
    params: &Value,
) -> Result<ActionOutcome, AppError> {
    let mut action_key = normalize_action_name(action);

    // The model occasionally namespaces tool names ("word.doc_write",
    // "radio.radio_stop"): when the raw key isn't registered, retry with the
    // segment after the first dot — it usually matches a real tool name.
    if !state.plugins.tools().has(&action_key) {
        if let Some((_, rest)) = action_key.split_once('.') {
            let candidate = normalize_action_name(rest);
            if state.plugins.tools().has(&candidate) {
                action_key = candidate;
            }
        }
    }

    // Focus mode: desktop actions are refused while a window is focused, because
    // they act on the desktop rather than on the plugin the user is working with.
    // Two rules keep this from becoming a trap:
    //   • only REAL core actions are checked — a tool name the model invented
    //     must reach its own "unknown action" error, not be blamed on focus mode;
    //   • web search and the plugin lifecycle stay available (see the list).
    if focus_blocks_core(ctx, &action_key) {
        return Ok(focus_core_refusal(ctx, &action_key));
    }

    // Plugin registry takes priority — if a plugin claimed this action key,
    // dispatch to its `Tool::invoke`. The registry itself consults the per-user
    // activation set, so deactivated plugins refuse here.
    if state.plugins.tools().has(&action_key) {
        let owner = state.plugins.tools().owner_of(&action_key);
        // Only a plugin that is not focused is refused — the focused plugins' own
        // tools are exactly what focus mode is FOR. Checked here, where the owner
        // is a real plugin name, so no invented tool name can be misread.
        if focus_blocks_plugin(ctx, &owner) {
            return Ok(focus_plugin_refusal(ctx, &owner));
        }
        // Focus-only surfaces (a Craft app's MCP tools) exist only while that
        // app's window is in focus mode. The MCP host also stops the server
        // when the app is unfocused; this check keeps a stale registration from
        // ever leaking tools into a turn where nothing is focused.
        if state.plugins.tools().is_focus_only(&owner)
            && !ctx.focused.iter().any(|focused| focused == &owner)
        {
            return Ok(focus_only_refusal(&owner));
        }
        let plugin_ctx = state.plugin_ctx();
        let os_home = state.os_identity_for(traveler).map(|u| u.home);
        let outcome = state
            .plugins
            .tools()
            .invoke(
                &action_key,
                &plugin_ctx,
                &traveler.id,
                &traveler.id,
                params,
                ctx,
                os_home.as_deref(),
            )
            .await?;
        return Ok(ActionOutcome {
            action: outcome.action,
            result: outcome.result,
            data: outcome.data,
            artifact: outcome.artifact,
            extra_artifacts: outcome.extra_artifacts,
            owner: if owner.is_empty() { None } else { Some(owner) },
        });
    }

    // No plugin claimed the verb. If it belongs to the traveler domain, refuse
    // with a clear message (plugin uninstalled, or deactivated for this user).
    let is_traveler_verb = matches!(action_key.as_str(),
        "create_trip" | "list_trips" | "get_trip" | "get_active_trip" | "start_trip"
        | "end_trip" | "trip_stats" | "submit_location" | "list_locations"
        | "trip_route" | "map_search" | "map_reverse" | "map_route"
        | "navigate_to" | "map_poi" | "list_diary" | "get_diary"
        | "search_diary" | "generate_diary" | "plan_trip"
        | "show_artifact" | "update_artifact"
    );
    if is_traveler_verb {
        return Ok(ActionOutcome {
            action: action_key.clone(),
            result: "error".into(),
            data: json!({ "error": "plugin 'traveler' is deactivated for this user" }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        });
    }

    let traveler_active = state
        .plugins
        .session_active_plugin_enabled(&traveler.id, "traveler")
        .await;

    let outcome = match action_key.as_str() {
        // Surface a plugin's window (tile or full screen, per user preference).
        // The frontend receives `focus_plugin` in the agent response.
        "show_plugin" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            let installed = state.plugins.list().iter().any(|m| m.name == name);
            if installed && state.plugins.session_active_plugin_enabled(&traveler.id, &name).await {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "ok".into(),
                    data: json!({ "plugin": name }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            } else if !installed {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({ "error": format!("plugin '{name}' is not installed") }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            } else {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({ "error": format!("plugin '{name}' is inactive — activate it with plugin_activate first") }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            }
        }
        // Activate a plugin for this user so its tools become available.
        "plugin_activate" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            let installed = state.plugins.list().iter().any(|m| m.name == name);
            if !installed {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({ "error": format!("plugin '{name}' is not installed") }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            } else {
                let already = state.plugins.is_enabled_for(&traveler.id, &name).await;
                state.plugins.set_enabled_for(&traveler.id, &name, true).await?;
                ActionOutcome {
                    action: action_key.clone(),
                    result: "ok".into(),
                    data: json!({ "plugin": name, "enabled": true, "already": already }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            }
        }
        // Deactivate a plugin for this user — its tools stop working.
        "plugin_deactivate" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            let installed = state.plugins.list().iter().any(|m| m.name == name);
            if !installed {
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({ "error": format!("plugin '{name}' is not installed") }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            } else {
                let already = !state.plugins.is_enabled_for(&traveler.id, &name).await;
                state.plugins.set_enabled_for(&traveler.id, &name, false).await?;
                ActionOutcome {
                    action: action_key.clone(),
                    result: "ok".into(),
                    data: json!({ "plugin": name, "enabled": false, "already": already }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: None,
                }
            }
        }
        // Deactivate every session-active plugin in one call — "close all
        // plugins". Deterministic: doesn't depend on the model looping through
        // the catalog one window at a time.
        "plugin_deactivate_all" => {
            let active = state.plugins.session_active_set(&traveler.id).await;
            let mut deactivated: Vec<String> = Vec::new();
            for name in &active {
                state.plugins.set_enabled_for(&traveler.id, name, false).await?;
                deactivated.push(name.clone());
            }
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "deactivated": deactivated }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        // Every installed plugin with its activation status — lets the model
        // re-check the catalog mid-conversation (e.g. after activating).
        "list_plugins" => {
            let plugins: Vec<Value> = {
                let mut out = Vec::new();
                for m in state.plugins.list() {
                    let active = state
                        .plugins
                        .session_active_plugin_enabled(&traveler.id, &m.name)
                        .await;
                    out.push(json!({
                        "name": m.name,
                        "active": active,
                        "description": m.description.clone().or_else(|| m.summary.clone()).unwrap_or_default(),
                    }));
                }
                out
            };
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "plugins": plugins }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        // ── Desktop control (Hyprland-style window/workspace management) ──
        // These are relay tools: the desktop state lives in the browser per
        // traveler. The server validates the plugin name and passes the intent
        // back through `actions_taken` so the frontend desktop manager applies
        // it — the same path `show_plugin` uses for `focus_plugin`.
        "desktop_fullscreen" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            if let Some(err) = active_plugin_error(state, &traveler.id, &name, &action_key).await {
                return Ok(err);
            }
            let on = params.get("on").and_then(|v| v.as_bool()).unwrap_or(true);
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "plugin": name, "fullscreen": on }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        "desktop_focus" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            if let Some(err) = active_plugin_error(state, &traveler.id, &name, &action_key).await {
                return Ok(err);
            }
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "plugin": name }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        // Refused on a vertical, phone-like screen (see AgentContext).
        "workspace_create" if !ctx.workspaces_enabled => ActionOutcome {
            action: action_key.clone(),
            result: "unavailable".into(),
            data: json!({ "reason": "workspaces are off on this screen" }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        },
        "workspace_create" => ActionOutcome {
            action: action_key.clone(),
            result: "ok".into(),
            data: json!({ "created": true }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        },
        "workspace_remove" => ActionOutcome {
            action: action_key.clone(),
            result: "ok".into(),
            data: json!({ "removed": true }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        },
        "workspace_switch" => {
            let to = params
                .get("to")
                .map(|v| workspace_target(v, "next"))
                .unwrap_or_else(|| "next".to_string());
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "workspace": to }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        "workspace_move" => {
            let name = param_str(params, "name")
                .or_else(|| param_str(params, "plugin"))
                .ok_or_else(|| AppError::BadRequest("name required".into()))?;
            if let Some(err) = active_plugin_error(state, &traveler.id, &name, &action_key).await {
                return Ok(err);
            }
            // `to` accepts a 1-based number, a numeric string, or "new" to
            // spin up a fresh workspace. Missing/unknown targets default to
            // "new" so "move X to a new workspace" works in a single call.
            let to = params
                .get("to")
                .map(|v| workspace_target(v, "new"))
                .unwrap_or_else(|| "new".to_string());
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "plugin": name, "workspace": to }),
                artifact: None,
                extra_artifacts: vec![],
                owner: None,
            }
        }
        "web_search" => {
            let query = param_str(params, "query").ok_or_else(|| AppError::BadRequest("query required".into()))?;
            // The Instant Answer API is empty for most queries — use the HTML
            // results endpoint first and fall back to it for instant answers.
            let mut results = state.search.search_html(&query, 5).await?;
            if results.is_empty() {
                results = state.search.search(&query).await?;
            }
            // Chat-only mode: without the traveler plugin there is no card
            // surface — the answer travels in `data` and spoken prose only.
            let artifact = if traveler_active {
                Some(Artifact {
                    id: Uuid::new_v4().to_string(),
                    artifact_type: "site_info".into(),
                    title: query.clone(),
                    subtitle: results.first().map(|r| r.snippet.clone()),
                    coordinates: None,
                    sections: results.iter().take(4).map(|r| artifacts::ArtifactSection {
                        label: r.title.clone(),
                        value: r.snippet.clone(),
                    }).collect(),
                    actions: vec![],
                    days: vec![],
                    route: None,
                    geometry: vec![],
                    narrative: None,
                    theme: None,
                    destination: None,
                })
            } else {
                None
            };
            ActionOutcome {
                action: action_key.clone(),
                result: "ok".into(),
                data: json!({ "results": results }),
                artifact,
                extra_artifacts: vec![],
                owner: Some("core".into()),
            }
        }
        "generate_image" => {
            let prompt = param_str(params, "prompt")
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .ok_or_else(|| AppError::BadRequest("prompt required".into()))?;

            let requested_model = match param_str(params, "model") {
                Some(m) => m,
                None => image_model_pref(state, &traveler.id)
                    .await
                    .unwrap_or_else(|| crate::services::image_gen::DEFAULT_FAMILY.to_string()),
            };
            let Some(mut family) = crate::services::image_gen::resolve_family(&requested_model)
            else {
                return Err(AppError::BadRequest(format!(
                    "unknown image model '{requested_model}' (use z-image or qwen-image-2.1)"
                )));
            };

            // Editing: only families with `supports_edit` (Qwen-Image 2.1) can
            // do it. An explicit Z-Image request is an error; otherwise the
            // tool silently switches to the edit-capable family.
            let reference = param_str(params, "image")
                .or_else(|| param_str(params, "reference"))
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty());
            if reference.is_some() && !family.supports_edit {
                if param_str(params, "model").is_some() {
                    return Err(AppError::BadRequest(format!(
                        "the {} model cannot edit images — edits run with qwen-image-2.1",
                        family.label
                    )));
                }
                let Some(edit_family) = crate::services::image_gen::FAMILIES
                    .iter()
                    .copied()
                    .find(|f| f.supports_edit)
                else {
                    return Err(AppError::BadRequest("image editing is not available".into()));
                };
                family = edit_family;
            }

            let requested_quality = match param_str(params, "quality") {
                Some(q) => q,
                None => image_quality_pref(state, &traveler.id)
                    .await
                    .unwrap_or_else(|| family.default_quality.to_string()),
            };
            let Some((quality, ..)) =
                crate::services::image_gen::resolve_quality(family, &requested_quality)
            else {
                return Err(AppError::BadRequest(format!(
                    "unknown image quality '{requested_quality}' (use fast, balanced, high or max)"
                )));
            };

            let os_home = state.os_identity_for(traveler).map(|u| u.home);
            let home = crate::services::image_gen::files_home(&traveler.id, os_home.as_deref());

            // The image to edit: absolute, or relative to the user's files
            // home. Validated here so a typo fails fast; the service copies it
            // before the engine ever sees it.
            let reference_path = match &reference {
                Some(raw) => {
                    let p = std::path::PathBuf::from(raw);
                    let resolved = if p.is_absolute() { p } else { home.join(p) };
                    if !resolved.is_file() {
                        return Err(AppError::BadRequest(format!(
                            "image to edit not found: {}",
                            resolved.display()
                        )));
                    }
                    Some(resolved)
                }
                None => None,
            };

            // Edits default to 1024² (a photo-sized canvas); fresh generations
            // keep the smaller 512² default.
            let (width, height) = parse_image_size(params.get("size")).unwrap_or(
                if reference_path.is_some() { (1024, 1024) } else { (512, 512) },
            );
            let steps = params.get("steps").and_then(|v| v.as_u64()).unwrap_or(8).clamp(1, 50) as u32;
            let seed = match params.get("seed").and_then(|v| v.as_i64()) {
                Some(s) if s >= 0 => s,
                _ => random_seed(),
            };
            // Results open in the Image window by default; `open:false` keeps
            // them in the chat only.
            let open = params.get("open").and_then(|v| v.as_bool()).unwrap_or(true);
            let allow_download = params.get("allow_download").and_then(|v| v.as_bool()).unwrap_or(false);

            let mut missing = state.image_gen.missing_for(family, quality);
            if !missing.is_empty() && allow_download {
                if let Err(e) = state.image_gen.start_download(family, quality) {
                    return Err(AppError::Internal(e));
                }
                let client = state.image_gen.clone();
                let key = quality.to_string();
                client
                    .wait_for_ready(family, &key, crate::services::image_gen::DOWNLOAD_WAIT)
                    .await;
                missing = state.image_gen.missing_for(family, &key);
            }
            if !missing.is_empty() {
                let bytes = state.image_gen.missing_bytes_for(family, quality);
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({
                        "error": "the image models are not downloaded yet",
                        "model": family.id,
                        "needs_download": true,
                        "missing": missing,
                        "download_bytes": bytes,
                        "download_size": crate::services::image_gen::human_bytes(bytes),
                        "hint": "Tell the user how large the download is and ask whether you should fetch it. Only after they agree, call generate_image again with allow_download=true and the same prompt.",
                    }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: Some("core".into()),
                }
            } else {
                let edit_note = reference_path.as_ref().map(|p| p.display().to_string());
                let req = crate::services::image_gen::GenerateRequest {
                    prompt,
                    family: family.id.to_string(),
                    quality: quality.to_string(),
                    width,
                    height,
                    steps,
                    seed,
                    home,
                    reference: reference_path,
                };
                let client = state.image_gen.clone();
                let rendered = tokio::task::spawn_blocking(move || client.generate(&req))
                    .await
                    .map_err(|e| AppError::Internal(format!("image task panicked: {e}")))?;
                match rendered {
                    Ok(res) => {
                        let mut data = json!({
                            "url": res.url,
                            "file": res.file,
                            "path": res.rel_path,
                            "model": res.model,
                            "model_label": family.label,
                            "edit": edit_note.is_some(),
                            "reference": edit_note,
                            "width": res.width,
                            "height": res.height,
                            "steps": res.steps,
                            "seed": res.seed,
                            "quality": res.quality,
                            "quant": res.quant,
                            "seconds": (res.seconds * 10.0).round() / 10.0,
                            "size": crate::services::image_gen::human_bytes(res.bytes),
                            "hint": "End your reply with the image on its own line: ![short description](<url>)",
                        });
                        if open {
                            data["open_file"] = json!({
                                "plugin": "image",
                                "path": res.rel_path,
                                "name": res.file,
                            });
                        }
                        ActionOutcome {
                            action: action_key.clone(),
                            result: "ok".into(),
                            data,
                            artifact: None,
                            extra_artifacts: vec![],
                            owner: Some("core".into()),
                        }
                    }
                    Err(e) => ActionOutcome {
                        action: action_key.clone(),
                        result: "error".into(),
                        data: json!({ "error": e }),
                        artifact: None,
                        extra_artifacts: vec![],
                        owner: Some("core".into()),
                    },
                }
            }
        }
        "generate_song" => {
            let style = param_str(params, "style")
                .or_else(|| param_str(params, "prompt"))
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .ok_or_else(|| AppError::BadRequest("style required".into()))?;
            let lyrics = param_str(params, "lyrics").unwrap_or_default();
            let title = param_str(params, "title")
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| style.clone());
            // Songs bleed wall time: the engine needs ~6 min of NAR per ~75 s
            // of audio on this class of GPU, so the tool bounds the length.
            let duration_secs = params
                .get("duration")
                .and_then(|v| v.as_u64())
                .unwrap_or(90)
                .clamp(30, 300) as u32;
            let seed = params.get("seed").and_then(|v| v.as_u64());
            let allow_download = params.get("allow_download").and_then(|v| v.as_bool()).unwrap_or(false);

            let requested_quality = match param_str(params, "quality") {
                Some(q) => q,
                None => music_quality_pref(state, &traveler.id)
                    .await
                    .unwrap_or_else(|| crate::services::song_gen::DEFAULT_TIER.to_string()),
            };
            let Some(tier) = crate::services::song_gen::resolve_tier(&requested_quality) else {
                return Err(AppError::BadRequest(format!(
                    "unknown song quality '{requested_quality}' (use q4 or q8)"
                )));
            };

            // The weights are useless without the engine binary — refuse with a
            // setup hint instead of offering a download that cannot render.
            if state.song_gen.engine().is_none() {
                return Ok(ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({
                        "error": "the song engine is not installed",
                        "hint": "Tell the user the YuE2 engine (ace-server) is not installed; it is set up with scripts/install-yue2-engine.sh or the YUE2_ENGINE variable.",
                    }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: Some("core".into()),
                });
            }

            let mut missing = state.song_gen.missing_for(tier.key);
            if !missing.is_empty() && allow_download {
                if let Err(e) = state.song_gen.start_download(tier.key) {
                    return Err(AppError::Internal(e));
                }
                let client = state.song_gen.clone();
                let key = tier.key.to_string();
                client
                    .wait_for_ready(&key, crate::services::song_gen::DOWNLOAD_WAIT)
                    .await;
                missing = state.song_gen.missing_for(&key);
            }
            if !missing.is_empty() {
                let bytes = state.song_gen.missing_bytes_for(tier.key);
                ActionOutcome {
                    action: action_key.clone(),
                    result: "error".into(),
                    data: json!({
                        "error": "the song models are not downloaded yet",
                        "needs_download": true,
                        "missing": missing,
                        "download_bytes": bytes,
                        "download_size": crate::services::media::human_bytes(bytes),
                        "hint": "Tell the user how large the download is and ask whether you should fetch it. Only after they agree, call generate_song again with allow_download=true and the same style.",
                    }),
                    artifact: None,
                    extra_artifacts: vec![],
                    owner: Some("core".into()),
                }
            } else {
                let os_home = state.os_identity_for(traveler).map(|u| u.home);
                let home = crate::services::media::files_home(&traveler.id, os_home.as_deref());
                let req = crate::services::song_gen::SongRequest {
                    style,
                    lyrics,
                    title: title.clone(),
                    duration_secs,
                    seed,
                    quality: tier.key.to_string(),
                    home,
                };
                let client = state.song_gen.clone();
                let rendered = tokio::task::spawn_blocking(move || client.generate(&req))
                    .await
                    .map_err(|e| AppError::Internal(format!("song task panicked: {e}")))?;
                match rendered {
                    Ok(res) => ActionOutcome {
                        action: action_key.clone(),
                        result: "ok".into(),
                        data: json!({
                            "url": res.url,
                            "file": res.file,
                            "path": res.rel_path,
                            "title": title,
                            "quality": res.quality,
                            "quant": res.quant,
                            "duration_secs": (res.duration_secs * 10.0).round() / 10.0,
                            "seconds": (res.seconds * 10.0).round() / 10.0,
                            "size": crate::services::media::human_bytes(res.bytes),
                            "hint": "End your reply with the track on its own line: [short title](<url>)",
                        }),
                        artifact: None,
                        extra_artifacts: vec![],
                        owner: Some("core".into()),
                    },
                    Err(e) => ActionOutcome {
                        action: action_key.clone(),
                        result: "error".into(),
                        data: json!({ "error": e }),
                        artifact: None,
                        extra_artifacts: vec![],
                        owner: Some("core".into()),
                    },
                }
            }
        }
        other => {
            tracing::warn!("Unknown agent action: {:?}", other);
            return Err(AppError::BadRequest(format!("Unknown action: {}", other)));
        }
    };

    Ok(outcome)
}

fn param_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// `"768x768"`, `"1024X512"` or a single number → a size pair.
fn parse_image_size(v: Option<&Value>) -> Option<(u32, u32)> {
    let s = v?.as_str()?.trim().to_ascii_lowercase();
    if let Some((w, h)) = s.split_once('x') {
        return Some((w.trim().parse().ok()?, h.trim().parse().ok()?));
    }
    let n: u32 = s.parse().ok()?;
    Some((n, n))
}

/// A fresh seed each call, so "make me an image" varies run to run.
fn random_seed() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_nanos() % 1_000_000_000) as i64)
        .unwrap_or(42)
}

/// The user's saved image-quality preference (Settings → Images).
async fn image_quality_pref(state: &AppState, user_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = 'image.quality'",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .filter(|q| !q.trim().is_empty())
}

/// The user's saved image-model preference (Settings → Images). Unknown values
/// are ignored so a stale or hand-edited row can't break generation.
async fn image_model_pref(state: &AppState, user_id: &str) -> Option<String> {
    let value = sqlx::query_scalar::<_, String>(
        "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = 'image.model'",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .filter(|m| !m.trim().is_empty())?;
    crate::services::image_gen::resolve_family(&value).map(|f| f.id.to_string())
}

/// The user's saved song-quality preference (Settings → Music). Unknown values
/// are ignored so a stale or hand-edited row can't break generation.
async fn music_quality_pref(state: &AppState, user_id: &str) -> Option<String> {
    let value = sqlx::query_scalar::<_, String>(
        "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = 'music.quality'",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .filter(|q| !q.trim().is_empty())?;
    crate::services::song_gen::resolve_tier(&value).map(|t| t.key.to_string())
}

/// Normalize a workspace target: 1-based number or numeric string → 0-based
/// index string; "new" passes through; anything else falls back to `default`.
fn workspace_target(v: &Value, default: &str) -> String {
    if let Some(n) = v.as_u64() {
        return n.saturating_sub(1).to_string();
    }
    if let Some(s) = v.as_str() {
        if s == "new" {
            return "new".to_string();
        }
        if let Ok(n) = s.parse::<u64>() {
            return n.saturating_sub(1).to_string();
        }
        return s.to_string();
    }
    default.to_string()
}

/// Core actions focus mode refuses: they act on the desktop — other windows,
/// workspaces — rather than on the plugin the user is focused on. The list is
/// deliberately explicit, so a core tool that is not on it keeps working: it
/// fails open, never locking the model out of something it needs.
///
/// Note what is NOT here: `web_search`, and the plugin lifecycle
/// (`plugin_activate` / `plugin_deactivate`). Refusing activation was a real bug:
/// with a plugin focused but not yet active, the model could not activate it and
/// the focused plugin stayed unusable.
const FOCUS_MODE_BLOCKED_CORE: &[&str] = &[
    "show_plugin",
    "desktop_focus",
    "desktop_fullscreen",
    "workspace_create",
    "workspace_remove",
    "workspace_switch",
    "workspace_move",
    "plugin_deactivate_all",
];

/// Whether focus mode blocks this core action. Public so the end-to-end test
/// (`tests/focus_mode_scope.rs`) can check the rule against the real registry.
pub fn focus_blocks_core(ctx: &AgentContext, action: &str) -> bool {
    !ctx.focused.is_empty() && FOCUS_MODE_BLOCKED_CORE.contains(&action)
}

/// Whether focus mode blocks a plugin tool. Only called once the registry has
/// confirmed it owns the action, so `owner` is a real plugin name.
pub fn focus_blocks_plugin(ctx: &AgentContext, owner: &str) -> bool {
    !ctx.focused.is_empty() && !ctx.focused.iter().any(|f| f == owner)
}

/// Refusal for a desktop action taken while a window is focused.
fn focus_core_refusal(ctx: &AgentContext, action: &str) -> ActionOutcome {
    let focused = ctx.focused.join(", ");
    ActionOutcome {
        action: action.into(),
        result: "unavailable".into(),
        data: json!({ "error": format!(
            "focus mode is on (focused: {focused}), so '{action}' is unavailable this turn — it \
             is a desktop action, not something the focused plugin does. Say so in one short \
             sentence if it matters; web search and the {focused} tools work normally."
        ), "focus_mode": true, "action_kind": "desktop" }),
        artifact: None,
        extra_artifacts: vec![],
        owner: None,
    }
}

/// Refusal for another plugin's tool. Only the USER changes the focus, so the
/// message says what to suggest instead.
fn focus_plugin_refusal(ctx: &AgentContext, owner: &str) -> ActionOutcome {
    let focused = ctx.focused.join(", ");
    ActionOutcome {
        action: owner.into(),
        result: "unavailable".into(),
        data: json!({ "error": format!(
            "focus mode is on and plugin '{owner}' is not focused (focused: {focused}), so its \
             tools are unavailable this turn — only web search and the {focused} tools run. Say \
             so in one short sentence and SUGGEST the user focuses {owner} — the eye button in \
             its window, or right-clicking its icon in the top bar — if that is what the request \
             needs; only they can change the focus."
        ), "focus_mode": true, "suggest_focus": owner }),
        artifact: None,
        extra_artifacts: vec![],
        owner: None,
    }
}

/// Refusal for a focus-only surface (a Craft app's MCP tools) used while that
/// app's window is not in focus mode. Hard rule: these tools exist only for the
/// focused app — the same rule the MCP host enforces by lifecycle.
fn focus_only_refusal(owner: &str) -> ActionOutcome {
    ActionOutcome {
        action: owner.into(),
        result: "unavailable".into(),
        data: json!({ "error": format!(
            "'{owner}' exposes its tools only while its window is in focus mode, and it is not \
             focused right now, so its tools are unavailable this turn. Say so in one short \
             sentence and SUGGEST the user focuses {owner} — the eye button in its window, or \
             right-clicking its icon in the top bar — if the request needs it; only they can \
             change the focus."
        ), "focus_mode": true, "suggest_focus": owner }),
        artifact: None,
        extra_artifacts: vec![],
        owner: None,
    }
}

/// Error outcome for a desktop-control tool whose plugin isn't installed or
/// isn't active for this user. Returns `None` when the plugin is usable.
async fn active_plugin_error(
    state: &AppState,
    traveler_id: &str,
    name: &str,
    action: &str,
) -> Option<ActionOutcome> {
    let installed = state.plugins.list().iter().any(|m| m.name == name);
    if !installed {
        return Some(ActionOutcome {
            action: action.into(),
            result: "error".into(),
            data: json!({ "error": format!("plugin '{name}' is not installed") }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        });
    }
    if !state.plugins.session_active_plugin_enabled(traveler_id, name).await {
        return Some(ActionOutcome {
            action: action.into(),
            result: "error".into(),
            data: json!({ "error": format!("plugin '{name}' is inactive — activate it with plugin_activate first") }),
            artifact: None,
            extra_artifacts: vec![],
            owner: None,
        });
    }
    None
}

pub async fn fetch_active_trip(pool: &SqlitePool, traveler_id: &str) -> Result<Option<Trip>, AppError> {
    Ok(sqlx::query_as::<_, Trip>(
        "SELECT * FROM trips WHERE traveler_id = ?1 AND status = 'active' LIMIT 1",
    )
    .bind(traveler_id)
    .fetch_optional(pool)
    .await?)
}

fn normalize_action_name(raw: &str) -> String {
    let a = raw.trim().trim_matches('"').to_lowercase();
    match a.as_str() {
        "navigate" | "start_navigation" | "start_navigator" | "navigation"
        | "directions" | "drive_to" | "navigate-to" | "go_to" => "navigate_to".into(),
        "activate_plugin" | "enable_plugin" => "plugin_activate".into(),
        "deactivate_plugin" | "disable_plugin" => "plugin_deactivate".into(),
        "plugins" | "list_available_plugins" | "available_plugins" => "list_plugins".into(),
        // Desktop control — the model phrases these loosely; normalize them so
        // "move the radio to a new workspace" never fails as an unknown action.
        "move_to_workspace" | "move_window" | "move_plugin"
        | "move_window_to_workspace" | "move_plugin_to_workspace" => "workspace_move".into(),
        "new_workspace" | "add_workspace" | "create_workspace" => "workspace_create".into(),
        "delete_workspace" | "close_workspace" | "remove_workspace" => "workspace_remove".into(),
        "switch_workspace" | "goto_workspace" | "go_to_workspace" => "workspace_switch".into(),
        "fullscreen" | "make_fullscreen" | "fullscreen_plugin"
        | "toggle_fullscreen" => "desktop_fullscreen".into(),
        // `focus_plugin` used to land here; focus mode is not something the model
        // can set (only SUGGEST), so neither action claims it.
        "focus_window" | "switch_focus" | "bring_to_front" => "desktop_focus".into(),
        "close_all_plugins" | "close_all" | "close_all_windows"
        | "deactivate_all" | "deactivate_all_plugins" => "plugin_deactivate_all".into(),
        "close_plugin" | "close_window" => "plugin_deactivate".into(),
        _ => a,
    }
}

pub fn parse_actions(text: &str) -> Vec<(String, Value)> {
    let normalized = text
        .replace("```json", "")
        .replace("```JSON", "")
        .replace("```", "");

    let mut actions = Vec::new();
    let mut search_from = 0;
    while let Some(start) = normalized[search_from..].find('{') {
        let abs_start = search_from + start;
        if let Some(end) = find_json_end(&normalized[abs_start..]) {
            let slice = &normalized[abs_start..abs_start + end + 1];
            if let Ok(v) = serde_json::from_str::<Value>(slice) {
                let action = v
                    .get("action")
                    .and_then(|a| a.as_str())
                    .or_else(|| v.get("tool").and_then(|a| a.as_str()));
                if let Some(action) = action {
                    let params = v
                        .get("params")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    actions.push((action.to_string(), params));
                }
            }
            search_from = abs_start + end + 1;
        } else {
            break;
        }
    }
    actions
}

fn find_json_end(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn strip_action_blocks(text: &str) -> String {
    let mut result = text.to_string();

    // Remove well-formed action blocks (compact + pretty JSON) exactly.
    for (action, params) in parse_actions(text) {
        let compact = json!({ "action": action, "params": params }).to_string();
        result = result.replace(&compact, "");

        let pretty = serde_json::to_string_pretty(&json!({ "action": action, "params": params }))
            .unwrap_or_default();
        result = result.replace(&pretty, "");
    }

    // The model sometimes emits MALFORMED JSON (a stray `\"` inside a cell
    // formula, an unquoted key, …) that serde_json rejects, so `parse_actions`
    // misses it and the raw tool call would leak into the visible reply. Strip
    // any remaining brace-balanced `{…}` block regardless of whether it parses.
    result = strip_balanced_braces(&result);

    result = result
        .replace("```json", "")
        .replace("```JSON", "")
        .replace("```", "");

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove every `{…}` region (nesting aware) from `s`, keeping everything else.
fn strip_balanced_braces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        if c == '{' {
            depth += 1;
            continue;
        }
        if c == '}' && depth > 0 {
            depth -= 1;
            continue;
        }
        if depth > 0 {
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        focus_blocks_core, focus_blocks_plugin, focus_core_refusal, focus_only_refusal,
        focus_plugin_refusal, AgentContext,
    };

    fn ctx(focused: &[&str]) -> AgentContext {
        AgentContext {
            lat: None,
            lon: None,
            heading: None,
            lang: "en".into(),
            ollama_model: None,
            workspaces_enabled: true,
            focused: focused.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// The focused plugin's own tools MUST run — that is the entire point of
    /// focus mode. Regression: an invented tool name resolved to "no owner",
    /// which the old gate read as a core action and refused, so the model was
    /// told focus mode blocked Word and it gave up instead of using `doc_create`.
    #[test]
    fn the_focused_plugins_tools_always_work() {
        let off = ctx(&[]);
        assert!(!focus_blocks_plugin(&off, "word"));
        assert!(!focus_blocks_core(&off, "show_plugin"));

        let on = ctx(&["word"]);
        assert!(!focus_blocks_plugin(&on, "word"), "the focused plugin is the point");
        assert!(focus_blocks_plugin(&on, "calculator"), "another plugin does not");

        // Lifecycle and web search are never blocked: a focused plugin that is
        // not active yet must still be activatable, or it stays unusable forever.
        assert!(!focus_blocks_core(&on, "web_search"));
        assert!(!focus_blocks_core(&on, "plugin_activate"));
        assert!(!focus_blocks_core(&on, "plugin_deactivate"));

        // Desktop actions are, while a window is focused.
        assert!(focus_blocks_core(&on, "show_plugin"));
        assert!(focus_blocks_core(&on, "desktop_focus"));
        assert!(focus_blocks_core(&on, "workspace_switch"));

        // A name nobody knows is NOT focus mode's business: it has to reach its
        // own "unknown action" error so the model corrects the name instead of
        // concluding that focus mode is locking it out.
        assert!(!focus_blocks_core(&on, "word_create_document"));
    }

    #[test]
    fn several_plugins_can_be_focused_and_the_refusal_suggests_one() {
        let many = ctx(&["files", "word"]);
        assert!(!focus_blocks_plugin(&many, "word"));
        assert!(!focus_blocks_plugin(&many, "files"));
        assert!(focus_blocks_plugin(&many, "calculator"));

        let refusal = focus_plugin_refusal(&many, "calculator");
        assert_eq!(refusal.result, "unavailable");
        let msg = refusal.data.get("error").and_then(|v| v.as_str()).unwrap_or_default();
        assert!(msg.contains("SUGGEST"), "{msg}");
        assert!(msg.contains("calculator"), "{msg}");
        assert!(msg.contains("files, word"), "names the current focus: {msg}");
        // It must never tell the model to change the focus itself.
        assert!(!msg.contains("plugin_focus"), "{msg}");

        let core = focus_core_refusal(&many, "show_plugin");
        let core_msg = core.data.get("error").and_then(|v| v.as_str()).unwrap_or_default();
        assert!(core_msg.contains("desktop action"), "{core_msg}");
        assert!(core_msg.contains("files, word"), "{core_msg}");
        assert!(core_msg.contains("web search"), "says what still works: {core_msg}");
    }

    /// A Craft app's MCP tools are a focus-only surface: without that app's
    /// window focused they must be refused with a suggestion, never silently
    /// dispatched.
    #[test]
    fn focus_only_tools_are_refused_without_their_app_focus() {
        let refusal = focus_only_refusal("photocraft");
        assert_eq!(refusal.result, "unavailable");
        assert_eq!(refusal.data["focus_mode"], true);
        assert_eq!(refusal.data["suggest_focus"], "photocraft");
        let msg = refusal.data["error"].as_str().unwrap_or_default();
        assert!(msg.contains("focus mode"), "{msg}");
        assert!(msg.contains("SUGGEST"), "{msg}");
        assert!(msg.contains("eye button"), "{msg}");
    }
}
