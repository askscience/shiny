//! Shell integration for the Terminal window's shells.
//!
//! The window draws command marks, command blocks and the live working
//! directory from escape sequences the shell prints around each command
//! (OSC 133 prompt/command marks, OSC 7 cwd — the same protocol Ghostty,
//! WezTerm and VS Code use). Shells do not print them on their own, so the
//! window's sessions start with small hook scripts (`plugins/terminal/shell/`):
//!
//! - **bash**: `bash --init-file <script>` — the script replays a login shell's
//!   startup files, then prepends its hook to `PROMPT_COMMAND` and appends the
//!   output mark to `PS0`.
//! - **zsh**: `ZDOTDIR=<dir> zsh -l` — the dir's `.zshenv` hands `ZDOTDIR`
//!   back, sources the user's `.zshenv` and adds `precmd`/`preexec` hooks.
//! - anything else: a plain login shell (fish ≥ 4 prints the marks itself).
//!
//! Only the window's sessions get this; the AI's `terminal_exec` shell stays a
//! plain login shell. If the scripts cannot be written the shell still starts,
//! just without marks.

use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;

const BASH_SCRIPT: &str = include_str!("../shell/bash-integration.bash");
const ZSH_ENV: &str = include_str!("../shell/zsh/.zshenv");

/// How a shell is started.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// `bash --init-file <dir>/bash-integration.bash`
    Bash,
    /// `zsh -l` with `ZDOTDIR=<dir>/zsh`
    Zsh,
    /// `<shell> -l`, untouched
    Login,
}

fn plan_for(shell: &str) -> Plan {
    match Path::new(shell).file_name().and_then(|n| n.to_str()) {
        Some("bash") => Plan::Bash,
        Some("zsh") => Plan::Zsh,
        _ => Plan::Login,
    }
}

/// Where the scripts live: the private runtime dir when there is one, else the
/// user's cache. Never a shared temp dir — another user could plant a script
/// there that every new shell would source.
fn scripts_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("shiny").join("terminal-integration"))
}

fn write_if_changed(path: &Path, content: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)
}

/// Add the arguments/environment that start `shell` with the integration.
/// Falls back to a plain login shell when the scripts cannot be written.
pub fn configure(cmd: &mut CommandBuilder, shell: &str) {
    let plan = plan_for(shell);
    let dir = match (&plan, scripts_dir()) {
        (Plan::Login, _) | (_, None) => {
            cmd.arg("-l");
            return;
        }
        (_, Some(dir)) => dir,
    };
    match plan {
        Plan::Bash => {
            let script = dir.join("bash-integration.bash");
            if write_if_changed(&script, BASH_SCRIPT).is_ok() {
                cmd.arg("--init-file");
                cmd.arg(script.as_os_str());
            } else {
                cmd.arg("-l");
            }
        }
        Plan::Zsh => {
            let zdotdir = dir.join("zsh");
            if write_if_changed(&zdotdir.join(".zshenv"), ZSH_ENV).is_ok() {
                cmd.env("ZDOTDIR", zdotdir.as_os_str());
            }
            cmd.arg("-l");
        }
        Plan::Login => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_a_plan_from_the_shell_name() {
        assert_eq!(plan_for("/bin/bash"), Plan::Bash);
        assert_eq!(plan_for("/usr/bin/zsh"), Plan::Zsh);
        assert_eq!(plan_for("/usr/bin/fish"), Plan::Login);
        assert_eq!(plan_for("/bin/sh"), Plan::Login);
    }

    #[test]
    fn scripts_print_every_mark() {
        for script in [BASH_SCRIPT, ZSH_ENV] {
            for mark in ["133;A", "133;C", "133;D;", "]7;file://"] {
                assert!(script.contains(mark), "missing {mark}");
            }
        }
    }

    #[test]
    fn bash_replays_the_login_startup_files() {
        for file in ["/etc/profile", ".bash_profile", ".bash_login", ".profile"] {
            assert!(BASH_SCRIPT.contains(file), "missing {file}");
        }
    }
}
