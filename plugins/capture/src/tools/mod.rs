//! Agent tools. Recording and going live are only *requested* by the agent:
//! the window (or the top-bar chip) asks the user to confirm, because
//! nothing should start filming a screen on its own. Editing tools work on
//! the project file directly; proposals wait for the user.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::editor::{export, project, routes as er, summary, transcribe};
use crate::{library, routes, session, store};

fn home(req: &ToolRequest<'_>) -> PathBuf {
    req.os_home
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn err(action: &str, e: impl std::fmt::Display) -> Result<ActionOutcome, AppError> {
    Ok(ActionOutcome::error(action, e.to_string()))
}

fn load(req: &ToolRequest<'_>) -> Result<project::Project, String> {
    let id = req
        .params
        .param_str("project")
        .ok_or("missing \"project\" (a project id)")?;
    project::load(&home(req), &id).ok_or_else(|| format!("no project {id}"))
}

fn save(req: &ToolRequest<'_>, p: &mut project::Project) -> Result<(), String> {
    project::save(&home(req), p)
}

macro_rules! tool {
    ($ty:ident, $name:literal, $label:literal, $doc:literal, |$self_:ident, $ctx:ident, $req:ident| $body:block) => {
        pub struct $ty;
        #[async_trait]
        impl Tool for $ty {
            fn name(&self) -> &str {
                $name
            }
            fn step_label(&self) -> &str {
                $label
            }
            fn doc_fragment(&self) -> Option<&str> {
                Some($doc)
            }
            #[allow(unused_variables)]
            async fn invoke(&$self_, $ctx: &PluginCtx, $req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> $body
        }
    };
}

tool!(CaptureStatus, "capture_status", "Checking the recording…",
    "- `capture_status` — Is a recording or live stream running, for how long, any problem. params: `{}`",
    |self, ctx, req| {
        Ok(ActionOutcome::ok("capture_status", routes::status_json()))
    });

/// Shared by record and live: the window turns this into a confirmation.
fn start_request(action: &str, req: &ToolRequest<'_>, live: Vec<Value>) -> ActionOutcome {
    if session::is_running() {
        return ActionOutcome::error(action, "a recording or live stream is already running");
    }
    let p = req.params;
    let request = json!({
        "source": p.param_str("source").unwrap_or_else(|| "screen".into()),
        "window": p.param_str("window"),
        "webcam": p.param_bool("webcam"),
        "mic": p.param_bool("mic"),
        "desktop": p.param_bool("desktop_audio"),
        "record": p.param_bool("record").unwrap_or(action == "capture_record_start"),
        "live": live,
    });
    ActionOutcome::ok(
        action,
        json!({
            "pending": true,
            "request": request,
            "note": "The user must confirm in the Capture window before anything is recorded. Tell them so in one short sentence.",
        }),
    )
}

tool!(CaptureRecordStart, "capture_record_start", "Getting the recording ready…",
    "- `capture_record_start` — Ask to record the screen (the user confirms on screen). params: `{ source?: \"screen\"|\"window\"|\"region\", window?: string (window title or app name), webcam?: bool, mic?: bool, desktop_audio?: bool }`",
    |self, ctx, req| {
        Ok(start_request("capture_record_start", &req, Vec::new()))
    });

tool!(CaptureLiveStart, "capture_live_start", "Getting the live stream ready…",
    "- `capture_live_start` — Ask to go live (the user confirms on screen). params: `{ destinations: [string] (names or platforms: youtube, twitch, kick, facebook), record?: bool, source?, window?, webcam?, mic?, desktop_audio? }`",
    |self, ctx, req| {
        let want: Vec<String> = req.params.get("destinations").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_lowercase())).collect())
            .unwrap_or_default();
        let all = match store::list_destinations(ctx, req.user_id) { Ok(d) => d, Err(e) => return err("capture_live_start", e) };
        if all.is_empty() {
            return err("capture_live_start", "no live destinations yet: the user adds them (with the stream key) in the Capture window");
        }
        let chosen: Vec<Value> = all.iter()
            .filter(|d| d.enabled && (want.is_empty() || want.iter().any(|w| d.name.to_lowercase().contains(w) || d.platform == *w)))
            .map(|d| json!({ "id": d.id, "name": d.name, "platform": d.platform }))
            .collect();
        if chosen.is_empty() {
            return err("capture_live_start", format!("none of the destinations matches; they are: {}", all.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ")));
        }
        Ok(start_request("capture_live_start", &req, chosen))
    });

tool!(
    CaptureStop,
    "capture_stop",
    "Stopping…",
    "- `capture_stop` — Stop the recording or live stream (the file is saved). params: `{}`",
    |self, ctx, req| {
        if !session::is_running() {
            return err("capture_stop", "nothing is recording");
        }
        session::stop();
        Ok(ActionOutcome::ok(
            "capture_stop",
            json!({ "stopping": true }),
        ))
    }
);

tool!(CaptureMarker, "capture_marker", "Adding a marker…",
    "- `capture_marker` — Mark this moment of the recording (a chapter for the edit). params: `{ label?: string }`",
    |self, ctx, req| {
        let label = req.params.param_str("label").unwrap_or_default();
        if routes::add_marker(label.clone()) {
            Ok(ActionOutcome::ok("capture_marker", json!({ "label": label })))
        } else {
            err("capture_marker", "nothing is recording")
        }
    });

tool!(CaptureRecordings, "capture_recordings", "Looking at the recordings…",
    "- `capture_recordings` — Recordings in the Videos folder, newest first (id = file name). params: `{ limit?: number }`",
    |self, ctx, req| {
        let n = req.params.param_u32("limit").unwrap_or(10).clamp(1, 50) as usize;
        let list: Vec<Value> = library::list(&library::capture_dir(&home(&req))).into_iter().take(n).map(|r| json!({
            "id": r.id, "bytes": r.bytes, "modified": r.modified,
            "duration_s": r.meta.get("duration_us").and_then(|d| d.as_f64()).map(|d| (d / 1e5).round() / 10.0),
            "window": r.meta.get("window"), "markers": r.meta.get("markers").and_then(|m| m.as_array()).map(|m| m.len()),
        })).collect();
        Ok(ActionOutcome::ok("capture_recordings", json!({ "recordings": list })))
    });

tool!(
    CaptureProjects,
    "capture_projects",
    "Looking at the video projects…",
    "- `capture_projects` — Video projects (id, title, duration). params: `{}`",
    |self, ctx, req| {
        Ok(ActionOutcome::ok(
            "capture_projects",
            json!({ "projects": project::list(&home(&req)) }),
        ))
    }
);

tool!(CaptureProjectCreate, "capture_project_create", "Creating the video project…",
    "- `capture_project_create` — New video project, optionally with media placed one after another. params: `{ title?: string, media?: [string] (recording ids or file paths) }`",
    |self, ctx, req| {
        let h = home(&req);
        let mut p = match project::create(&h, req.params.param_str("title").as_deref()) { Ok(p) => p, Err(e) => return err("capture_project_create", e) };
        let files: Vec<String> = req.params.get("media").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        if !files.is_empty() {
            if let Err(e) = er::import(&h, &mut p, &files, true) { return err("capture_project_create", e) }
            if let Err(e) = project::save(&h, &mut p) { return err("capture_project_create", e) }
        }
        for m in &p.media {
            transcribe::start(m.path.clone(), voice_track(m), None);
        }
        Ok(ActionOutcome::ok("capture_project_create", json!({ "project": p.id, "open": true, "summary": summary::summary(&p, false) })))
    });

/// The stream that carries the voice: the mic of a recording, else the first.
fn voice_track(m: &project::Media) -> u32 {
    m.tracks.iter().position(|t| t == "mic").unwrap_or(0) as u32
}

tool!(CaptureProjectGet, "capture_project_get", "Reading the project…",
    "- `capture_project_get` — The project as you can work on it: timeline (clips with ids, seconds), each media's transcript in segments with ids, fillers and pauses, the recording's events and markers, scenes, the style, open proposals. params: `{ project: string }`",
    |self, ctx, req| {
        match load(&req) {
            Ok(p) => {
                let pending: Vec<Value> = p.media.iter().filter_map(|m| transcribe::state(&m.path).filter(|s| s.state == "running").map(|s| json!([m.id, s.progress]))).collect();
                let mut v = summary::summary(&p, false);
                v["transcribing"] = json!(pending);
                Ok(ActionOutcome::ok("capture_project_get", v))
            }
            Err(e) => err("capture_project_get", e),
        }
    });

tool!(CaptureImport, "capture_import", "Adding media…",
    "- `capture_import` — Add files to a project (video, audio, images), placed at the end unless `place` is false. params: `{ project: string, files: [string], place?: bool }`",
    |self, ctx, req| {
        let mut p = match load(&req) { Ok(p) => p, Err(e) => return err("capture_import", e) };
        let files: Vec<String> = req.params.get("files").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        let ids = match er::import(&home(&req), &mut p, &files, req.params.param_bool("place").unwrap_or(true)) { Ok(i) => i, Err(e) => return err("capture_import", e) };
        if let Err(e) = save(&req, &mut p) { return err("capture_import", e) }
        for m in p.media.iter().filter(|m| ids.contains(&m.id)) {
            transcribe::start(m.path.clone(), voice_track(m), None);
        }
        Ok(ActionOutcome::ok("capture_import", json!({ "project": p.id, "media": ids })))
    });

tool!(CaptureTranscribe, "capture_transcribe", "Transcribing…",
    "- `capture_transcribe` — Transcribe the project's media (word timings; runs in the background, then read it with capture_project_get). params: `{ project: string, lang?: string }`",
    |self, ctx, req| {
        let p = match load(&req) { Ok(p) => p, Err(e) => return err("capture_transcribe", e) };
        let lang = req.params.param_str("lang");
        let mut started = Vec::new();
        for m in &p.media {
            if m.info.audio_tracks > 0 {
                transcribe::start(m.path.clone(), voice_track(m), lang.clone());
                started.push(json!({ "media": m.id, "state": transcribe::state(&m.path) }));
            }
        }
        Ok(ActionOutcome::ok("capture_transcribe", json!({ "media": started })))
    });

const OPS_DOC: &str = "ops: add_clip{media,track?,at?,in?,out?,speed?,volume?,rect?{x,y,w,h 0..1},radius?}, place_media{media,at?}, move_clip{clip,at?,track?}, trim_clip{clip,in?,out?,at?}, set_clip{clip,speed?,volume?,fade_in?,fade_out?,opacity?,rect?,radius?}, split_clip{clip,t}, delete_clip{clip}, cut_ranges{ranges:[[from,to]] timeline seconds, closes gaps}, add_overlay{html,at,dur,rect?,name?}, update_overlay{id,…}, delete_overlay{id}, add_caption{at,dur,text}, set_captions{captions:[{at,dur,text}]}, add_marker{t,label}, set_style{guide?,css?}, set_track{track,volume?,muted?,hidden?}";

pub struct CaptureEdit;
#[async_trait]
impl Tool for CaptureEdit {
    fn name(&self) -> &str {
        "capture_edit"
    }
    fn step_label(&self) -> &str {
        "Editing the video…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `capture_edit` — Apply edits now (when the user asked for that exact change, or chose \"edit everything\"). All or nothing; returns new ids. params: `{ project: string, ops: [op] }` — see capture_propose for the op list")
    }
    async fn invoke(
        &self,
        _ctx: &PluginCtx,
        req: ToolRequest<'_>,
    ) -> Result<ActionOutcome, AppError> {
        let mut p = match load(&req) {
            Ok(p) => p,
            Err(e) => return err("capture_edit", e),
        };
        let ops: Vec<Value> = req
            .params
            .get("ops")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        if ops.is_empty() {
            return err("capture_edit", "missing \"ops\"");
        }
        let ids = match project::apply_all(&mut p, &ops) {
            Ok(i) => i,
            Err(e) => return err("capture_edit", e),
        };
        if let Err(e) = save(&req, &mut p) {
            return err("capture_edit", e);
        }
        Ok(ActionOutcome::ok(
            "capture_edit",
            json!({ "project": p.id, "created": ids, "duration": p.duration(), "revision": p.revision }),
        ))
    }
}

pub struct CapturePropose;
#[async_trait]
impl Tool for CapturePropose {
    fn name(&self) -> &str {
        "capture_propose"
    }
    fn step_label(&self) -> &str {
        "Preparing a proposal…"
    }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `capture_propose` — Propose edits the user accepts, rejects or refines on the timeline (new clips and graphics show as ghosts). params: `{ project: string, summary: string (one sentence, the user's language), ops: [op] }`")
    }
    async fn invoke(
        &self,
        _ctx: &PluginCtx,
        req: ToolRequest<'_>,
    ) -> Result<ActionOutcome, AppError> {
        let mut p = match load(&req) {
            Ok(p) => p,
            Err(e) => return err("capture_propose", e),
        };
        let mut ops: Vec<Value> = req
            .params
            .get("ops")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        if ops.is_empty() {
            return err("capture_propose", format!("missing \"ops\". {OPS_DOC}"));
        }
        // Validate on a copy first: a proposal that cannot apply is refused now.
        let mut check = p.clone();
        if let Err(e) = project::apply_all(&mut check, &ops) {
            return err("capture_propose", e);
        }
        // Ghost previews for what adds something.
        for op in ops.iter_mut() {
            let kind = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            if matches!(kind, "add_clip" | "add_overlay") {
                let mut g = op.clone();
                g["ghost"] = json!(true);
                if let Ok(ids) = project::apply(&mut p, &g) {
                    if let Some(id) = ids.first() {
                        op["_ghost_id"] = json!(id);
                    }
                }
            }
        }
        let id = format!("p{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        p.proposals.push(project::Proposal {
            id: id.clone(),
            summary: req
                .params
                .param_str("summary")
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect(),
            ops,
            status: "pending".into(),
            created: project::now_ms(),
            feedback: String::new(),
        });
        if let Err(e) = save(&req, &mut p) {
            return err("capture_propose", e);
        }
        Ok(ActionOutcome::ok(
            "capture_propose",
            json!({ "project": p.id, "proposal": id }),
        ))
    }
}

tool!(CaptureProposal, "capture_proposal", "Updating the proposal…",
    "- `capture_proposal` — Accept or drop a proposal (only when the user says so). params: `{ project: string, proposal: string, accept: bool }`",
    |self, ctx, req| {
        let mut p = match load(&req) { Ok(p) => p, Err(e) => return err("capture_proposal", e) };
        let Some(pid) = req.params.param_str("proposal") else { return err("capture_proposal", "missing \"proposal\"") };
        let r = if req.params.param_bool("accept").unwrap_or(false) {
            er::accept(&mut p, &pid).map(|_| ()).map_err(|e| e.to_string())
        } else {
            er::reject(&mut p, &pid);
            Ok(())
        };
        if let Err(e) = r { return err("capture_proposal", e) }
        if let Err(e) = save(&req, &mut p) { return err("capture_proposal", e) }
        Ok(ActionOutcome::ok("capture_proposal", json!({ "project": p.id, "duration": p.duration() })))
    });

tool!(CaptureGraphic, "capture_graphic", "Designing the graphic…",
    "- `capture_graphic` — Create or replace an HTML graphic (title, lower third, animated explainer, slide, full-screen scene) shown over the video. Write a complete self-contained document: transparent background, no network (inline CSS/JS/SVG, data: images), CSS animations or JS driven by `capture.time` (seconds since the graphic starts); use the style's CSS variables. params: `{ project: string, html: string, at: number, dur: number, rect?: {x,y,w,h} (0..1 of the canvas, default full), name?: string, id?: string (to replace), propose?: bool }`",
    |self, ctx, req| {
        let mut p = match load(&req) { Ok(p) => p, Err(e) => return err("capture_graphic", e) };
        let mut op = req.params.clone();
        let kind = if req.params.param_str("id").is_some() { "update_overlay" } else { "add_overlay" };
        op["op"] = json!(kind);
        if let Some(o) = op.as_object_mut() { o.remove("project"); o.remove("propose"); }
        if req.params.param_bool("propose").unwrap_or(false) {
            let mut g = op.clone();
            g["ghost"] = json!(true);
            let ids = match project::apply(&mut p, &g) { Ok(i) => i, Err(e) => return err("capture_graphic", e) };
            if let Some(id) = ids.first() { op["_ghost_id"] = json!(id); }
            let pid = format!("p{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
            p.proposals.push(project::Proposal { id: pid.clone(), summary: req.params.param_str("name").unwrap_or_else(|| "Graphic".into()), ops: vec![op], status: "pending".into(), created: project::now_ms(), feedback: String::new() });
            if let Err(e) = save(&req, &mut p) { return err("capture_graphic", e) }
            return Ok(ActionOutcome::ok("capture_graphic", json!({ "project": p.id, "proposal": pid, "graphic": ids.first() })));
        }
        let ids = match project::apply(&mut p, &op) { Ok(i) => i, Err(e) => return err("capture_graphic", e) };
        if let Err(e) = save(&req, &mut p) { return err("capture_graphic", e) }
        Ok(ActionOutcome::ok("capture_graphic", json!({ "project": p.id, "graphic": ids.first() })))
    });

tool!(CaptureStyle, "capture_style", "Writing the style…",
    "- `capture_style` — Set the project's visual language, kept by every later graphic: `guide` (palette, type, weights, radii, easing and durations, entrances and exits, density) and `css` (custom properties like `--accent: #…;`). Do it after the first graphic the user approves, or when they ask for a new look. params: `{ project: string, guide?: string, css?: string }`",
    |self, ctx, req| {
        let mut p = match load(&req) { Ok(p) => p, Err(e) => return err("capture_style", e) };
        let mut op = json!({ "op": "set_style" });
        if let Some(g) = req.params.param_str("guide") { op["guide"] = json!(g); }
        if let Some(c) = req.params.param_str("css") { op["css"] = json!(c); }
        if let Err(e) = project::apply(&mut p, &op) { return err("capture_style", e) }
        if let Err(e) = save(&req, &mut p) { return err("capture_style", e) }
        Ok(ActionOutcome::ok("capture_style", json!({ "project": p.id, "style": p.style })))
    });

tool!(CaptureUndo, "capture_undo", "Going back…",
    "- `capture_undo` — Restore the project as it was before the last change (or a given version). params: `{ project: string, version?: number }`",
    |self, ctx, req| {
        let h = home(&req);
        let p = match load(&req) { Ok(p) => p, Err(e) => return err("capture_undo", e) };
        let versions = project::versions(&h, &p.id);
        let given = req.params.param_f64("version").map(|v| v as u64);
        let Some(ts) = given.or_else(|| versions.first().copied()) else { return err("capture_undo", "no earlier version") };
        let Some(mut old) = project::load_version(&h, &p.id, ts) else { return err("capture_undo", "version not found") };
        old.id = p.id.clone();
        old.revision = p.revision;
        // A plain undo steps back through the history; restoring a chosen
        // version keeps the current state as a version, so nothing is lost.
        let saved = if given.is_some() { project::save(&h, &mut old) } else { project::save_undone(&h, &mut old, ts) };
        if let Err(e) = saved { return err("capture_undo", e) }
        Ok(ActionOutcome::ok("capture_undo", json!({ "project": old.id, "restored": ts })))
    });

tool!(CaptureExport, "capture_export", "Exporting the video…",
    "- `capture_export` — Render the project to a file in Videos/LiteAura/Exports (runs in the background). params: `{ project: string, preset?: \"source\"|\"1080p\"|\"720p\"|\"4k\"|\"vertical\"|\"square\"|\"gif\" }`",
    |self, ctx, req| {
        let p = match load(&req) { Ok(p) => p, Err(e) => return err("capture_export", e) };
        if p.duration() <= 0.0 { return err("capture_export", "the timeline is empty"); }
        let preset = export::Preset::named(&req.params.param_str("preset").unwrap_or_else(|| "source".into()));
        let job = export::start(home(&req), p, preset);
        Ok(ActionOutcome::ok("capture_export", json!({ "job": job })))
    });

pub fn all() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(CaptureStatus),
        std::sync::Arc::new(CaptureRecordStart),
        std::sync::Arc::new(CaptureLiveStart),
        std::sync::Arc::new(CaptureStop),
        std::sync::Arc::new(CaptureMarker),
        std::sync::Arc::new(CaptureRecordings),
        std::sync::Arc::new(CaptureProjects),
        std::sync::Arc::new(CaptureProjectCreate),
        std::sync::Arc::new(CaptureProjectGet),
        std::sync::Arc::new(CaptureImport),
        std::sync::Arc::new(CaptureTranscribe),
        std::sync::Arc::new(CaptureEdit),
        std::sync::Arc::new(CapturePropose),
        std::sync::Arc::new(CaptureProposal),
        std::sync::Arc::new(CaptureGraphic),
        std::sync::Arc::new(CaptureStyle),
        std::sync::Arc::new(CaptureUndo),
        std::sync::Arc::new(CaptureExport),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn ops_doc_lists_every_op() {
        for op in [
            "add_clip",
            "cut_ranges",
            "add_overlay",
            "set_style",
            "split_clip",
            "set_captions",
        ] {
            assert!(super::OPS_DOC.contains(op), "{op}");
        }
    }
}
