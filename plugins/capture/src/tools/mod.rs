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

use crate::editor::{export, project, routes as er, summary, transcribe, voice};
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
    "- `capture_style` — Set the project's visual language, kept by every later graphic: `guide` (palette, type, weights, radii, easing and durations, entrances and exits, density) and `css` (custom property declarations only, like `--accent: #…; --ease-out: cubic-bezier(…);`, without a selector). Do it after the first graphic the user approves, or when they ask for a new look. params: `{ project: string, guide?: string, css?: string }`",
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
        // `resolution` and `format` are the names models reach for first.
        let name = ["preset", "resolution", "format"].iter().find_map(|k| req.params.param_str(k)).unwrap_or_else(|| "source".into());
        let preset = export::Preset::named(&name.to_ascii_lowercase());
        let job = export::start(home(&req), p, preset);
        Ok(ActionOutcome::ok("capture_export", json!({ "job": job })))
    });

tool!(CaptureVoices, "capture_voices", "Checking the voices…",
    "- `capture_voices` — The text-to-speech engines on this computer (supertonic, qwen), whether each answers, and their voices. params: `{}`",
    |self, ctx, req| {
        let url = ctx.config.supertonic_url.clone();
        let v = tokio::task::spawn_blocking(move || voice::engines(&url))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(ActionOutcome::ok("capture_voices", v))
    });

tool!(CaptureVoice, "capture_voice", "Recording the voice…",
    "- `capture_voice` — Speak lines with a local voice and put them on the timeline: one narrator, or a dialogue / podcast with several speakers (one voice each). Returns every line's timeline time. params: `{ project: string, lines: [{ speaker?: string, text: string, pause?: seconds after (default 0.35) }] (or plain strings for one narrator), voices?: { speaker: voice }, voice?: one voice for every speaker without one, engine?: \"supertonic\"|\"qwen\", lang?: \"it\"|\"en\"|…, speed?: 0.7–2, at?: timeline seconds (default: end of the track), track?: \"a1\"|\"a2\", captions?: bool }`",
    |self, ctx, req| {
        let mut p = match load(&req) { Ok(p) => p, Err(e) => return err("capture_voice", e) };
        let prm = req.params;
        let Some(engine) = voice::Engine::parse(&prm.param_str("engine").unwrap_or_default()) else {
            return err("capture_voice", "engine must be \"supertonic\" or \"qwen\"");
        };
        // Models write the lines a few ways: objects, plain strings, or one
        // `text`; a top-level `voice` / `pause` applies to every line.
        let default_pause = prm.param_f64("pause").unwrap_or(0.35);
        let raw: Vec<Value> = match prm.get("lines") {
            Some(Value::Array(a)) => a.clone(),
            Some(Value::String(t)) => vec![Value::String(t.clone())],
            _ => prm.get("text").cloned().into_iter().collect(),
        };
        let lines: Vec<voice::Line> = raw.iter().filter_map(|l| {
            let (speaker, text, pause) = match l {
                Value::String(t) => (None, t.clone(), None),
                Value::Object(_) => (
                    l.get("speaker").and_then(|v| v.as_str()).map(str::to_string),
                    l.get("text").and_then(|v| v.as_str())?.to_string(),
                    l.get("pause").and_then(|v| v.as_f64()),
                ),
                _ => return None,
            };
            let text = text.trim().to_string();
            if text.is_empty() { return None; }
            Some(voice::Line {
                speaker: speaker.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "narrator".into()),
                text,
                pause: pause.unwrap_or(default_pause),
            })
        }).collect();
        if lines.is_empty() { return err("capture_voice", "missing \"lines\": [{ speaker?, text, pause? }] or [\"text\", …]"); }
        let mut voices: std::collections::HashMap<String, String> = prm.get("voices").and_then(|v| v.as_object()).map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default();
        if let Some(v) = prm.param_str("voice") {
            for l in &lines { voices.entry(l.speaker.clone()).or_insert_with(|| v.clone()); }
        }
        let track = prm.param_str("track").unwrap_or_else(|| "a1".into());
        if p.track(&track).is_none() { return err("capture_voice", format!("no track {track}")); }
        let at = prm.param_f64("at").unwrap_or_else(|| p.clips.iter().filter(|c| c.track == track).map(|c| c.end()).fold(0.0, f64::max)).max(0.0);
        let rq = voice::Request {
            engine,
            base_url: match engine { voice::Engine::Supertonic => voice::supertonic_url(&ctx.config.supertonic_url), voice::Engine::Qwen => voice::qwen_url() },
            lang: prm.param_str("lang").unwrap_or_else(|| "en".into()),
            speed: prm.param_f64("speed").unwrap_or(1.0) as f32,
            lines,
            voices,
        };
        let h = home(&req);
        let dir = library::capture_dir(&h).join("Voices");
        let stem = format!("{}-{}", engine.name(), project::now_ms());
        let rendered = tokio::task::spawn_blocking(move || voice::render(&rq, &dir, &stem).map_err(|e| e.to_string()))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let (path, timed) = match rendered { Ok(r) => r, Err(e) => return err("capture_voice", e) };
        let media = match er::import(&h, &mut p, &[path.display().to_string()], false) {
            Ok(ids) if !ids.is_empty() => ids[0].clone(),
            Ok(_) => return err("capture_voice", "the voice file could not be added"),
            Err(e) => return err("capture_voice", e),
        };
        let mut ops = vec![json!({ "op": "add_clip", "media": media, "track": track, "at": at })];
        let several = timed.iter().map(|t| &t.speaker).collect::<std::collections::HashSet<_>>().len() > 1;
        if prm.param_bool("captions").unwrap_or(false) {
            for t in &timed {
                let text = if several { format!("{}: {}", t.speaker, t.text) } else { t.text.clone() };
                ops.push(json!({ "op": "add_caption", "at": at + t.at, "dur": t.dur.max(0.6), "text": text }));
            }
        }
        let created = match project::apply_all(&mut p, &ops) { Ok(i) => i, Err(e) => return err("capture_voice", e) };
        if let Err(e) = save(&req, &mut p) { return err("capture_voice", e); }
        let lines: Vec<Value> = timed.iter().map(|t| json!({ "speaker": t.speaker, "voice": t.voice, "text": t.text, "at": ((at + t.at) * 100.0).round() / 100.0, "dur": (t.dur * 100.0).round() / 100.0 })).collect();
        let end = timed.last().map(|t| at + t.at + t.dur).unwrap_or(at);
        Ok(ActionOutcome::ok("capture_voice", json!({
            "project": p.id, "engine": engine.name(), "media": media, "clip": created.first(), "track": track,
            "at": at, "end": (end * 100.0).round() / 100.0, "lines": lines, "revision": p.revision,
        })))
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
        std::sync::Arc::new(CaptureVoices),
        std::sync::Arc::new(CaptureVoice),
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
