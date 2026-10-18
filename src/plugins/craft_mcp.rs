//! Craft-app MCP hosts.
//!
//! Every app in the Craft suite ships `<app>-cli` in its Linux package, and
//! `<app>-cli mcp` speaks the Model Context Protocol over stdio: the agent can
//! inspect and edit the app's documents through the app's own engine. The store
//! installs the CLI next to the web app (`PLUGINS_DIR/.craft-mcp/<id>/bin/`)
//! when the user turns AI control on for that app.
//!
//! **Focus mode is the boundary, and it is enforced here, in code.** One server
//! exists per focused Craft app and nothing else: [`CraftMcp::sync`] runs at the
//! start of every agent turn, starts the servers of the apps whose windows are
//! focused, and stops + unregisters every other one. An unfocused app's tools
//! are therefore neither advertised nor callable — no prompt wording involved.
//! Their tools are registered with the app's plugin id as owner, so the
//! existing focus gate (another plugin focused) applies as well, and
//! `mark_focus_only` keeps the boundary even if a server ever outlives its turn.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{Tool, ToolRequest};

use async_trait::async_trait;

use crate::plugins::manager::PluginManager;
use crate::plugins::store::CraftApp;

/// How long a freshly spawned app CLI gets to answer `initialize` and
/// `tools/list` before the turn continues without it.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
/// One `tools/call` may run an app command; long ones use the app's own
/// `wait:false` + job tools, so this is a backstop.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Tool results are model context: cap what one call may add to it.
const MAX_RESULT_BYTES: usize = 64 * 1024;
/// One tool's doc fragment (name + description + argument list).
const MAX_DOC_BYTES: usize = 4_000;
/// The protocol revision this host speaks. A server answers with the revision
/// it supports, so a newer server degrades gracefully.
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Where the store keeps an app's CLI. A dot-directory: the plugin scanner
/// skips it, and updating the web app (which swaps `PLUGINS_DIR/<id>/`) never
/// touches the binary.
pub fn cli_path(plugins_dir: &Path, id: &str) -> PathBuf {
    plugins_dir
        .join(".craft-mcp")
        .join(id)
        .join("bin")
        .join(format!("{id}-cli"))
}

/// The version of the installed CLI (written by the store), used to tell an
/// outdated AI-control bundle from a current one.
pub fn version_path(plugins_dir: &Path, id: &str) -> PathBuf {
    plugins_dir.join(".craft-mcp").join(id).join("version")
}

// ── Transport ───────────────────────────────────────────────────────────────

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// One JSON-RPC-over-stdio connection to an app's MCP server.
struct McpClient {
    stdin: Mutex<ChildStdin>,
    pending: Pending,
    next_id: AtomicU64,
    /// Held so the process is reaped (and, with `kill_on_drop`, stopped) when
    /// the host drops.
    child: Mutex<Child>,
    label: String,
}

impl McpClient {
    fn spawn(cli: &Path, label: &str, home: &str) -> Result<Self, AppError> {
        let mut child = Command::new(cli)
            .arg("mcp")
            .arg("--automation-read-root")
            .arg(home)
            .arg("--automation-write-root")
            .arg(home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // Diagnostics are not part of the protocol; never block on them.
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                AppError::Internal(format!("could not start {label} MCP: {error}"))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Internal(format!("{label} MCP has no stdin")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Internal(format!("{label} MCP has no stdout")))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = pending.clone();
        let reader_label = label.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                // Responses carry an id; notifications (progress, log) do not.
                let Some(id) = message.get("id").and_then(|value| value.as_u64()) else {
                    continue;
                };
                let Some(sender) = reader_pending.lock().await.remove(&id) else {
                    continue;
                };
                let answer = match message.get("error") {
                    Some(error) => Err(error.to_string()),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = sender.send(answer);
            }
            // EOF: every waiter fails now instead of waiting out its timeout.
            for (_, sender) in reader_pending.lock().await.drain() {
                let _ = sender.send(Err(format!("{reader_label} MCP closed the connection")));
            }
        });

        Ok(Self {
            stdin: Mutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
            child: Mutex::new(child),
            label: label.to_string(),
        })
    }

    async fn write_line(&self, message: &Value) -> Result<(), AppError> {
        let mut line = message.to_string();
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await.map_err(|error| {
            AppError::Internal(format!("{} MCP write failed: {error}", self.label))
        })?;
        stdin.flush().await.map_err(|error| {
            AppError::Internal(format!("{} MCP write failed: {error}", self.label))
        })
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), AppError> {
        self.write_line(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, AppError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        let message = json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        });
        if let Err(error) = self.write_line(&message).await {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(CALL_TIMEOUT, receiver).await {
            Ok(Ok(answer)) => answer.map_err(|error| {
                AppError::Internal(format!("{} MCP call failed: {error}", self.label))
            }),
            Ok(Err(_)) => Err(AppError::Internal(format!(
                "{} MCP dropped the request",
                self.label
            ))),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(AppError::Internal(format!(
                    "{} MCP did not answer within {}s",
                    self.label,
                    CALL_TIMEOUT.as_secs()
                )))
            }
        }
    }

    async fn list_tools(&self) -> Result<Vec<McpToolInfo>, AppError> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let result = self.request("tools/list", params).await?;
            if let Some(tools) = result.get("tools").and_then(|value| value.as_array()) {
                for tool in tools {
                    let Some(name) = tool.get("name").and_then(|value| value.as_str()) else {
                        continue;
                    };
                    out.push(McpToolInfo {
                        name: name.to_string(),
                        description: tool
                            .get("description")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        input_schema: tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| json!({})),
                    });
                }
            }
            cursor = result
                .get("nextCursor")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// Ask the process to exit and reap it. Idempotent.
    async fn stop(&self) {
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

// ── Tool metadata and registration ──────────────────────────────────────────

#[derive(Debug, Clone)]
struct McpToolInfo {
    /// The name the app's server answers to (`doc_open`, …).
    name: String,
    description: String,
    input_schema: Value,
}

/// The action key a Craft app tool is registered as. The app id prefixes it so
/// two apps (and the core) can never collide on a common name, and the model
/// can tell whose tool it is.
fn registered_tool_name(app_id: &str, mcp_name: &str) -> String {
    let mut suffix = String::with_capacity(mcp_name.len());
    for ch in mcp_name.chars() {
        if ch.is_ascii_alphanumeric() {
            suffix.push(ch.to_ascii_lowercase());
        } else {
            suffix.push('_');
        }
    }
    format!("{app_id}_{suffix}")
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut cut = max;
    while !text.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    format!("{}…", &text[..cut])
}

/// A tool result cut at the context budget, saying how much was left out and
/// what to do about it — an app catalog can be hundreds of kilobytes, and a
/// silently cut JSON blob reads as corrupt.
fn truncate_result(text: &str) -> String {
    if text.len() <= MAX_RESULT_BYTES {
        return text.to_string();
    }
    let mut cut = MAX_RESULT_BYTES;
    while !text.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    format!(
        "{}\n[truncated: {} of {} bytes shown — ask the app for less at a time (filter, page \
         or a narrower query) instead of the whole list]",
        &text[..cut],
        cut,
        text.len()
    )
}

/// One line of argument names/types from the tool's JSON schema — enough for
/// the model to call it, without dumping the whole schema into the prompt.
fn argument_summary(schema: &Value) -> String {
    let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) else {
        return String::new();
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|value| value.as_array())
        .map(|list| list.iter().filter_map(|value| value.as_str()).collect())
        .unwrap_or_default();
    let mut names: Vec<(&String, &Value)> = properties.iter().collect();
    names.sort_by_key(|(name, _)| name.to_string());
    let arguments: Vec<String> = names
        .into_iter()
        .map(|(name, spec)| {
            let kind = spec.get("type").and_then(|value| value.as_str()).unwrap_or("any");
            if required.iter().any(|required| *required == name) {
                format!("{name} ({kind}, required)")
            } else {
                format!("{name} ({kind})")
            }
        })
        .collect();
    truncate(&arguments.join(", "), 1200)
}

fn tool_doc(app_name: &str, registered: &str, info: &McpToolInfo) -> String {
    let description = truncate(info.description.trim(), 900);
    let arguments = argument_summary(&info.input_schema);
    let mut doc = format!("### {app_name}: `{registered}`\n");
    if !description.is_empty() {
        doc.push_str(&description);
        doc.push('\n');
    }
    if !arguments.is_empty() {
        doc.push_str(&format!("Arguments: {arguments}\n"));
    }
    truncate(&doc, MAX_DOC_BYTES)
}

/// A running app server, shared by every tool it advertised.
struct ServerHandle {
    client: McpClient,
}

// ── The `Tool` adapter ──────────────────────────────────────────────────────

struct McpTool {
    registered: String,
    step: String,
    doc: String,
    tool_name: String,
    server: Arc<ServerHandle>,
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.registered
    }

    fn step_label(&self) -> &str {
        &self.step
    }

    fn doc_fragment(&self) -> Option<&str> {
        Some(&self.doc)
    }

    async fn invoke(
        &self,
        _ctx: &PluginCtx,
        req: ToolRequest<'_>,
    ) -> Result<ActionOutcome, AppError> {
        let result = self
            .server
            .client
            .request(
                "tools/call",
                json!({ "name": self.tool_name, "arguments": req.params }),
            )
            .await?;
        let is_error = result
            .get("isError")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mut text = String::new();
        let mut images = 0usize;
        if let Some(items) = result.get("content").and_then(|value| value.as_array()) {
            for item in items {
                match item.get("type").and_then(|value| value.as_str()) {
                    Some("text") => {
                        if let Some(chunk) = item.get("text").and_then(|value| value.as_str()) {
                            text.push_str(chunk);
                            text.push('\n');
                        }
                    }
                    Some("image") => images += 1,
                    _ => {}
                }
            }
        }
        let text = truncate_result(text.trim());
        if is_error {
            return Ok(ActionOutcome::error(
                self.registered.clone(),
                if text.is_empty() { "the app reported an error".into() } else { text },
            ));
        }
        let mut data = json!({ "content": text });
        if images > 0 {
            data["images"] = json!(images);
            data["note"] = json!("image content is not included in this reply");
        }
        Ok(ActionOutcome::ok(self.registered.clone(), data))
    }
}

// ── The host ────────────────────────────────────────────────────────────────

/// Runs at most one MCP server per focused Craft app. Cloned cheaply (the
/// process map is shared).
#[derive(Clone, Default)]
pub struct CraftMcp {
    hosts: Arc<Mutex<HashMap<String, Arc<ServerHandle>>>>,
}

impl CraftMcp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apps whose MCP server is running right now (diagnostics/tests).
    pub async fn running_apps(&self) -> Vec<String> {
        let mut out: Vec<String> = self.hosts.lock().await.keys().cloned().collect();
        out.sort();
        out
    }

    /// The hard focus rule. Called once per agent turn, before the tool list is
    /// built: start the servers of the focused Craft apps, stop and unregister
    /// every other one. `home` scopes the app's file access.
    pub async fn sync(
        &self,
        manager: &PluginManager,
        plugins_dir: &Path,
        home: &str,
        focused: &BTreeSet<String>,
    ) {
        // Stop what is no longer focused.
        let stale: Vec<(String, Arc<ServerHandle>)> = {
            let hosts = self.hosts.lock().await;
            hosts
                .iter()
                .filter(|(id, _)| !focused.contains(*id))
                .map(|(id, host)| (id.clone(), host.clone()))
                .collect()
        };
        for (id, host) in stale {
            host.client.stop().await;
            // Craft apps contribute no tools of their own, so this removes
            // exactly the MCP tools registered under this app id.
            manager.tools().uninstall_plugin(&id);
            self.hosts.lock().await.remove(&id);
            tracing::debug!("Craft MCP for {id} stopped (no longer focused)");
        }

        // Start what is focused and has a CLI installed.
        for id in focused {
            if self.hosts.lock().await.contains_key(id) {
                continue;
            }
            let Some(app) = CraftApp::by_id(id) else { continue };
            if !manager.kind_for(id).is_web() {
                continue; // only the store's web apps carry an MCP CLI
            }
            let cli = cli_path(plugins_dir, id);
            if !cli.is_file() {
                continue; // AI control was not enabled for this app
            }
            match start_server(app, &cli, home, manager).await {
                Ok(host) => {
                    self.hosts.lock().await.insert(id.clone(), host);
                }
                Err(error) => {
                    tracing::warn!("Craft MCP for {} did not start: {error}", app.id)
                }
            }
        }
    }
}

/// Spawn one app CLI, handshake, list its tools and register them under the
/// app's plugin id (marked focus-only).
async fn start_server(
    app: &'static CraftApp,
    cli: &Path,
    home: &str,
    manager: &PluginManager,
) -> Result<Arc<ServerHandle>, AppError> {
    let client = McpClient::spawn(cli, app.name, home)?;
    let initialized = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        client
            .request(
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "shiny", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        client.notify("notifications/initialized", json!({})).await?;
        client.list_tools().await
    })
    .await
    .map_err(|_| AppError::Internal(format!("{} MCP handshake timed out", app.name)))??;

    let server = Arc::new(ServerHandle { client });
    let registry = manager.tools();
    for info in initialized {
        let registered = registered_tool_name(app.id, &info.name);
        let tool = McpTool {
            doc: tool_doc(app.name, &registered, &info),
            step: format!("Using {}…", app.name),
            tool_name: info.name,
            registered,
            server: server.clone(),
        };
        registry.install_owned(Arc::new(tool), app.id);
    }
    registry.mark_focus_only(app.id);
    tracing::info!("Craft MCP for {} ready", app.id);
    Ok(server)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, description: &str, schema: Value) -> McpToolInfo {
        McpToolInfo {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
        }
    }

    /// Tool keys are prefixed with the app id so two apps (and the core) never
    /// collide, and the model can tell whose tool it is.
    #[test]
    fn tool_names_are_prefixed_and_normalized() {
        assert_eq!(
            registered_tool_name("photocraft", "doc_open"),
            "photocraft_doc_open"
        );
        assert_eq!(
            registered_tool_name("wordcraft", "execute"),
            "wordcraft_execute"
        );
        assert_eq!(
            registered_tool_name("photocraft", "ui.menu.invoke"),
            "photocraft_ui_menu_invoke"
        );
        assert_ne!(
            registered_tool_name("photocraft", "doc_open"),
            registered_tool_name("wordcraft", "doc_open")
        );
    }

    /// The doc fragment names the tool, says what it does and lists its
    /// arguments (required first-class), so the model can call it without the
    /// whole JSON schema in the prompt.
    #[test]
    fn docs_are_compact_and_name_the_arguments() {
        let doc = tool_doc(
            "PhotoCraft",
            "photocraft_command_run",
            &info(
                "command_run",
                "Run one engine command by id. Use command_list to discover ids.",
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "params": { "type": "object" },
                        "wait": { "type": "boolean" }
                    },
                    "required": ["id"]
                }),
            ),
        );
        assert!(doc.contains("PhotoCraft: `photocraft_command_run`"), "{doc}");
        assert!(doc.contains("Use command_list"), "{doc}");
        assert!(doc.contains("id (string, required)"), "{doc}");
        assert!(doc.contains("wait (boolean)"), "{doc}");
        assert!(!doc.contains("JSON Schema"), "the raw schema stays out: {doc}");
        assert!(doc.len() <= MAX_DOC_BYTES);
    }

    #[test]
    fn docs_and_results_are_bounded() {
        let long = "x".repeat(10_000);
        let doc = tool_doc("App", "app_tool", &info("tool", &long, json!({})));
        assert!(doc.len() <= MAX_DOC_BYTES + 8, "{}", doc.len());
        assert!(doc.contains('…'), "the description is truncated: {doc}");
        assert!(!doc.contains(&"x".repeat(2_000)), "not the whole description");
        assert!(truncate(&long, 100).len() <= 104);
        // A multi-byte character is never split.
        assert!(truncate(&"é".repeat(100), 7).is_char_boundary(0));
    }

    /// A whole app catalog can be hundreds of kilobytes: the result is cut at
    /// the budget and says what was left out, so the model asks for less
    /// instead of reading a silently corrupted JSON blob.
    #[test]
    fn oversized_results_say_so() {
        let long = "y".repeat(MAX_RESULT_BYTES + 500);
        let out = truncate_result(&long);
        assert!(out.len() < long.len(), "{}", out.len());
        assert!(out.contains("[truncated:"), "{}", &out[out.len() - 160..]);
        assert!(
            out.contains(&format!("of {} bytes", long.len())),
            "names the real size"
        );
        // A small result passes through untouched.
        assert_eq!(truncate_result("ok"), "ok");
    }

    /// The CLI lives outside the plugin directory, in a dot-dir the plugin
    /// scanner skips — a web-app update swaps the plugin dir and must not take
    /// the binary with it.
    #[test]
    fn the_cli_lives_outside_the_plugin_directory() {
        let path = cli_path(Path::new("/data/plugins"), "photocraft");
        assert_eq!(
            path,
            Path::new("/data/plugins/.craft-mcp/photocraft/bin/photocraft-cli")
        );
        let relative = path.strip_prefix("/data/plugins").unwrap();
        assert!(relative.components().next().unwrap().as_os_str() == ".craft-mcp");
    }

    /// End to end against a real app CLI, when one is pointed at:
    ///
    /// ```sh
    /// SHINY_CRAFT_CLI=/path/to/photocraft-cli \
    ///   cargo test -p shiny --lib a_real_app_cli -- --ignored --nocapture
    /// ```
    ///
    /// Ignored by default: CI has no app binary (and the CLI downloads 60–90 MB
    /// of engine). This is the check that the handshake, `tools/list` and
    /// `tools/call` work against the real server.
    #[tokio::test]
    #[ignore = "needs a Craft app CLI (set SHINY_CRAFT_CLI)"]
    async fn a_real_app_cli_registers_and_runs_its_tools() {
        let Ok(cli) = std::env::var("SHINY_CRAFT_CLI") else {
            eprintln!("SHINY_CRAFT_CLI not set, skipping");
            return;
        };
        let client = McpClient::spawn(Path::new(&cli), "PhotoCraft", "/tmp").expect("spawn");
        let tools = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            client
                .request(
                    "initialize",
                    json!({
                        "protocolVersion": MCP_PROTOCOL_VERSION,
                        "capabilities": {},
                        "clientInfo": { "name": "shiny-test", "version": "0.1" },
                    }),
                )
                .await?;
            client.notify("notifications/initialized", json!({})).await?;
            client.list_tools().await
        })
        .await
        .expect("handshake did not time out")
        .expect("handshake succeeded");
        assert!(
            tools.iter().any(|tool| tool.name == "command_list"),
            "the app advertises command_list: {:?}",
            tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>()
        );
        let info = tools.iter().find(|tool| tool.name == "command_list").unwrap();
        assert!(!info.description.is_empty());
        assert!(tool_doc("PhotoCraft", "photocraft_command_list", info)
            .contains("photocraft_command_list"));
        // What a focused PhotoCraft adds to the prompt, with the real server.
        let total: usize = tools
            .iter()
            .map(|tool| tool_doc("PhotoCraft", "x", tool).len())
            .sum();
        println!("{} tools, {total} doc bytes", tools.len());
        assert!(total < 120_000, "doc fragments stay bounded: {total}");

        let result = client
            .request(
                "tools/call",
                json!({ "name": "command_list", "arguments": {} }),
            )
            .await
            .expect("tools/call answers");
        assert_ne!(
            result.get("isError"),
            Some(&Value::Bool(true)),
            "command_list must not report an error"
        );
        let text = result["content"][0]["text"]
            .as_str()
            .expect("text content");
        assert!(
            text.trim_start().starts_with('['),
            "command_list answers a bare JSON array: {}",
            &text[..text.len().min(120)]
        );
        client.stop().await;
    }
}
