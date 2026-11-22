//! The builder's commands: what each one does once the core has decided the
//! conversation may call it.
//!
//! [`run`] is the only way in. Before any command it applies, in code, the
//! rules a conversation works under: a remote client builds nothing; a
//! command of another language's module is refused; and nothing is written,
//! built, packed or installed until the user approved a plan in this chat.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::state::{self, AgentState, PlanState};
use super::workspace::{NewPlugin, TemplateValues, Workspace, Workspaces};
use super::{checks, package, python, rust, Language};
use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::services::agent_cancel::CancelHandle;
use crate::services::agent_tools::ActionOutcome;

/// What a command needs from its turn.
pub struct Builder<'a> {
    pub state: &'a AppState,
    pub traveler: &'a Traveler,
    pub conversation_id: &'a str,
    pub cancel: Option<&'a CancelHandle>,
    /// The turn came from a remote client (Iroh, a public URL).
    pub remote: bool,
}

/// Commands that change something: they wait for an approved plan.
const NEEDS_PLAN: &[&str] = &[
    "plugin_dev_new",
    "plugin_dev_write",
    "plugin_dev_patch",
    "plugin_dev_delete",
    "plugin_dev_pack",
    "plugin_dev_install",
    "rust_build",
    "rust_test",
    "py_check",
    "py_test",
];

fn outcome(action: &str, result: &str, data: Value) -> ActionOutcome {
    ActionOutcome {
        action: action.to_string(),
        result: result.to_string(),
        data,
        artifact: None,
        extra_artifacts: vec![],
        owner: None,
    }
}

fn refused(action: &str, reason: impl Into<String>) -> ActionOutcome {
    outcome(action, "refused", json!({ "reason": reason.into() }))
}

fn text(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

/// A string parameter kept exactly as written (file content, text to find).
fn raw<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn required(params: &Value, key: &str) -> Result<String, AppError> {
    text(params, key).ok_or_else(|| AppError::BadRequest(format!("{key} required")))
}

/// Run one builder command for this conversation.
pub async fn run(builder: &Builder<'_>, action: &str, params: &Value) -> Result<ActionOutcome, AppError> {
    if builder.remote {
        return Ok(refused(
            action,
            "plugins can only be built from the machine itself, not from a remote connection. Nothing was done.",
        ));
    }
    let pool = &builder.state.pool;
    let chat = state::load(pool, builder.conversation_id, &builder.traveler.id).await?;
    if let Some(reason) = super::scope_refusal(action, chat.language) {
        return Ok(refused(action, reason));
    }
    if NEEDS_PLAN.contains(&action) && chat.plan_state != PlanState::Approved {
        return Ok(refused(
            action,
            match chat.plan_state {
                PlanState::Proposed => "the user has not answered the plan yet. Nothing was changed: wait for their answer.",
                PlanState::Changes => "the user asked for changes to the plan. Nothing was changed: revise it and call plan_propose again.",
                PlanState::Cancelled => "the user cancelled the plan. Nothing was changed: do not go on unless they ask again, and then propose a new plan.",
                _ => "this changes the plugin, and the user has not approved a plan in this chat. Nothing was changed: call plan_propose with the steps you intend to take, and go on once they approve.",
            },
        ));
    }
    let Some(language) = chat.language else {
        // `scope_refusal` lets only the entry command through without one.
        return Ok(refused(action, "call plugin_dev_start first"));
    };
    let workspaces = Workspaces::beside_plugins(&builder.state.config.plugins_dir);

    match action {
        "plugin_dev_new" => new_plugin(builder, &workspaces, language, params).await,
        "plugin_dev_list" => Ok(list(&workspaces, &chat, language)),
        "rust_doctor" => Ok(doctor(language)),
        "plugin_dev_open" => open(builder, &workspaces, &chat, language, params).await,
        "plugin_dev_rollback" => rollback(builder, &chat, params).await,
        "plugin_dev_uninstall" => uninstall(builder, &chat, params).await,
        _ => {
            let workspace = match target(builder, &workspaces, &chat, language, params).await? {
                Ok(workspace) => workspace,
                Err(reason) => return Ok(refused(action, reason)),
            };
            match action {
                "plugin_dev_tree" => Ok(tree(&workspace)),
                "plugin_dev_read" => read(&workspace, params),
                "plugin_dev_write" => write(&workspace, params),
                "plugin_dev_patch" => patch(&workspace, params),
                "plugin_dev_delete" => delete(&workspace, params),
                "check_all" => check(builder, workspace, action, None),
                "check_manifest" | "check_web" | "check_i18n" | "check_migrations" | "check_conflicts" => {
                    check(builder, workspace, action, action.strip_prefix("check_"))
                }
                "rust_build" | "rust_lint" if workspace.prebuilt() => Ok(refused(
                    action,
                    "this plugin was opened from its installed copy and its engine has no sources here: only its \
                     window, skill, translations and manifest can change.",
                )),
                "rust_build" => build(builder, &workspaces, workspace).await,
                "rust_lint" => lint(builder, &workspaces, workspace).await,
                "plugin_dev_console" => Ok(console(builder, &workspace, params)),
                "plugin_dev_call" => call(builder, &workspace, params).await,
                "plugin_dev_http" => http(builder, &workspace, params).await,
                "py_check" => py_check(workspace).await,
                "rust_test" | "py_test" => dry_run(builder, &workspace, action).await,
                "plugin_dev_pack" => pack(workspace),
                "plugin_dev_install" => install(builder, workspace).await,
                "plugin_dev_status" => Ok(status(builder, &workspace)),
                "plugin_dev_logs" => Ok(logs(builder, &workspace, params)),
                other => Err(AppError::BadRequest(format!("Unknown action: {other}"))),
            }
        }
    }
}

/// The workspace a command works on: the one named, else the one this chat
/// is working on. A plugin written in another language is another chat's.
async fn target(
    builder: &Builder<'_>,
    workspaces: &Workspaces,
    chat: &AgentState,
    language: Language,
    params: &Value,
) -> Result<Result<Workspace, String>, AppError> {
    let Some(name) = text(params, "name").or_else(|| text(params, "plugin")).or_else(|| chat.plugin.clone()) else {
        return Ok(Err(
            "this chat is not working on a plugin yet. Create one with plugin_dev_new, or name an existing one \
             (plugin_dev_list shows them)."
                .into(),
        ));
    };
    let workspace = match workspaces.open(&name) {
        Ok(workspace) => workspace,
        Err(error) => return Ok(Err(error.to_string())),
    };
    // An adopted plugin has no engine sources: any chat can work on its window.
    if !workspace.prebuilt() && workspace.language() != Some(language) {
        return Ok(Err(format!(
            "the plugin '{name}' is written in {} and this chat builds in {}. Tell the user it has to be opened in \
             a new chat.",
            workspace.language().map_or("another language", Language::label),
            language.label()
        )));
    }
    if chat.plugin.as_deref() != Some(name.as_str()) {
        state::set_plugin(&builder.state.pool, builder.conversation_id, &builder.traveler.id, &name).await?;
    }
    Ok(Ok(workspace))
}

async fn new_plugin(
    builder: &Builder<'_>,
    workspaces: &Workspaces,
    language: Language,
    params: &Value,
) -> Result<ActionOutcome, AppError> {
    const ACTION: &str = "plugin_dev_new";
    let name = required(params, "name")?.to_ascii_lowercase();
    super::workspace::validate_name(&name)?;
    // A plugin's tools are named `<name>_…`: these would shadow core commands.
    const RESERVED: &[&str] = &["rust", "python", "plugin", "plugin_dev", "core", "ask", "plan", "check"];
    if RESERVED.contains(&name.replace('-', "_").as_str()) {
        return Ok(refused(ACTION, format!("'{name}' is a reserved name. Choose another name.")));
    }
    if crate::plugins::store::CraftApp::by_id(&name).is_some() {
        return Ok(refused(ACTION, format!("'{name}' is the name of a Craft app. Choose another name.")));
    }
    if builder.state.plugins.is_installed(&name) && !workspaces.exists(&name) {
        return Ok(refused(
            ACTION,
            format!("a plugin named '{name}' is already installed. Choose another name for the new one."),
        ));
    }
    let title = text(params, "title").unwrap_or_else(|| {
        let mut chars = name.chars();
        chars.next().map(|first| first.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
    });
    let plugin = NewPlugin {
        name: name.clone(),
        title,
        description: text(params, "description").unwrap_or_else(|| "A plugin built by the assistant".into()),
        category: text(params, "category").unwrap_or_else(|| "Office".into()),
    };
    let values = TemplateValues::of(&plugin);
    let workspace = workspaces.create(&name, language)?;
    let filled = super::workspace::scaffold_common(&workspace, &values).and_then(|()| match language {
        Language::Rust => rust::scaffold(&workspace, &values),
        Language::Python => python::scaffold(&workspace, &values),
    });
    if let Err(error) = filled {
        // Half a template is worse than none: the name stays free.
        let _ = std::fs::remove_dir_all(&workspace.dir);
        return Err(error);
    }
    state::set_plugin(&builder.state.pool, builder.conversation_id, &builder.traveler.id, &name).await?;
    let files: Vec<String> = workspace.files().into_iter().map(|entry| entry.path).collect();
    Ok(outcome(
        ACTION,
        "ok",
        json!({
            "plugin": name,
            "language": language.as_str(),
            "files": files,
            "note": format!(
                "Created the plugin '{name}' from the {} template: a working example with a window, two tools \
                 ({ident}_add, {ident}_list), two routes and one table. Read the files before changing them, and \
                 turn the example into what the user asked for.",
                language.label(),
                ident = values.ident,
            ),
        }),
    ))
}

fn list(workspaces: &Workspaces, chat: &AgentState, language: Language) -> ActionOutcome {
    let plugins: Vec<Value> = workspaces
        .list()
        .into_iter()
        .map(|(name, written_in)| {
            json!({
                "current": chat.plugin.as_deref() == Some(name.as_str()),
                "usable_in_this_chat": written_in == language.as_str(),
                "name": name,
                "language": written_in,
            })
        })
        .collect();
    outcome("plugin_dev_list", "ok", json!({ "plugins": plugins }))
}

fn doctor(language: Language) -> ActionOutcome {
    let readiness = language.readiness();
    let note = if readiness.ready() {
        "Everything a Rust plugin needs is on this machine.".to_string()
    } else {
        format!("Not on this machine: {}. A Rust plugin cannot be built until it is.", readiness.missing_text().join("; "))
    };
    outcome(
        "rust_doctor",
        "ok",
        json!({ "ready": readiness.ready(), "missing": readiness.missing_text(), "note": note }),
    )
}

fn tree(workspace: &Workspace) -> ActionOutcome {
    let files: Vec<Value> =
        workspace.files().into_iter().map(|entry| json!({ "path": entry.path, "bytes": entry.bytes })).collect();
    outcome("plugin_dev_tree", "ok", json!({ "plugin": workspace.name, "files": files }))
}

fn read(workspace: &Workspace, params: &Value) -> Result<ActionOutcome, AppError> {
    let path = required(params, "path")?;
    let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let (content, bytes, more) = workspace.read(&path, offset)?;
    let mut data = json!({ "path": path, "bytes": bytes, "content": content });
    if more {
        data["next_offset"] = json!(offset + content.len());
        data["note"] = json!("The file continues: read on with the same path and \"offset\": next_offset.");
    }
    Ok(outcome("plugin_dev_read", "ok", data))
}

/// The Python kit is the contract with the server: it is copied in, never
/// written by hand.
fn is_kit(path: &str) -> bool {
    path.trim().trim_start_matches("./") == python::KIT_PATH
}

const KIT_REFUSAL: &str = "engine/shiny_plugin.py is the kit that talks to the server: it is not edited. Put the \
                           plugin's code in engine/plugin.py.";

fn write(workspace: &Workspace, params: &Value) -> Result<ActionOutcome, AppError> {
    let path = required(params, "path")?;
    if is_kit(&path) {
        return Ok(refused("plugin_dev_write", KIT_REFUSAL));
    }
    let content = raw(params, "content").ok_or_else(|| AppError::BadRequest("content required".into()))?;
    let replaced = workspace.write(&path, content)?;
    Ok(outcome(
        "plugin_dev_write",
        "ok",
        json!({ "path": path, "bytes": content.len(), "replaced": replaced }),
    ))
}

fn patch(workspace: &Workspace, params: &Value) -> Result<ActionOutcome, AppError> {
    let path = required(params, "path")?;
    if is_kit(&path) {
        return Ok(refused("plugin_dev_patch", KIT_REFUSAL));
    }
    let find = raw(params, "find").ok_or_else(|| AppError::BadRequest("find required".into()))?;
    let replace = raw(params, "replace").ok_or_else(|| AppError::BadRequest("replace required".into()))?;
    let all = params.get("all").and_then(Value::as_bool).unwrap_or(false);
    let replaced = workspace.patch(&path, find, replace, all)?;
    Ok(outcome("plugin_dev_patch", "ok", json!({ "path": path, "replaced": replaced })))
}

fn delete(workspace: &Workspace, params: &Value) -> Result<ActionOutcome, AppError> {
    let path = required(params, "path")?;
    if is_kit(&path) {
        return Ok(refused("plugin_dev_delete", KIT_REFUSAL));
    }
    workspace.delete(&path)?;
    Ok(outcome("plugin_dev_delete", "ok", json!({ "path": path, "deleted": true })))
}

/// The plugin a command names, else the one this chat is working on.
fn named(chat: &AgentState, params: &Value) -> Option<String> {
    text(params, "name").or_else(|| text(params, "plugin")).or_else(|| chat.plugin.clone())
}

/// Why the builder does not touch an installed plugin, if it does not: the
/// Craft apps are the store's, and the plugins that manage the system itself
/// ship with the server.
fn off_limits(builder: &Builder<'_>, name: &str) -> Option<String> {
    use crate::plugins::loader::PluginRuntime;
    let plugins = &builder.state.plugins;
    if crate::plugins::store::CraftApp::by_id(name).is_some() || plugins.kind_for(name).is_framed() {
        return Some(format!("'{name}' is an app from the store: the plugin builder does not change those."));
    }
    if plugins.is_installed(name) && plugins.loader().runtime_for(name) != PluginRuntime::Process {
        return Some(format!("'{name}' is part of the system itself and is not changed from here."));
    }
    None
}

/// Make a workspace out of an installed plugin, so the chat can change it.
async fn open(
    builder: &Builder<'_>,
    workspaces: &Workspaces,
    chat: &AgentState,
    language: Language,
    params: &Value,
) -> Result<ActionOutcome, AppError> {
    const ACTION: &str = "plugin_dev_open";
    let name = required(params, "name")?.to_ascii_lowercase();
    if let Some(reason) = off_limits(builder, &name) {
        return Ok(refused(ACTION, reason));
    }
    let workspace = if workspaces.exists(&name) {
        workspaces.open(&name)?
    } else {
        let Some(installed) = builder.state.plugins.plugin_dir_for(&name) else {
            return Ok(refused(ACTION, format!("no plugin named '{name}' is installed. list_plugins shows the installed ones.")));
        };
        workspaces.adopt(&name, &installed)?
    };
    if !workspace.prebuilt() && workspace.language() != Some(language) {
        return Ok(refused(
            ACTION,
            format!(
                "the plugin '{name}' is written in {} and this chat builds in {}. Tell the user it has to be opened in a new chat.",
                workspace.language().map_or("another language", Language::label),
                language.label()
            ),
        ));
    }
    if chat.plugin.as_deref() != Some(name.as_str()) {
        state::set_plugin(&builder.state.pool, builder.conversation_id, &builder.traveler.id, &name).await?;
    }
    let files: Vec<String> = workspace
        .files()
        .into_iter()
        .map(|entry| entry.path)
        .filter(|path| workspace.source_path(path).is_ok())
        .collect();
    let note = if workspace.prebuilt() {
        format!(
            "Opened the installed plugin '{name}'. Its engine has no sources on this machine, so its tools and routes \
             stay as they are: you can change its window (web/), its skill, its translations and its manifest. Raise \
             \"version\" in plugin.toml, then check, pack and install: the changed copy replaces the installed one \
             for this account, and plugin_dev_rollback puts the previous one back."
        )
    } else {
        format!("Opened the plugin '{name}'. Raise \"version\" in plugin.toml before you install a changed copy.")
    };
    Ok(outcome(ACTION, "ok", json!({ "plugin": name, "prebuilt": workspace.prebuilt(), "files": files, "note": note })))
}

/// Put back the copy the last install replaced (`<name>.bak`).
async fn rollback(builder: &Builder<'_>, chat: &AgentState, params: &Value) -> Result<ActionOutcome, AppError> {
    const ACTION: &str = "plugin_dev_rollback";
    let Some(name) = named(chat, params) else {
        return Ok(refused(ACTION, "name required: the plugin to put back"));
    };
    crate::plugins::loader::validate_plugin_name(&name)?;
    if let Some(reason) = off_limits(builder, &name) {
        return Ok(refused(ACTION, reason));
    }
    let state = builder.state;
    let plugins_dir = std::path::PathBuf::from(&state.config.plugins_dir);
    let current = plugins_dir.join(&name);
    let backup = plugins_dir.join(format!("{name}.bak"));
    if !backup.join("plugin.toml").is_file() {
        return Ok(refused(ACTION, format!("there is no previous version of '{name}' to put back.")));
    }
    // Swap the two copies, so a rollback can itself be rolled back.
    let aside = plugins_dir.join(format!("_staging-rollback-{name}"));
    let _ = std::fs::remove_dir_all(&aside);
    state.plugins.uninstall(&name).await;
    let swapped = (|| -> std::io::Result<()> {
        if current.exists() {
            std::fs::rename(&current, &aside)?;
        }
        std::fs::rename(&backup, &current)?;
        Ok(())
    })();
    if let Err(error) = swapped {
        if aside.exists() && !current.exists() {
            let _ = std::fs::rename(&aside, &current);
        }
        let _ = state.plugins.install_dir_static(&current, state.plugin_ctx()).await;
        return Err(AppError::Internal(format!("could not swap the plugin's copies: {error}")));
    }
    let loaded = state.plugins.install_dir_static(&current, state.plugin_ctx()).await;
    if let Err(error) = &loaded {
        // The previous copy does not load: put the newer one back.
        let _ = std::fs::rename(&current, &backup);
        if aside.exists() {
            let _ = std::fs::rename(&aside, &current);
            let _ = state.plugins.install_dir_static(&current, state.plugin_ctx()).await;
        }
        if let Some(rebuild) = &state.router_rebuild {
            rebuild();
        }
        return Ok(outcome(
            ACTION,
            "error",
            json!({ "error": format!("the previous version does not load ({error}); the current one was kept.") }),
        ));
    }
    if aside.exists() {
        let _ = std::fs::rename(&aside, &backup);
    }
    crate::plugins::installer::log_event(&plugins_dir, &format!("rollback-ok name={name}"));
    if let Some(rebuild) = &state.router_rebuild {
        rebuild();
    }
    let version = state.plugins.list().into_iter().find(|m| m.name == name).map(|m| m.version.to_string());
    Ok(outcome(
        ACTION,
        "ok",
        json!({
            "plugin": name,
            "version": version,
            "note": format!("Put back the previous version of '{name}'. The one it replaced is kept: another rollback returns to it."),
        }),
    ))
}

async fn uninstall(builder: &Builder<'_>, chat: &AgentState, params: &Value) -> Result<ActionOutcome, AppError> {
    const ACTION: &str = "plugin_dev_uninstall";
    let Some(name) = named(chat, params) else {
        return Ok(refused(ACTION, "name required: the plugin to uninstall"));
    };
    crate::plugins::loader::validate_plugin_name(&name)?;
    if let Some(reason) = off_limits(builder, &name) {
        return Ok(refused(ACTION, reason));
    }
    if !builder.state.plugins.is_installed(&name) {
        return Ok(refused(ACTION, format!("no plugin named '{name}' is installed.")));
    }
    let (removed, system_fallback) = match crate::plugins::admin_api::uninstall_plugin(builder.state, &name).await {
        Ok(done) => done,
        Err(AppError::BadRequest(reason)) => return Ok(refused(ACTION, reason)),
        Err(error) => return Err(error),
    };
    builder.state.plugin_console.clear(&name);
    let note = if system_fallback {
        format!("Removed this account's copy of '{name}'. The system's own '{name}' is in use again.")
    } else {
        format!("Uninstalled '{name}'. Its workspace (the sources) and its data tables are kept.")
    };
    Ok(outcome(ACTION, "ok", json!({ "plugin": name, "removed": removed, "system_plugin_restored": system_fallback, "note": note })))
}

async fn lint(builder: &Builder<'_>, workspaces: &Workspaces, mut workspace: Workspace) -> Result<ActionOutcome, AppError> {
    let report = rust::lint(workspaces, &mut workspace, builder.cancel).await?;
    let mut data = report.to_json();
    if let Some(map) = data.as_object_mut() {
        map.remove("built");
    }
    data["note"] = json!(match report.end {
        rust::BuildEnd::Built if report.warning_count == 0 => "No errors and no advice from the linter.".to_string(),
        rust::BuildEnd::Built => format!("No errors; {} piece(s) of advice, each with its file and line.", report.warning_count),
        rust::BuildEnd::Failed => format!("{} error(s), each with its file and line.", report.error_count),
        rust::BuildEnd::Broken => "The linter could not run: see \"detail\". If clippy is not installed, skip this step.".to_string(),
        rust::BuildEnd::Stopped => "Stopped with the turn.".to_string(),
        rust::BuildEnd::TimedOut => "Ran out of time and was stopped.".to_string(),
    });
    Ok(outcome("rust_lint", "ok", data))
}

/// The errors the plugin's window reported from the page.
fn console(builder: &Builder<'_>, workspace: &Workspace, params: &Value) -> ActionOutcome {
    let entries = builder.state.plugin_console.of(&workspace.name);
    if params.get("clear").and_then(Value::as_bool) == Some(true) {
        builder.state.plugin_console.clear(&workspace.name);
    }
    let note = if entries.is_empty() {
        "The window reported no error. It reports only while it is open in the desktop: call show_plugin first if it is closed.".to_string()
    } else {
        format!("{} error(s) reported by the window, oldest first.", entries.len())
    };
    outcome("plugin_dev_console", "ok", json!({ "plugin": workspace.name, "errors": entries, "note": note }))
}

/// Largest answer of a tool or a route that is handed back as it is.
const TRY_OUTPUT_BYTES: usize = 8 * 1024;

fn clip(text: &str) -> (String, bool) {
    if text.len() <= TRY_OUTPUT_BYTES {
        return (text.to_string(), false);
    }
    let mut end = TRY_OUTPUT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

/// Call one of the installed plugin's own tools, as the assistant would.
async fn call(builder: &Builder<'_>, workspace: &Workspace, params: &Value) -> Result<ActionOutcome, AppError> {
    const ACTION: &str = "plugin_dev_call";
    let tool = required(params, "tool")?;
    let registry = builder.state.plugins.tools();
    if !registry.has(&tool) || registry.owner_of(&tool) != workspace.name {
        return Ok(refused(
            ACTION,
            format!(
                "'{tool}' is not a tool of the installed plugin '{}'. Its tools: {}.",
                workspace.name,
                registry.names_owned_by(&workspace.name).join(", ")
            ),
        ));
    }
    let arguments = params.get("params").cloned().unwrap_or_else(|| json!({}));
    let context: shiny_plugin_sdk::context::AgentContext = serde_json::from_value(
        json!({ "lat": null, "lon": null, "heading": null, "lang": "en", "ollama_model": null }),
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;
    let os_home = builder.state.os_identity_for(builder.traveler).map(|identity| identity.home);
    let user = &builder.traveler.id;
    let result = registry
        .invoke(&tool, &builder.state.plugin_ctx(), user, user, &arguments, &context, os_home.as_deref())
        .await;
    Ok(match result {
        Ok(done) => {
            let (data, clipped) = clip(&done.data.to_string());
            outcome(ACTION, "ok", json!({ "tool": tool, "result": done.result, "data": data, "clipped": clipped }))
        }
        Err(error) => outcome(
            ACTION,
            "ok",
            json!({ "tool": tool, "result": "error", "error": error.to_string(), "note": format!("{tool} failed: {error}") }),
        ),
    })
}

/// Call one of the installed plugin's own routes, as its window would.
async fn http(builder: &Builder<'_>, workspace: &Workspace, params: &Value) -> Result<ActionOutcome, AppError> {
    use tower::ServiceExt;
    const ACTION: &str = "plugin_dev_http";
    let path = required(params, "path")?;
    let method = text(params, "method").unwrap_or_else(|| "GET".into()).to_ascii_uppercase();
    let base = format!("/api/{}", workspace.name);
    let bare = path.split(['?', '#']).next().unwrap_or_default();
    if (bare != base && !bare.starts_with(&format!("{base}/"))) || bare.contains("..") {
        return Ok(refused(ACTION, format!("only this plugin's own routes can be called: paths under {base}/")));
    }
    let Ok(method) = axum::http::Method::from_bytes(method.as_bytes()) else {
        return Ok(refused(ACTION, "method must be GET, POST, PUT, PATCH or DELETE"));
    };
    let Some(token) = builder.traveler.auth_token.as_deref() else {
        return Ok(refused(ACTION, "this account has no session token to call the route with"));
    };
    let body = params.get("body").filter(|body| !body.is_null()).map(Value::to_string).unwrap_or_default();
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(&path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .map_err(|e| AppError::BadRequest(format!("invalid request: {e}")))?;
    // The request is made from the server itself.
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 0))));
    let response = crate::api::build_router(builder.state.clone())
        .oneshot(request)
        .await
        .map_err(|e| AppError::Internal(format!("the route could not be called: {e}")))?;
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .map_err(|e| AppError::Internal(format!("the route's answer could not be read: {e}")))?;
    let (body, clipped) = clip(&String::from_utf8_lossy(&bytes));
    Ok(outcome(ACTION, "ok", json!({ "status": status, "body": body, "clipped": clipped })))
}

/// What the checks need to know about the rest of the system.
fn surroundings(builder: &Builder<'_>, name: &str) -> checks::Surroundings {
    let registry = builder.state.plugins.tools();
    let foreign_tools: BTreeMap<String, String> = registry
        .list()
        .into_iter()
        .map(|tool| {
            let owner = registry.owner_of(&tool);
            (tool, owner)
        })
        .filter(|(_, owner)| owner != name)
        .collect();
    checks::Surroundings {
        foreign_tools,
        installed_dir: builder.state.plugins.plugin_dir_for(name),
        overrides_system: builder.state.plugins.is_system_only(name),
        is_craft_app: crate::plugins::store::CraftApp::by_id(name).is_some(),
    }
}

/// One check (`check_web`, …) or all of them (`check_all`).
fn check(
    builder: &Builder<'_>,
    mut workspace: Workspace,
    action: &str,
    only: Option<&str>,
) -> Result<ActionOutcome, AppError> {
    let report = checks::run(&workspace, only, &surroundings(builder, &workspace.name)).map_err(AppError::BadRequest)?;
    // Only the whole set, passed, lets the plugin be packed.
    let whole = only.is_none();
    if whole {
        workspace.meta.checked = report.ok().then(|| workspace.sources_digest());
        workspace.save_meta()?;
    }
    let errors = report.findings.iter().filter(|finding| finding.error).count();
    let mut data = report.to_json();
    data["note"] = json!(match (report.ok(), whole) {
        (true, true) => "Every check passed on the current sources.".to_string(),
        (true, false) => "This check passed. Run check_all before packing.".to_string(),
        (false, _) => format!("{errors} error(s) to fix, each with its file. Fix them and check again."),
    });
    Ok(outcome(action, "ok", data))
}

async fn build(
    builder: &Builder<'_>,
    workspaces: &Workspaces,
    mut workspace: Workspace,
) -> Result<ActionOutcome, AppError> {
    let report = rust::build(workspaces, &mut workspace, builder.cancel).await?;
    let mut data = report.to_json();
    data["note"] = json!(match report.end {
        rust::BuildEnd::Built => format!("Built in {} s.", report.seconds),
        rust::BuildEnd::Failed => format!(
            "The compiler found {} error(s), each with its file and line. Fix them and build again.",
            report.error_count
        ),
        rust::BuildEnd::Broken =>
            "The build could not start: see \"detail\" for cargo's own message. Do not retry unchanged.".to_string(),
        rust::BuildEnd::Stopped => "The build was stopped with the turn. Nothing was built.".to_string(),
        rust::BuildEnd::TimedOut => "The build ran out of time and was stopped. Nothing was built.".to_string(),
    });
    Ok(outcome("rust_build", "ok", data))
}

async fn py_check(mut workspace: Workspace) -> Result<ActionOutcome, AppError> {
    let mut data = python::check(&mut workspace).await?;
    let errors = data["errors"].as_array().map_or(0, Vec::len);
    data["note"] = json!(if errors == 0 {
        "The engine's sources are valid Python and use the standard library only. The plugin can be tested and packed.".to_string()
    } else {
        format!("{errors} error(s) to fix, each with its file and line. Fix them and check again.")
    });
    Ok(outcome("py_check", "ok", data))
}

/// Start the plugin's engine as the server would, without installing it: the
/// handshake says whether it starts at all and what it declares. Nothing of
/// it is called, and it is stopped at once.
async fn dry_run(builder: &Builder<'_>, workspace: &Workspace, action: &str) -> Result<ActionOutcome, AppError> {
    let fresh = workspace.prebuilt() || workspace.meta.built.as_deref() == Some(workspace.sources_digest().as_str());
    if !fresh || crate::plugins::process::find_executable(&workspace.dir, &workspace.name).is_none() {
        return Ok(refused(
            action,
            "the engine is not built from the current sources. Build it (or run the language's check) first.",
        ));
    }
    let manifest: shiny_plugin_sdk::manifest::Manifest =
        match std::fs::read_to_string(workspace.dir.join("plugin.toml")).map_err(|e| e.to_string()).and_then(|text| {
            toml::from_str(&text).map_err(|e: toml::de::Error| e.message().to_string())
        }) {
            Ok(manifest) => manifest,
            Err(error) => return Ok(refused(action, format!("plugin.toml cannot be read: {error}. Run check_manifest."))),
        };
    let started = crate::plugins::process::ProcessPlugin::start(
        &workspace.dir,
        manifest,
        &builder.state.config.snapshot(),
    )
    .await;
    let plugin = match started {
        Ok(plugin) => plugin,
        Err(error) => {
            return Ok(outcome(
                action,
                "ok",
                json!({
                    "started": false,
                    "error": error.to_string(),
                    "note": "The engine did not start. plugin_dev_logs shows what it printed; fix it and test again.",
                }),
            ));
        }
    };
    let (tools, routes) = plugin.declared();
    plugin.supervisor().shutdown().await;
    drop(plugin);

    // What the static checks read from the sources, the handshake confirms.
    let registry = builder.state.plugins.tools();
    let ident = workspace.name.replace('-', "_");
    let base = format!("/api/{}", workspace.name);
    let mut problems: Vec<String> = Vec::new();
    for tool in &tools {
        if !tool.starts_with(&format!("{ident}_")) {
            problems.push(format!("the tool \"{tool}\" must be named \"{ident}_…\""));
        }
        if registry.has(tool) && registry.owner_of(tool) != workspace.name {
            problems.push(format!("the tool \"{tool}\" already belongs to the plugin \"{}\"", registry.owner_of(tool)));
        }
    }
    for route in &routes {
        let path = route.split_once(' ').map_or(route.as_str(), |(_, path)| path);
        if path != base && !path.starts_with(&format!("{base}/")) {
            problems.push(format!("the route \"{route}\" must be under \"{base}/\""));
        }
    }
    let note = if problems.is_empty() {
        format!("The engine starts and declares {} tool(s) and {} route(s).", tools.len(), routes.len())
    } else {
        format!("The engine starts, but {} thing(s) have to be fixed before it can be installed.", problems.len())
    };
    Ok(outcome(
        action,
        "ok",
        json!({ "started": true, "ok": problems.is_empty(), "tools": tools, "routes": routes, "errors": problems, "note": note }),
    ))
}

fn pack(mut workspace: Workspace) -> Result<ActionOutcome, AppError> {
    let packed = match package::pack(&mut workspace) {
        Ok(packed) => packed,
        Err(AppError::BadRequest(reason)) => return Ok(refused("plugin_dev_pack", reason)),
        Err(error) => return Err(error),
    };
    Ok(outcome(
        "plugin_dev_pack",
        "ok",
        json!({
            "plugin": workspace.name,
            "version": packed.version,
            "archive": packed.file,
            "sha256": packed.sha256,
            "note": format!("Packed version {} as {}. It can be installed.", packed.version, packed.file),
        }),
    ))
}

async fn install(builder: &Builder<'_>, workspace: Workspace) -> Result<ActionOutcome, AppError> {
    const ACTION: &str = "plugin_dev_install";
    let (packed, bytes) = match package::packed_archive(&workspace) {
        Ok(found) => found,
        Err(AppError::BadRequest(reason)) => return Ok(refused(ACTION, reason)),
        Err(error) => return Err(error),
    };
    let state = builder.state;
    let base_ctx = state.plugin_ctx();
    let plugins_dir = std::path::PathBuf::from(&state.config.plugins_dir);
    let manager = state.plugins.clone();
    let rebuild = state.router_rebuild.clone();
    // A stopped turn must not cancel a half-published install: once started
    // it finishes on its own, router included.
    let installed = tokio::spawn(async move {
        let name = crate::plugins::installer::install_archive(bytes, &plugins_dir, &manager, base_ctx).await?;
        if let Some(rebuild) = rebuild {
            rebuild();
        }
        Ok::<_, AppError>(name)
    })
    .await
    .map_err(|error| AppError::Internal(format!("Plugin installation task failed: {error}")))?;
    let name = match installed {
        Ok(name) => name,
        // The installer's own words say what is wrong with the archive.
        Err(error) => return Ok(outcome(ACTION, "error", json!({ "error": error.to_string() }))),
    };
    state.plugins.set_enabled_for(&builder.traveler.id, &name, true).await?;
    let tools = state.plugins.tools().names_owned_by(&name);
    Ok(outcome(
        ACTION,
        "ok",
        json!({
            "plugin": name,
            "version": packed.version,
            "tools": tools,
            "note": format!(
                "Installed and turned on '{name}' {}. Its tools work from the next step. Call show_plugin to open \
                 its window if the user wants to see it, and plugin_dev_status to confirm it is running.",
                packed.version
            ),
        }),
    ))
}

fn status(builder: &Builder<'_>, workspace: &Workspace) -> ActionOutcome {
    let plugins = &builder.state.plugins;
    let name = &workspace.name;
    let sources = workspace.sources_digest();
    let fresh = |recorded: &Option<String>| recorded.as_deref() == Some(sources.as_str());
    let installed = plugins.list().into_iter().find(|manifest| &manifest.name == name);
    let supervisor = plugins.loader().supervisor_for(name);
    let data = json!({
        "plugin": name,
        "sources": {
            "checked": fresh(&workspace.meta.checked),
            "built": fresh(&workspace.meta.built),
            "packed": workspace.meta.packed.as_ref().is_some_and(|packed| packed.sources == sources),
            "packed_version": workspace.meta.packed.as_ref().map(|packed| packed.version.clone()),
        },
        "installed": installed.as_ref().map(|manifest| json!({
            "version": manifest.version.to_string(),
            "running": supervisor.as_ref().is_some_and(|supervisor| supervisor.is_running()),
            "pid": supervisor.as_ref().and_then(|supervisor| supervisor.pid()),
            "tools": plugins.tools().names_owned_by(name),
            "routes": plugins.routes().into_iter().filter(|(owner, _, _)| owner == name)
                .map(|(_, spec, _)| format!("{} {}", spec.method.as_str(), spec.path)).collect::<Vec<_>>(),
        })),
    });
    outcome("plugin_dev_status", "ok", data)
}

/// How much of the server's log is searched for a plugin's lines.
const LOG_TAIL_BYTES: u64 = 512 * 1024;

fn tail(path: &std::path::Path, bytes: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let length = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(length.saturating_sub(bytes)));
    let mut buffer = Vec::new();
    let _ = file.read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).into_owned()
}

/// The last lines the plugin wrote (the server logs them as `[<name>] …`)
/// and the last install events that name it.
fn logs(builder: &Builder<'_>, workspace: &Workspace, params: &Value) -> ActionOutcome {
    let wanted = params.get("lines").and_then(Value::as_u64).unwrap_or(40).clamp(1, 200) as usize;
    let name = &workspace.name;
    let last = |text: String, matches: &dyn Fn(&str) -> bool| -> Vec<String> {
        let lines: Vec<&str> = text.lines().filter(|line| matches(line)).collect();
        lines[lines.len().saturating_sub(wanted)..]
            .iter()
            .map(|line| line.chars().take(600).collect())
            .collect()
    };
    let marker = format!("[{name}]");
    let quoted = format!("'{name}'");
    let spaced = format!("plugin {name}");
    let output = last(tail(std::path::Path::new(&builder.state.config.log_file), LOG_TAIL_BYTES), &|line| {
        line.contains(&marker) || line.contains(&quoted) || line.contains(&spaced)
    });
    let install_log = std::path::Path::new(&builder.state.config.plugins_dir).join("install.log");
    let installs = last(tail(&install_log, LOG_TAIL_BYTES), &|line| line.contains(name.as_str()));
    outcome("plugin_dev_logs", "ok", json!({ "plugin": name, "output": output, "install_log": installs }))
}
