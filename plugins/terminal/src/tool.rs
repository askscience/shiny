//! `terminal_exec` — run a shell command in the Terminal.
//!
//! Commands run in a session the tool owns ([`pty::agent_for`]) — never the
//! window's session, so nothing can be typed into whatever the user (or a CLI
//! they are running there) is doing. The shell persists across calls, so
//! `cd`/environment changes carry over. Output is collected until the shell
//! goes quiet, or `MAX_WAIT` elapses and the command keeps running.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::pty::{self, Event};

/// Output counts as complete once the shell has been quiet this long.
const QUIET: Duration = Duration::from_millis(400);
/// Longest a single call waits before reporting back with what it has.
const MAX_WAIT: Duration = Duration::from_secs(10);
/// When the echo of the typed command is not visible (readline redrew a long
/// line, or the shell runs with echo off), any bytes count as output after
/// this grace period.
const ECHO_GRACE: Duration = Duration::from_millis(300);
/// Cap on the cleaned output returned to the model.
const MAX_OUTPUT_CHARS: usize = 16_000;
/// Only the start of the capture can hold the command echo.
const ECHO_WINDOW: usize = 8 * 1024;

const DOC: &str = "- `terminal_exec` — Run a shell command on this machine and return its output. params: `{ command: string }`. Commands run in a persistent shell of their own, so `cd`/environment changes carry over between calls. Prefer short, non-interactive commands (`ls`, `cat`, `grep`, `df`, `git status`); don't use it for editors, pagers or REPLs (`vim`, `htop`, `python`). A command still running after ~10s returns its output so far and keeps running.";

pub struct TerminalExec;

#[async_trait]
impl Tool for TerminalExec {
    fn name(&self) -> &str {
        "terminal_exec"
    }

    fn aliases(&self) -> &[&str] {
        &["terminal", "terminal_run", "shell", "run_command"]
    }

    fn step_label(&self) -> &str {
        "Running a shell command…"
    }

    fn doc_fragment(&self) -> Option<&str> {
        Some(DOC)
    }

    fn humanize(&self, _result: &str, data: &Value) -> String {
        let command = data
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("command");
        let short: String = command.chars().take(60).collect();
        if data.get("timed_out").and_then(|v| v.as_bool()).unwrap_or(false) {
            format!("`{short}` is still running")
        } else {
            format!("Ran `{short}`")
        }
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        // The model may spell it `command` or `cmd` depending on the plugin.
        let Some(command) = req
            .params
            .param_str("command")
            .or_else(|| req.params.param_str("cmd"))
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
        else {
            return Ok(ActionOutcome::error(
                "terminal_exec",
                "No command given — pass {\"command\": \"…\"}, e.g. {\"command\": \"ls\"}. To just show the Terminal window use show_plugin.",
            ));
        };

        let session = pty::agent_for(req.user_id)?;

        // Subscribe before typing so the capture starts at the write, and the
        // echo of the command is the first thing the stream carries.
        let (_scrollback, mut events, alive) = session.subscribe();
        if !alive {
            return Err(AppError::Internal("the terminal session is not running".into()));
        }

        let mut line = command.clone();
        if !line.ends_with('\n') {
            line.push('\n');
        }
        session.write_input(line.as_bytes())?;

        let capture = capture(&mut events, &command).await;
        let output = truncate_middle(
            &clean_output(after_echo(&capture.bytes, &command)),
            MAX_OUTPUT_CHARS,
        );

        Ok(ActionOutcome::ok(
            "terminal_exec",
            json!({
                "command": command,
                "output": output,
                "timed_out": capture.timed_out,
            }),
        ))
    }
}

struct Capture {
    bytes: Vec<u8>,
    timed_out: bool,
}

/// Collect the session's output until the shell goes quiet (or `MAX_WAIT`).
async fn capture(events: &mut UnboundedReceiver<Event>, command: &str) -> Capture {
    let started = Instant::now();
    let mut bytes: Vec<u8> = Vec::new();
    let mut last_output: Option<Instant> = None;
    let mut timed_out = false;

    loop {
        let elapsed = started.elapsed();
        if elapsed >= MAX_WAIT {
            timed_out = true;
            break;
        }
        let mut wait = MAX_WAIT - elapsed;
        if let Some(last) = last_output {
            let quiet_left = QUIET.saturating_sub(last.elapsed());
            if quiet_left.is_zero() {
                break;
            }
            wait = wait.min(quiet_left);
        }

        match tokio::time::timeout(wait, events.next()).await {
            Ok(Some(Event::Out(encoded))) => {
                if let Ok(chunk) =
                    base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes())
                {
                    let arrived = !chunk.is_empty();
                    bytes.extend_from_slice(&chunk);
                    if arrived
                        && (last_output.is_some()
                            || has_real_output(&bytes, command, started.elapsed()))
                    {
                        last_output = Some(Instant::now());
                    }
                }
            }
            Ok(Some(Event::Exit { .. })) => break,
            // Ready / Ping carry no output.
            Ok(Some(_)) => {}
            // The session's feed closed.
            Ok(None) => break,
            // Tick: re-evaluate the quiet/deadline conditions.
            Err(_) => {}
        }
    }

    Capture { bytes, timed_out }
}

/// Where the shell's echo of the typed command ends, if it is visible in the
/// capture. `None` while the echo hasn't arrived (or the shell has echo off).
fn echo_end(acc: &[u8], command: &str) -> Option<usize> {
    let needle = command.as_bytes();
    if needle.is_empty() {
        return None;
    }
    let window = &acc[..acc.len().min(ECHO_WINDOW)];
    if window.len() < needle.len() {
        return None;
    }
    window
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|pos| pos + needle.len())
}

/// True once the capture holds something past the shell's echo of the typed
/// command — the command's own output or the next prompt.
fn has_real_output(acc: &[u8], command: &str, since_write: Duration) -> bool {
    if acc.is_empty() {
        return false;
    }
    match echo_end(acc, command) {
        Some(end) => acc[end..].iter().any(|b| !b.is_ascii_whitespace()),
        None => since_write >= ECHO_GRACE,
    }
}

/// The command's output: everything the session emitted after the echo of the
/// typed command, so the echo and any pre-command banner drop out.
fn after_echo<'a>(acc: &'a [u8], command: &str) -> &'a [u8] {
    match echo_end(acc, command) {
        Some(end) => &acc[end..],
        None => acc,
    }
}

/// Turn raw PTY bytes into text for the model: lossy UTF-8, ANSI/OSC
/// sequences removed, CRLF normalised, control characters dropped.
fn clean_output(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes);
    let stripped = strip_ansi(&raw);
    let mut out = String::with_capacity(stripped.len());
    let mut chars = stripped.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    out.push('\n');
                }
            }
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

/// Remove ANSI/VT control sequences (CSI, OSC, DCS, two-char escapes) so the
/// model reads text, not terminal choreography.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') | Some('P') | Some('^') | Some('_') => {
                chars.next();
                for c in chars.by_ref() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            _ => {
                // Two-byte escape (ESC c) or a sequence with intermediate
                // bytes (ESC ( B, ESC # 8): consume intermediates, then the
                // final byte.
                if matches!(chars.peek(), Some(c) if ('\u{20}'..='\u{2f}').contains(c)) {
                    for c in chars.by_ref() {
                        if ('\u{30}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
        }
    }
    out
}

/// Keep the head and the tail when the output is longer than `max` chars —
/// results usually start at the top, errors at the bottom.
fn truncate_middle(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let half = max / 2;
    let head: String = text.chars().take(half).collect();
    let tail: String = text.chars().skip(count - half).collect();
    format!("{head}\n… [output truncated] …\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_end_finds_the_command() {
        let acc = b"Last login: today\r\neev@box:~$ ls\r\n";
        assert_eq!(echo_end(acc, "ls"), Some(acc.len() - "\r\n".len()));
        assert_eq!(echo_end(acc, "top"), None);
    }

    #[test]
    fn after_echo_drops_the_banner_and_echo() {
        let acc = b"Last login: today\r\neev@box:~$ ls\r\nfile1  file2\r\neev@box:~$ ";
        let tail = String::from_utf8(after_echo(acc, "ls").to_vec()).expect("utf8");
        assert_eq!(tail, "\r\nfile1  file2\r\neev@box:~$ ");
    }

    #[test]
    fn echo_alone_is_not_real_output() {
        let acc = b"eev@box:~$ ls\r\n";
        assert!(!has_real_output(acc, "ls", Duration::from_millis(10)));
        let with_output = b"eev@box:~$ ls\r\nfile1\r\n";
        assert!(has_real_output(with_output, "ls", Duration::from_millis(10)));
    }

    #[test]
    fn missing_echo_falls_back_to_the_grace_period() {
        let acc = b"partial";
        assert!(!has_real_output(acc, "a-much-longer-command", Duration::from_millis(50)));
        assert!(has_real_output(acc, "a-much-longer-command", ECHO_GRACE));
    }

    #[test]
    fn strip_ansi_removes_csi_and_osc() {
        let input = "\u{1b}[31mred\u{1b}[0m \u{1b}]0;title\u{7}plain\u{1b}(B";
        assert_eq!(strip_ansi(input), "red plain");
    }

    #[test]
    fn clean_output_normalises_newlines_and_drops_controls() {
        assert_eq!(clean_output(b"\r\nfile1\r\nfile2\x07\r\n"), "file1\nfile2");
        assert_eq!(clean_output(b"progress\rdone"), "progress\ndone");
    }

    #[test]
    fn truncate_middle_keeps_head_and_tail() {
        let s: String = "a".repeat(MAX_OUTPUT_CHARS + 500);
        let t = truncate_middle(&s, MAX_OUTPUT_CHARS);
        assert!(t.len() < s.len());
        assert!(t.starts_with("aaaa"));
        assert!(t.ends_with("aaaa"));
        assert!(t.contains("[output truncated]"));
    }

    fn test_ctx() -> std::sync::Arc<PluginCtx> {
        use shiny_plugin_sdk::manifest::Manifest;
        use shiny_plugin_sdk::services::ConfigSnapshot;

        let config = ConfigSnapshot {
            server_host: "127.0.0.1".into(),
            server_port: 0,
            database_url: "sqlite::memory:".into(),
            ollama_url: "http://127.0.0.1:1".into(),
            ollama_model: "test".into(),
            supertonic_url: "http://127.0.0.1:1".into(),
            supertonic_voice: "test".into(),
            web_dir: "web".into(),
            vosk_models_dir: "vosk".into(),
            auto_start_supertonic: false,
            log_level: "info".into(),
            plugins_dir: "plugins".into(),
            system_plugins_dir: None,
            admin_token: None,
        };
        let manifest = Manifest {
            name: "terminal".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: shiny_plugin_sdk::plugin::PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: None,
            author: None,
            summary: None,
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        };
        PluginCtx::new(config, manifest)
    }

    /// End to end through a real PTY: spawn the shell, type `echo`, read the
    /// output back.
    #[cfg(unix)]
    #[tokio::test]
    async fn runs_a_command_in_an_agent_shell() {
        use shiny_plugin_sdk::context::AgentContext;

        let ctx = test_ctx();
        let params = json!({ "command": "echo shiny-terminal-exec-ok" });
        let agent_ctx = AgentContext {
            lat: None,
            lon: None,
            heading: None,
            lang: "en".into(),
            ollama_model: None,
            workspaces_enabled: true,
        };
        let req = ToolRequest {
            user_id: "tool-test-user",
            traveler_id: "tool-test-user",
            params: &params,
            ctx: &agent_ctx,
            os_home: None,
        };

        let outcome = TerminalExec.invoke(&ctx, req).await.expect("terminal_exec");
        assert_eq!(outcome.result, "ok");
        let output = outcome
            .data
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            output.contains("shiny-terminal-exec-ok"),
            "expected the echo output, got: {output:?}"
        );
        assert_eq!(
            outcome.data.get("timed_out").and_then(|v| v.as_bool()),
            Some(false)
        );

        // Close the shell this test started so it does not linger.
        if let Ok(session) = crate::pty::agent_for("tool-test-user") {
            let _ = session.close();
        }
    }
}
