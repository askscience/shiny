//! Capture REST routes (`/api/capture/*`).
//!
//! Starting a recording, a live stream or the camera is refused for remote
//! requests (Iroh, Tailscale): only someone at the machine records it. A
//! remote client may still watch the status and play recordings.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{
    bridged_route, is_remote_request, ok, os_home_from_request, path_params_from_request, user_id,
    RouteHandler, RouteSpec,
};
use shiny_plugin_sdk::services::PluginCtx;

use crate::bubble::Style;
use crate::session::{self, CameraPlan, Plan, SourceKind};
use crate::{camera, devices, editor, library, meter, platforms, store, Rect};

const MAX_BODY: usize = 4 * 1024 * 1024;

pub fn specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/capture/devices", "devices"),
        RouteSpec::get("/api/capture/settings", "settings_get"),
        RouteSpec::put("/api/capture/settings", "settings_put"),
        RouteSpec::get("/api/capture/status", "status"),
        RouteSpec::post("/api/capture/start", "start"),
        RouteSpec::post("/api/capture/stop", "stop"),
        RouteSpec::post("/api/capture/pause", "pause"),
        RouteSpec::post("/api/capture/update", "update"),
        RouteSpec::post("/api/capture/marker", "marker"),
        RouteSpec::post("/api/capture/event", "event"),
        RouteSpec::get("/api/capture/preview.jpg", "preview"),
        RouteSpec::get("/api/capture/camera.mjpeg", "camera_mjpeg"),
        RouteSpec::get("/api/capture/camera.jpg", "camera_jpg"),
        RouteSpec::post("/api/capture/camera/release", "camera_release"),
        RouteSpec::get("/api/capture/meter", "meter"),
        RouteSpec::get("/api/capture/platforms", "platforms"),
        RouteSpec::get("/api/capture/destinations", "dest_list"),
        RouteSpec::post("/api/capture/destinations", "dest_create"),
        RouteSpec::put("/api/capture/destinations/:id", "dest_update"),
        RouteSpec::delete("/api/capture/destinations/:id", "dest_delete"),
        RouteSpec::get("/api/capture/recordings", "rec_list"),
        RouteSpec::delete("/api/capture/recordings/:id", "rec_delete"),
        RouteSpec::get("/api/capture/media", "media"),
        RouteSpec::get("/api/capture/media/info", "media_info"),
        RouteSpec::get("/api/capture/media/peaks", "media_peaks"),
        RouteSpec::get("/api/capture/media/thumbs", "media_thumbs"),
        RouteSpec::get("/api/capture/projects", "proj_list"),
        RouteSpec::post("/api/capture/projects", "proj_create"),
        RouteSpec::get("/api/capture/projects/:id", "proj_get"),
        RouteSpec::put("/api/capture/projects/:id", "proj_put"),
        RouteSpec::delete("/api/capture/projects/:id", "proj_delete"),
        RouteSpec::post("/api/capture/projects/:id/export", "proj_export"),
        RouteSpec::get("/api/capture/projects/:id/summary", "proj_summary"),
        RouteSpec::post("/api/capture/projects/:id/transcribe", "proj_transcribe"),
        RouteSpec::get("/api/capture/jobs/:id", "job_get"),
        RouteSpec::get("/api/capture/overlay/:project/:id", "overlay_html"),
    ]
}

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "devices" => devices_route(),
        "settings_get" => settings_get(ctx),
        "settings_put" => settings_put(ctx),
        "status" => status(),
        "start" => start(ctx),
        "stop" => simple(|_| Ok(json!({ "stopping": session::stop() }))),
        "pause" => pause(),
        "update" => update(),
        "marker" => marker(),
        "event" => event(),
        "preview" => preview(),
        "camera_mjpeg" => camera_mjpeg(),
        "camera_jpg" => camera_jpg(),
        "camera_release" => simple(|_| {
            if !session::is_running() {
                camera::release();
            }
            Ok(json!({ "released": true }))
        }),
        "meter" => meter_route(),
        "platforms" => simple(|_| Ok(json!(platforms::PLATFORMS))),
        "dest_list" => dest_list(ctx),
        "dest_create" => dest_save(ctx, false),
        "dest_update" => dest_save(ctx, true),
        "dest_delete" => dest_delete(ctx),
        "rec_list" => rec_list(),
        "rec_delete" => rec_delete(),
        "media" => media(),
        "media_info" => media_info(),
        "media_peaks" => media_peaks(),
        "media_thumbs" => media_thumbs(),
        "proj_list" => editor::routes::list(),
        "proj_create" => editor::routes::create(),
        "proj_get" => editor::routes::get(),
        "proj_put" => editor::routes::put(),
        "proj_delete" => editor::routes::delete(),
        "proj_export" => editor::routes::export(),
        "job_get" => editor::routes::job(),
        "proj_summary" => editor::routes::summary(),
        "proj_transcribe" => editor::routes::transcribe(),
        "overlay_html" => editor::routes::overlay_html(),
        _ => return None,
    })
}

/* ── helpers ─────────────────────────────────────────────────── */

pub async fn take_json<T: DeserializeOwned>(req: Request) -> Result<T, AppError> {
    let bytes = axum::body::to_bytes(req.into_body(), MAX_BODY)
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
    if bytes.is_empty() {
        return serde_json::from_slice(b"{}")
            .map_err(|e| AppError::BadRequest(format!("invalid json: {e}")));
    }
    serde_json::from_slice(&bytes).map_err(|e| AppError::BadRequest(format!("invalid json: {e}")))
}

pub fn query<T: DeserializeOwned + Default>(req: &Request) -> Result<T, AppError> {
    match req.uri().query() {
        None | Some("") => Ok(T::default()),
        Some(_) => axum::extract::Query::<T>::try_from_uri(req.uri())
            .map(|q| q.0)
            .map_err(|e| AppError::BadRequest(format!("invalid query: {e}"))),
    }
}

pub fn param(req: &Request, name: &str) -> Result<String, AppError> {
    path_params_from_request(req)
        .and_then(|p| p.into_iter().find(|(k, _)| k == name).map(|(_, v)| v))
        .ok_or_else(|| AppError::BadRequest(format!("missing {name}")))
}

/// The caller's home: the real OS home when core bound them to an account.
pub fn home(req: &Request) -> PathBuf {
    os_home_from_request(req)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn local_only(req: &Request) -> Result<(), AppError> {
    if is_remote_request(req) {
        return Err(AppError::BadRequest(
            "recording and the camera can only be started on the machine itself".into(),
        ));
    }
    Ok(())
}

fn simple(f: fn(&Request) -> Result<Value, AppError>) -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        Ok(ok(f(&req)?))
    })
}

/// A streamed body produced on its own thread.
///
/// A response body is polled by the *host's* runtime, after this plugin's
/// handler returned: nothing inside it may touch this plugin's Tokio (a
/// timer or `spawn_blocking` there panics with "no reactor" and aborts the
/// whole server). So the bytes come from a plain thread through a
/// runtime-free channel; the thread ends when the client goes away.
fn thread_body<F>(produce: F) -> Body
where
    F: FnOnce(&mut dyn FnMut(Vec<u8>) -> bool) + Send + 'static,
{
    use futures::SinkExt;
    let (tx, rx) = futures::channel::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(2);
    std::thread::Builder::new()
        .name("capture-stream".into())
        .spawn(move || {
            let mut tx = tx;
            let mut send = |chunk: Vec<u8>| {
                futures::executor::block_on(tx.send(Ok(axum::body::Bytes::from(chunk)))).is_ok()
            };
            produce(&mut send);
        })
        .ok();
    Body::from_stream(rx)
}

fn bytes_response(ct: &'static str, body: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ct)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/* ── devices, settings, status ───────────────────────────────── */

fn devices_route() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let (audio, cams, va) = tokio::task::spawn_blocking(|| {
            (
                devices::audio_sources(),
                crate::v4l2::devices(),
                session::va_nodes().to_vec(),
            )
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
        let encoders: Vec<Value> = va
            .iter()
            .map(|n| json!({ "device": n.path, "intel": n.intel, "cbr": n.cbr }))
            .collect();
        Ok(ok(json!({
            "audio": audio,
            "cameras": cams,
            "encoders": encoders,
            "record_encoder": encoder_label(crate::encoder::Purpose::Record),
            "live_encoder": encoder_label(crate::encoder::Purpose::Live),
            "folder": library::capture_dir(&home(&req)),
        })))
    })
}

fn encoder_label(p: crate::encoder::Purpose) -> String {
    crate::encoder::choose(session::va_nodes(), p).friendly()
}

fn settings_get(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            Ok(ok(store::get_settings(&ctx, &uid)?))
        }
    })
}

fn settings_put(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let body: Value = take_json(req).await?;
            store::put_settings(&ctx, &uid, &body)?;
            Ok(ok(body))
        }
    })
}

pub fn status_json() -> Value {
    match session::current() {
        Some(s) => {
            let st = s.status.lock().unwrap().clone();
            let paused = s.ctl.lock().unwrap().paused;
            let mut v = serde_json::to_value(&st).unwrap_or(Value::Null);
            v["active"] = json!(session::is_running());
            v["paused"] = json!(paused);
            v["output_id"] = json!(st
                .output
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string()));
            v
        }
        None => json!({ "active": false, "state": "idle" }),
    }
}

fn status() -> RouteHandler {
    simple(|_| Ok(status_json()))
}

/* ── start / control ─────────────────────────────────────────── */

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CameraBody {
    pub device: Option<String>,
    pub style: Style,
    pub rect: Option<Rect>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct StartBody {
    pub source: SourceKind,
    pub rect: Option<Rect>,
    pub window_title: Option<String>,
    pub fps: Option<u32>,
    pub half: Option<bool>,
    pub cursor: Option<bool>,
    /// A source name, `"default"`, or absent for none.
    pub mic: Option<String>,
    pub desktop: Option<String>,
    pub camera: Option<CameraBody>,
    pub record: Option<bool>,
    /// Destination ids for a live stream.
    pub live: Vec<String>,
    pub kbps: Option<u32>,
    pub quality: Option<u32>,
    pub encoder: Option<String>,
}

fn camera_plan(c: CameraBody) -> Option<CameraPlan> {
    let rect = c.rect?;
    Some(CameraPlan {
        device: c.device.filter(|d| !d.is_empty()).map(PathBuf::from),
        style: c.style,
        rect,
    })
}

/// Build a plan from the window's (or the agent's) request.
pub fn plan_from(ctx: &PluginCtx, uid: &str, home: &Path, b: StartBody) -> Result<Plan, AppError> {
    let record = b.record.unwrap_or(b.live.is_empty());
    let (urls, labels, plats) = if b.live.is_empty() {
        (Vec::new(), Vec::new(), Vec::new())
    } else {
        store::ingest_for(ctx, uid, &b.live)?
    };
    let wanted = b
        .kbps
        .unwrap_or_else(|| plats.iter().map(|p| p.kbps_1080).min().unwrap_or(6000));
    let kbps = platforms::shared_kbps(wanted, &plats);
    if !record && urls.is_empty() {
        return Err(AppError::BadRequest(
            "choose recording, a live destination, or both".into(),
        ));
    }
    Ok(Plan {
        source: b.source,
        rect: b.rect,
        window_title: b.window_title.map(|t| t.chars().take(200).collect()),
        fps: b.fps.unwrap_or(30).clamp(10, 60),
        half: b.half,
        cursor: b.cursor.unwrap_or(true),
        mic: b
            .mic
            .as_deref()
            .and_then(|m| devices::resolve_source(Some(m), false)),
        desktop: b
            .desktop
            .as_deref()
            .and_then(|m| devices::resolve_source(Some(m), true)),
        camera: b.camera.and_then(camera_plan),
        record_dir: record.then(|| library::capture_dir(home)),
        live: urls,
        live_labels: labels,
        live_kbps: kbps,
        quality: b.quality.unwrap_or(20),
        encoder: b.encoder.unwrap_or_else(|| "auto".into()),
    })
}

fn start(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            local_only(&req)?;
            let h = home(&req);
            let body: StartBody = take_json(req).await?;
            let plan = plan_from(&ctx, &uid, &h, body)?;
            tokio::task::spawn_blocking(move || session::start(plan, Box::new(|_, _| {})))
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
            Ok(ok(status_json()))
        }
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PauseBody {
    paused: Option<bool>,
}

fn pause() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let b: PauseBody = take_json(req).await?;
        let s =
            session::current().ok_or_else(|| AppError::NotFound("nothing is recording".into()))?;
        let mut c = s.ctl.lock().unwrap();
        c.paused = b.paused.unwrap_or(!c.paused);
        Ok(ok(json!({ "paused": c.paused })))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UpdateBody {
    rect: Option<Rect>,
    /// `null` hides the bubble; absent leaves it.
    #[serde(default, deserialize_with = "some_null")]
    camera: Option<Option<CameraBody>>,
}

fn some_null<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<CameraBody>>, D::Error> {
    Ok(Some(Option::<CameraBody>::deserialize(d)?))
}

fn update() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let b: UpdateBody = take_json(req).await?;
        let s =
            session::current().ok_or_else(|| AppError::NotFound("nothing is recording".into()))?;
        let mut c = s.ctl.lock().unwrap();
        if let Some(r) = b.rect {
            c.rect = Some(r);
        }
        if let Some(cam) = b.camera {
            c.camera = Some(cam.and_then(camera_plan));
        }
        Ok(ok(json!({ "ok": true })))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MarkerBody {
    label: Option<String>,
    kind: Option<String>,
    detail: Option<String>,
}

fn marker() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let b: MarkerBody = take_json(req).await?;
        Ok(ok(
            json!({ "added": add_marker(b.label.unwrap_or_default()) }),
        ))
    })
}

pub fn add_marker(label: String) -> bool {
    match session::current() {
        Some(s) if session::is_running() => {
            s.ctl.lock().unwrap().markers.push(session::Marker {
                t_us: 0,
                label: label.chars().take(120).collect(),
            });
            true
        }
        _ => false,
    }
}

fn event() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let b: MarkerBody = take_json(req).await?;
        if let Some(s) = session::current().filter(|_| session::is_running()) {
            let mut c = s.ctl.lock().unwrap();
            if c.events.len() < 1000 {
                c.events.push(session::Event {
                    t_us: 0,
                    kind: b
                        .kind
                        .unwrap_or_else(|| "note".into())
                        .chars()
                        .take(32)
                        .collect(),
                    detail: b.detail.unwrap_or_default().chars().take(300).collect(),
                });
            }
        }
        Ok(ok(json!({ "ok": true })))
    })
}

/* ── previews ────────────────────────────────────────────────── */

fn preview() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let s =
            session::current().ok_or_else(|| AppError::NotFound("nothing is recording".into()))?;
        s.want_preview();
        let j = s.preview.lock().unwrap().clone();
        match j {
            Some(j) => Ok(bytes_response("image/jpeg", j.to_vec())),
            None => Ok(StatusCode::NO_CONTENT.into_response()),
        }
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CamQuery {
    device: Option<String>,
    w: Option<u32>,
    h: Option<u32>,
}

fn open_camera(q: &CamQuery) -> Result<Arc<camera::Latest>, AppError> {
    let need = (
        q.w.unwrap_or(640).clamp(64, 1920),
        q.h.unwrap_or(480).clamp(64, 1080),
    );
    let dev = q.device.as_deref().filter(|d| !d.is_empty()).map(Path::new);
    let l = camera::acquire(dev, need).map_err(|e| AppError::NotFound(e.to_string()))?;
    camera::want_jpeg();
    Ok(l)
}

/// A single JPEG of the camera (the bubble's fallback and first paint).
fn camera_jpg() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        local_only(&req)?;
        let q: CamQuery = query(&req)?;
        let l = open_camera(&q)?;
        for _ in 0..40 {
            if let Some(j) = l.jpeg.lock().unwrap().1.clone() {
                return Ok(bytes_response("image/jpeg", j.to_vec()));
            }
            if let Some(e) = l.error.lock().unwrap().clone() {
                return Err(AppError::Internal(e));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Ok(StatusCode::NO_CONTENT.into_response())
    })
}

/// `multipart/x-mixed-replace` JPEG stream for an `<img>`: the browser
/// decodes it in hardware and swaps frames in place.
fn camera_mjpeg() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        local_only(&req)?;
        let q: CamQuery = query(&req)?;
        let l = open_camera(&q)?;
        let body = thread_body(move |send| {
            let mut seen = 0u64;
            let mut quiet = 0u32;
            loop {
                if l.error.lock().unwrap().is_some() || !camera::is_open() || quiet > 300 {
                    return;
                }
                camera::want_jpeg();
                let (seq, j) = l.jpeg.lock().unwrap().clone();
                if seq != seen {
                    if let Some(j) = j {
                        seen = seq;
                        quiet = 0;
                        let mut part = format!(
                            "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                            j.len()
                        )
                        .into_bytes();
                        part.extend_from_slice(&j);
                        part.extend_from_slice(b"\r\n");
                        if !send(part) {
                            return;
                        }
                        continue;
                    }
                }
                quiet += 1;
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        });
        Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                "multipart/x-mixed-replace; boundary=frame",
            )
            .header(header::CACHE_CONTROL, "no-store")
            .body(body)
            .map_err(|e| AppError::Internal(e.to_string()))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MeterQuery {
    mic: Option<String>,
    desktop: Option<String>,
}

fn meter_route() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let q: MeterQuery = query(&req)?;
        let v = tokio::task::spawn_blocking(move || {
            let lv = |want: Option<String>, mon: bool| {
                devices::resolve_source(want.as_deref(), mon).map(|s| meter::level(&s))
            };
            json!({ "mic": lv(q.mic, false), "desktop": lv(q.desktop, true) })
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(ok(v))
    })
}

/* ── destinations ────────────────────────────────────────────── */

fn dest_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            Ok(ok(json!(store::list_destinations(&ctx, &uid)?)))
        }
    })
}

fn dest_save(ctx: Arc<PluginCtx>, update: bool) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = if update {
                Some(param(&req, "id")?)
            } else {
                None
            };
            let body: store::DestinationInput = take_json(req).await?;
            Ok(ok(json!(store::save_destination(
                &ctx,
                &uid,
                id.as_deref(),
                &body
            )?)))
        }
    })
}

fn dest_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = param(&req, "id")?;
            Ok(ok(
                json!({ "deleted": store::delete_destination(&ctx, &uid, &id)? }),
            ))
        }
    })
}

/* ── recordings and media ────────────────────────────────────── */

fn rec_list() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let dir = library::capture_dir(&home(&req));
        let list = tokio::task::spawn_blocking(move || library::list(&dir))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(ok(json!(list)))
    })
}

fn rec_delete() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let dir = library::capture_dir(&home(&req));
        let id = param(&req, "id")?;
        let p = library::resolve(&dir, &id)
            .ok_or_else(|| AppError::NotFound("recording not found".into()))?;
        std::fs::remove_file(&p).map_err(|e| AppError::Internal(e.to_string()))?;
        let _ = std::fs::remove_file(library::sidecar(&p));
        Ok(ok(json!({ "deleted": id })))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct MediaQuery {
    pub path: Option<String>,
    pub id: Option<String>,
    pub track: Option<u32>,
    pub n: Option<u32>,
    pub buckets: Option<u32>,
}

/// A media file the caller may read: a recording by id, or a path inside
/// their home with a media extension.
pub fn media_path(req: &Request, q: &MediaQuery) -> Result<PathBuf, AppError> {
    let h = home(req);
    if let Some(id) = &q.id {
        return library::resolve(&library::capture_dir(&h), id)
            .ok_or_else(|| AppError::NotFound("recording not found".into()));
    }
    let p = q
        .path
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("missing path".into()))?;
    let p = Path::new(p);
    let full = if p.is_absolute() {
        p.to_path_buf()
    } else {
        h.join(p)
    };
    editor::media::allowed(&h, &full)
        .ok_or_else(|| AppError::BadRequest("not a media file in your home".into()))
}

fn mime(p: &Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp4" | "m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("ogg" | "opus") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("m4a" | "aac") => "audio/mp4",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "application/octet-stream",
    }
}

fn parse_range(h: &HeaderMap, len: u64) -> Option<(u64, u64)> {
    let v = h
        .get(header::RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes=")?;
    let (a, b) = v.split_once('-')?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len - 1)
    } else {
        let s: u64 = a.parse().ok()?;
        let e = if b.is_empty() {
            len - 1
        } else {
            b.parse::<u64>().ok()?.min(len - 1)
        };
        (s, e)
    };
    (start <= end && end < len).then_some((start, end))
}

/// The file itself, with HTTP ranges so `<video>` seeks without downloading.
fn media() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let q: MediaQuery = query(&req)?;
        let p = media_path(&req, &q)?;
        let len = std::fs::metadata(&p)
            .map_err(|e| AppError::NotFound(e.to_string()))?
            .len();
        let range = if len > 0 {
            parse_range(req.headers(), len)
        } else {
            None
        };
        let (start, end) = range.unwrap_or((0, len.saturating_sub(1)));
        let count = if len == 0 { 0 } else { end - start + 1 };
        let ct = mime(&p);
        let body = thread_body(move |send| {
            use std::io::{Read, Seek, SeekFrom};
            let Ok(mut f) = std::fs::File::open(&p) else {
                return;
            };
            if f.seek(SeekFrom::Start(start)).is_err() {
                return;
            }
            let mut left = count;
            while left > 0 {
                let n = left.min(256 * 1024) as usize;
                let mut buf = vec![0u8; n];
                if f.read_exact(&mut buf).is_err() || !send(buf) {
                    return;
                }
                left -= n as u64;
            }
        });
        let mut b = Response::builder()
            .header(header::CONTENT_TYPE, ct)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_LENGTH, count)
            .header(header::CACHE_CONTROL, "private, max-age=60");
        b = if range.is_some() {
            b.status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
        } else {
            b.status(StatusCode::OK)
        };
        b.body(body).map_err(|e| AppError::Internal(e.to_string()))
    })
}

fn media_info() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let q: MediaQuery = query(&req)?;
        let p = media_path(&req, &q)?;
        let info = tokio::task::spawn_blocking(move || editor::media::probe(&p))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
        Ok(ok(json!(info)))
    })
}

fn media_peaks() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let q: MediaQuery = query(&req)?;
        let p = media_path(&req, &q)?;
        let track = q.track;
        let peaks = tokio::task::spawn_blocking(move || editor::media::peaks(&p, track))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
        Ok(ok(json!(peaks)))
    })
}

fn media_thumbs() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        user_id(&req)?;
        let q: MediaQuery = query(&req)?;
        let p = media_path(&req, &q)?;
        let n = q.n.unwrap_or(12).clamp(1, 60);
        let jpg = tokio::task::spawn_blocking(move || editor::media::strip(&p, n))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
        Ok(bytes_response("image/jpeg", jpg))
    })
}
