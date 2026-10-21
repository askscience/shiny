//! Short, human-readable failure text from child-process output and logs.

use std::path::Path;

/// The most useful last line of a failed command: the stdout tail when it
/// mentions an error, else the stderr tail.
pub(crate) fn last_error_line(stdout: &[u8], stderr: &[u8]) -> String {
    let out = String::from_utf8_lossy(stdout);
    let err = String::from_utf8_lossy(stderr);
    out.lines()
        .last()
        .filter(|l| l.to_lowercase().contains("error"))
        .or_else(|| err.lines().last())
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "Download failed".into())
}

/// The last three meaningful lines of a render log, joined for one message.
pub(crate) fn last_log_error(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let tail: Vec<&str> = text
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(3)
        .collect();
    if tail.is_empty() {
        return "no output captured".into();
    }
    tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
}
