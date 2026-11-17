//! Run a plugin as its own process.
//!
//! A plugin crate with `runtime = "process"` in its `plugin.toml` ships an
//! executable instead of a cdylib. Its `main` is one line:
//!
//! ```ignore
//! fn main() {
//!     shiny_plugin_sdk::process::run(my_plugin::MyPlugin::new());
//! }
//! ```
//!
//! The same `impl Plugin` serves both shapes: [`run`] listens on the Unix
//! socket the host names in `SHINY_PLUGIN_SOCKET`, answers the handshake with
//! what `register()` contributed, runs tools and lifecycle hooks when the host
//! asks, and hands forwarded route requests to `route_handler(tag)` exactly as
//! the host would in-process. The wire contract is the
//! `shiny-plugin-protocol` crate.
//!
//! Nothing Rust-shaped crosses the process boundary, so a plugin built with any
//! compiler and any version of this SDK keeps working with later hosts for as
//! long as they speak its protocol version.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures::FutureExt;
use shiny_plugin_protocol as proto;

use crate::context::AgentContext;
use crate::errors::AppError;
use crate::plugin::Plugin;
use crate::routes::RouteHandler;
use crate::services::{ConfigSnapshot, PluginCtx};
use crate::tools::{RegistryBuilder, Tool, ToolRequest, UiText};

/// Process-level choices a plugin makes in its `main`.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Let the host stop this process after this long without requests; the
    /// next request starts it again. Leave `None` when `on_load` starts
    /// background work that must keep running (a mail sync, a scheduler).
    pub idle_stop: Option<Duration>,
}

/// Serve `plugin` on the socket the host provided, until the host stops it.
pub fn run<P: Plugin + 'static>(plugin: P) -> ! {
    run_with(plugin, Options::default())
}

/// [`run`] with [`Options`].
pub fn run_with<P: Plugin + 'static>(plugin: P, options: Options) -> ! {
    init_logging();
    let socket = match std::env::var(proto::SOCKET_ENV) {
        Ok(path) if !path.is_empty() => path,
        _ => {
            eprintln!(
                "this is a Shiny plugin: the Shiny host starts it with {} set",
                proto::SOCKET_ENV
            );
            std::process::exit(2);
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("shiny-plugin")
        .build()
        .expect("plugin runtime");
    let code = runtime.block_on(async move {
        match serve(Arc::new(plugin), options, &socket).await {
            Ok(()) => 0,
            Err(error) => {
                tracing::error!("plugin server stopped: {error}");
                1
            }
        }
    });
    std::process::exit(code)
}

/// Plain lines on stderr: the host prefixes each with the plugin's name and
/// writes it to its own log.
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .try_init();
}

/// What `register()` produced, resolved for dispatch.
struct Registered {
    ctx: Arc<PluginCtx>,
    /// Name and every alias → the tool.
    tools: HashMap<String, Arc<dyn Tool>>,
    routes: HashMap<String, RouteHandler>,
}

struct Server {
    plugin: Arc<dyn Plugin>,
    options: Options,
    registered: RwLock<Option<Arc<Registered>>>,
}

impl Server {
    fn registered(&self) -> Option<Arc<Registered>> {
        self.registered
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

async fn serve(plugin: Arc<dyn Plugin>, options: Options, socket: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let _ = std::fs::remove_file(socket);
    let listener = tokio::net::UnixListener::bind(socket)?;
    // Only the account running the host may connect.
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;

    // The host also asks the kernel to stop us when it dies (PDEATHSIG), but
    // that is tied to the thread that started us; watching our parent covers
    // every case.
    // every case. Our socket being removed or replaced means the host has
    // moved on (a re-executed server keeps its pid): exit too.
    let parent = std::os::unix::process::parent_id();
    let socket_path = socket.to_string();
    let inode = socket_inode(&socket_path);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if std::os::unix::process::parent_id() != parent {
                tracing::info!("the host is gone; exiting");
                std::process::exit(0);
            }
            if socket_inode(&socket_path) != inode {
                tracing::info!("the socket was taken away; exiting");
                std::process::exit(0);
            }
        }
    });

    let server = Arc::new(Server {
        plugin,
        options,
        registered: RwLock::new(None),
    });
    let control = Router::new()
        .route(proto::paths::HELLO, post(hello))
        .route(proto::paths::LOAD, post(load))
        .route(proto::paths::UNLOAD, post(unload))
        .route(proto::paths::USER_REGISTERED, post(user_registered))
        .route(proto::paths::TOOL, post(tool))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024));
    let app: Router = control.fallback(route).with_state(server);

    loop {
        let (stream, _) = listener.accept().await?;
        let service = hyper_util::service::TowerToHyperService::new(app.clone());
        tokio::spawn(async move {
            let io = hyper_util::rt::TokioIo::new(stream);
            if let Err(error) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                tracing::debug!("connection ended: {error}");
            }
        });
    }
}

async fn hello(
    State(server): State<Arc<Server>>,
    Json(hello): Json<proto::Hello>,
) -> Json<proto::Welcome> {
    let manifest = server.plugin.manifest().clone();
    let manifest_json = serde_json::to_value(&manifest).unwrap_or_default();
    let refuse = |error: String| {
        Json(proto::Welcome {
            manifest: manifest_json.clone(),
            sdk: env!("CARGO_PKG_VERSION").into(),
            error: Some(error),
            ..Default::default()
        })
    };
    let Some(version) = proto::negotiate(hello.protocol, proto::VersionRange::current()) else {
        return refuse(format!(
            "no shared protocol version: host speaks {}..={}, plugin {}..={}",
            hello.protocol.min,
            hello.protocol.max,
            proto::PROTOCOL_MIN,
            proto::PROTOCOL_MAX
        ));
    };
    let config: ConfigSnapshot = match serde_json::from_value(hello.config) {
        Ok(config) => config,
        Err(error) => return refuse(format!("unreadable host config: {error}")),
    };
    let ctx = PluginCtx::new(config, manifest);

    let mut builder = RegistryBuilder::new();
    let plugin = server.plugin.clone();
    let registered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        plugin.register(ctx.clone(), &mut builder);
    }));
    if registered.is_err() {
        return refuse("registration panicked".into());
    }

    let mut tools = HashMap::new();
    let mut specs = Vec::new();
    for tool in &builder.tools {
        specs.push(proto::ToolSpec {
            name: tool.name().to_string(),
            aliases: tool.aliases().iter().map(|a| a.to_string()).collect(),
            step_label: tool.step_label().to_string(),
            doc_fragment: tool.doc_fragment().map(str::to_string),
            step_ui: tool.step_ui().map(ui_to_wire),
        });
        tools.insert(tool.name().to_string(), tool.clone());
        for alias in tool.aliases() {
            tools.insert(alias.to_string(), tool.clone());
        }
    }
    let mut routes = HashMap::new();
    let mut decls = Vec::new();
    for spec in &builder.routes {
        let Some(handler) = server.plugin.route_handler(&spec.handler_tag) else {
            return refuse(format!("unresolved route handler '{}'", spec.handler_tag));
        };
        routes.insert(spec.handler_tag.clone(), handler);
        decls.push(proto::RouteDecl {
            method: spec.method.as_str().to_string(),
            path: spec.path.clone(),
            auth: spec.auth.clone(),
            handler_tag: spec.handler_tag.clone(),
        });
    }
    *server.registered.write().unwrap_or_else(|e| e.into_inner()) =
        Some(Arc::new(Registered { ctx, tools, routes }));

    Json(proto::Welcome {
        protocol: version,
        manifest: manifest_json,
        tools: specs,
        routes: decls,
        skills_md: builder.skills_md,
        persona: builder.persona,
        context_lines: builder.context_lines,
        idle_stop_secs: server.options.idle_stop.map(|d| d.as_secs().max(1)),
        sdk: env!("CARGO_PKG_VERSION").into(),
        error: None,
    })
}

fn not_ready() -> proto::Done {
    proto::Done {
        error: Some("the host has not said hello yet".into()),
    }
}

/// Run a lifecycle hook, turning a panic into an error reply.
async fn guarded<F: std::future::Future<Output = ()>>(fut: F) -> proto::Done {
    match std::panic::AssertUnwindSafe(fut).catch_unwind().await {
        Ok(()) => proto::Done::default(),
        Err(panic) => proto::Done {
            error: Some(panic_message(&*panic)),
        },
    }
}

async fn load(State(server): State<Arc<Server>>) -> Json<proto::Done> {
    let Some(reg) = server.registered() else {
        return Json(not_ready());
    };
    Json(guarded(server.plugin.on_load(reg.ctx.clone())).await)
}

async fn unload(State(server): State<Arc<Server>>) -> Json<proto::Done> {
    let Some(reg) = server.registered() else {
        return Json(not_ready());
    };
    Json(guarded(server.plugin.on_unload(reg.ctx.clone())).await)
}

async fn user_registered(
    State(server): State<Arc<Server>>,
    Json(body): Json<proto::UserRegistered>,
) -> Json<proto::Done> {
    let Some(reg) = server.registered() else {
        return Json(not_ready());
    };
    Json(
        guarded(
            server
                .plugin
                .on_user_registered(reg.ctx.clone(), &body.user_id),
        )
        .await,
    )
}

async fn tool(
    State(server): State<Arc<Server>>,
    Json(call): Json<proto::ToolCall>,
) -> Json<proto::ToolReply> {
    let fail = |kind: &str, message: String| {
        Json(proto::ToolReply {
            error: Some(proto::WireError {
                kind: kind.into(),
                message,
            }),
            ..Default::default()
        })
    };
    let Some(reg) = server.registered() else {
        return fail("internal", "the host has not said hello yet".into());
    };
    let Some(tool) = reg.tools.get(&call.tool).cloned() else {
        return fail("not_found", format!("no tool '{}'", call.tool));
    };
    let context: AgentContext = match serde_json::from_value(call.context) {
        Ok(context) => context,
        Err(error) => return fail("bad_request", format!("unreadable agent context: {error}")),
    };
    let request = ToolRequest {
        user_id: &call.user_id,
        traveler_id: &call.traveler_id,
        params: &call.params,
        ctx: &context,
        os_home: call.os_home.as_deref(),
    };
    let result = std::panic::AssertUnwindSafe(tool.invoke(&reg.ctx, request))
        .catch_unwind()
        .await;
    match result {
        Ok(Ok(outcome)) => {
            let humanized = tool.humanize(&outcome.result, &outcome.data);
            let done_ui = tool.done_ui(&outcome.result, &outcome.data).map(ui_to_wire);
            Json(proto::ToolReply {
                outcome: Some(proto::Outcome {
                    action: outcome.action,
                    result: outcome.result,
                    data: outcome.data,
                    artifact: outcome.artifact.as_ref().map(to_value),
                    extra_artifacts: outcome.extra_artifacts.iter().map(to_value).collect(),
                    navigation: outcome.navigation.as_ref().map(to_value),
                }),
                error: None,
                humanized: Some(humanized),
                done_ui,
            })
        }
        Ok(Err(error)) => {
            let wire = error_to_wire(error);
            fail(&wire.kind, wire.message)
        }
        Err(panic) => fail(
            "internal",
            format!("tool panicked: {}", panic_message(&*panic)),
        ),
    }
}

/// Any request outside `/_shiny/`: a user's request to one of the routes,
/// forwarded by the host with the route's tag in a header.
async fn route(State(server): State<Arc<Server>>, mut req: Request) -> Response {
    let tag = req
        .headers_mut()
        .remove(proto::ROUTE_TAG_HEADER)
        .and_then(|v| v.to_str().ok().map(str::to_string));
    let handler = match (tag, server.registered()) {
        (Some(tag), Some(reg)) => reg.routes.get(&tag).cloned(),
        _ => None,
    };
    let Some(handler) = handler else {
        return AppError::NotFound("no such plugin route".into()).into_response();
    };
    match std::panic::AssertUnwindSafe(handler(req))
        .catch_unwind()
        .await
    {
        Ok(response) => response,
        Err(panic) => {
            tracing::error!("route handler panicked: {}", panic_message(&*panic));
            AppError::Internal("plugin route failed".into()).into_response()
        }
    }
}

fn ui_to_wire(ui: UiText) -> proto::UiText {
    proto::UiText {
        key: ui.key,
        params: ui.params,
        text: ui.text,
    }
}

fn error_to_wire(error: AppError) -> proto::WireError {
    let (kind, message) = match error {
        AppError::NotFound(m) => ("not_found", m),
        AppError::Unauthorized(m) => ("unauthorized", m),
        AppError::BadRequest(m) => ("bad_request", m),
        AppError::Internal(m) => ("internal", m),
        AppError::Database(e) => {
            tracing::error!("database error: {e}");
            ("internal", "Database error".to_string())
        }
        AppError::Http(e) => ("internal", format!("External service error: {e}")),
    };
    proto::WireError {
        kind: kind.into(),
        message,
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

fn socket_inode(path: &str) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).ok().map(|m| m.ino())
}

fn to_value<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_default()
}
