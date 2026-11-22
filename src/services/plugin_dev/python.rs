//! The plugin builder's Python module: what the machine needs, the Python
//! half of a new plugin, and the check that stands in for a build.
//!
//! A Python plugin is `engine/plugin.py` plus the kit it imports
//! (`engine/shiny_plugin.py`, which speaks the plugin protocol). Nothing is
//! compiled: once the sources pass `py_check`, the launcher `bin/<name>` is
//! written and the plugin can be packed.

use std::process::Stdio;

use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use super::workspace::{fill, TemplateValues, Workspace};
use super::Readiness;
use crate::errors::AppError;
use crate::services::ask::AskLine;

/// The kit: shipped with the server and copied into every Python plugin, so
/// an installed plugin carries the kit it was written against.
pub const KIT: &str = include_str!("templates/python/engine/shiny_plugin.py");
pub const KIT_PATH: &str = "engine/shiny_plugin.py";
const ENGINE: &str = include_str!("templates/python/engine/plugin.py");
const LAUNCHER: &str = include_str!("templates/python/launcher.sh");

/// Python plugins need the interpreter and nothing else: they use the
/// standard library only. The LiteAura package depends on it, so on an
/// installed system it is always there.
pub fn readiness() -> Readiness {
    assess(super::rust::on_path("python3", &[]))
}

fn assess(interpreter: bool) -> Readiness {
    let mut missing = Vec::new();
    if !interpreter {
        missing.push(AskLine::keyed("chat.ask.missing.python", "Python 3 (part of the LiteAura package)"));
    }
    Readiness { missing, fetchable: false }
}

/// Write the Python half of a new plugin: the kit and the engine, with one
/// example tool pair and two routes over the template's table.
pub fn scaffold(workspace: &Workspace, values: &TemplateValues) -> Result<(), AppError> {
    workspace.write(KIT_PATH, KIT)?;
    workspace.write("engine/plugin.py", &fill(ENGINE, &values.pairs()))?;
    Ok(())
}

/// The fixed program `py_check` runs: it parses every engine file and lists
/// what it imports. It only reads; nothing of the plugin is executed.
const CHECKER: &str = r#"
import ast, json, os, sys
root = sys.argv[1]
errors, local = [], set()
files = []
for folder, _dirs, names in os.walk(os.path.join(root, "engine")):
    for name in sorted(names):
        if name.endswith(".py"):
            path = os.path.join(folder, name)
            files.append(path)
            local.add(os.path.splitext(name)[0])
stdlib = set(getattr(sys, "stdlib_module_names", ()))
for path in files:
    shown = os.path.relpath(path, root)
    try:
        with open(path, encoding="utf-8") as handle:
            tree = ast.parse(handle.read(), filename=shown)
    except SyntaxError as error:
        errors.append({"file": shown, "line": error.lineno or 0, "message": "syntax error: " + (error.msg or "")})
        continue
    except (OSError, UnicodeDecodeError, ValueError) as error:
        errors.append({"file": shown, "line": 0, "message": str(error)})
        continue
    for node in ast.walk(tree):
        names = []
        if isinstance(node, ast.Import):
            names = [alias.name for alias in node.names]
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            names = [node.module]
        for name in names:
            top = name.split(".")[0]
            if top in local or (stdlib and top in stdlib) or not stdlib:
                continue
            errors.append({"file": shown, "line": node.lineno, "message":
                "import of '" + top + "': only the Python standard library and the plugin's own files are available"})
print(json.dumps({"files": len(files), "errors": errors}))
"#;

/// Check the engine's sources and, when they pass, write the launcher.
pub async fn check(workspace: &mut Workspace) -> Result<Value, AppError> {
    let mut errors: Vec<Value> = Vec::new();
    match std::fs::read_to_string(workspace.dir.join(KIT_PATH)) {
        Ok(kit) if kit == KIT => {}
        Ok(_) => errors.push(json!({
            "file": KIT_PATH,
            "line": 0,
            "message": "the kit was changed. It is not edited: it is restored now, put your code in engine/plugin.py",
        })),
        Err(_) => errors.push(json!({ "file": KIT_PATH, "line": 0, "message": "the kit is missing: it is restored now" })),
    }
    if !errors.is_empty() {
        workspace.write(KIT_PATH, KIT)?;
    }
    if !workspace.dir.join("engine/plugin.py").is_file() {
        errors.push(json!({ "file": "engine/plugin.py", "line": 0, "message": "engine/plugin.py is missing: it is the plugin's engine" }));
    }

    // -I: isolated from the user's environment and site-packages; -B: no
    // bytecode files in the workspace.
    let mut child = tokio::process::Command::new("python3")
        .args(["-I", "-B", "-", &workspace.dir.to_string_lossy()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| AppError::Internal(format!("could not start python3: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(CHECKER.as_bytes()).await;
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| AppError::Internal("the Python check ran out of time".into()))?
        .map_err(|e| AppError::Internal(format!("the Python check failed: {e}")))?;
    let report: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        AppError::Internal(format!(
            "the Python check gave no result: {}",
            String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default()
        ))
    })?;
    errors.extend(report.get("errors").and_then(Value::as_array).cloned().unwrap_or_default());

    let ok = errors.is_empty();
    if ok {
        let launcher = crate::plugins::process::executable_path(&workspace.dir, &workspace.name);
        if let Some(parent) = launcher.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&launcher, fill(LAUNCHER, &[("NAME", workspace.name.as_str())]))?;
        crate::plugins::process::ensure_executable(&launcher)?;
        workspace.meta.built = Some(workspace.sources_digest());
    } else {
        workspace.meta.built = None;
    }
    workspace.save_meta()?;
    Ok(json!({ "ok": ok, "files": report.get("files").cloned().unwrap_or(json!(0)), "errors": errors }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::plugin_dev::workspace::{scaffold_common, NewPlugin, Workspaces};
    use crate::services::plugin_dev::Language;

    #[test]
    fn python_needs_only_the_interpreter() {
        assert!(assess(true).ready());
        let without = assess(false);
        assert_eq!(without.missing.len(), 1);
        assert!(!without.fetchable, "it comes with the package, it is not ours to install");
    }

    fn scaffolded() -> (tempfile::TempDir, Workspace) {
        let root = tempfile::tempdir().unwrap();
        let workspace = Workspaces::at(root.path().join("plugin-dev")).create("notes", Language::Python).unwrap();
        let values = TemplateValues::of(&NewPlugin {
            name: "notes".into(),
            title: "Notes".into(),
            description: "Quick notes".into(),
            category: "Office".into(),
        });
        scaffold_common(&workspace, &values).unwrap();
        scaffold(&workspace, &values).unwrap();
        (root, workspace)
    }

    #[tokio::test]
    async fn the_template_passes_and_gets_its_launcher() {
        let (_root, mut workspace) = scaffolded();
        let report = check(&mut workspace).await.unwrap();
        assert_eq!(report["ok"], true, "{report:#}");
        assert_eq!(report["files"], 2);
        let launcher = workspace.dir.join("bin/notes");
        assert!(std::fs::read_to_string(&launcher).unwrap().contains("engine/plugin.py"));
        use std::os::unix::fs::PermissionsExt;
        assert!(std::fs::metadata(&launcher).unwrap().permissions().mode() & 0o111 != 0);
        assert_eq!(workspace.meta.built, Some(workspace.sources_digest()));
        assert!(!workspace.dir.join("engine/__pycache__").exists(), "the check leaves no bytecode behind");
    }

    #[tokio::test]
    async fn syntax_errors_and_foreign_imports_are_named_with_their_line() {
        let (_root, mut workspace) = scaffolded();
        workspace.write("engine/plugin.py", "import requests\nimport json\nfrom helper import x\n\ndef broken(:\n").unwrap();
        workspace.write("engine/helper.py", "import numpy.linalg\nx = 1\n").unwrap();
        let report = check(&mut workspace).await.unwrap();
        assert_eq!(report["ok"], false);
        let lines: Vec<String> = report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| format!("{}:{} {}", e["file"].as_str().unwrap(), e["line"], e["message"].as_str().unwrap()))
            .collect();
        assert!(lines.iter().any(|l| l.starts_with("engine/plugin.py:5 syntax error")), "{lines:#?}");
        assert!(lines.iter().any(|l| l.starts_with("engine/helper.py:1 import of 'numpy'")), "{lines:#?}");
        assert!(!workspace.dir.join("bin/notes").exists(), "no launcher for sources that do not pass");
        assert_eq!(workspace.meta.built, None);

        // Fixed: the standard library and the plugin's own files are fine.
        workspace.write("engine/plugin.py", "import json\nfrom helper import x\nfrom shiny_plugin import Plugin\n").unwrap();
        workspace.write("engine/helper.py", "import sqlite3\nx = 1\n").unwrap();
        assert_eq!(check(&mut workspace).await.unwrap()["ok"], true);
    }

    #[tokio::test]
    async fn the_kit_is_restored_when_it_was_touched() {
        let (_root, mut workspace) = scaffolded();
        std::fs::write(workspace.dir.join(KIT_PATH), "print('mine')\n").unwrap();
        let report = check(&mut workspace).await.unwrap();
        assert_eq!(report["ok"], false);
        assert!(report["errors"][0]["message"].as_str().unwrap().contains("the kit was changed"));
        assert_eq!(std::fs::read_to_string(workspace.dir.join(KIT_PATH)).unwrap(), KIT);
        assert_eq!(check(&mut workspace).await.unwrap()["ok"], true, "restored, it passes");
    }
}
