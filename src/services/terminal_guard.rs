//! Hard-coded safety guard for the assistant's `terminal_exec` tool.
//!
//! Every command the assistant wants to run is approved by the user first, and
//! "Allow for this chat" can make that automatic for one conversation. A small
//! set of commands is never allowed, not even then: they destroy the system
//! itself — the root tree, a disk, the machine's power state — which no
//! conversation-level consent can undo. Deleting files and folders is
//! deliberately **not** on the list: that is ordinary work, already covered by
//! the per-command approval.
//!
//! The guard reads a shell command line, so it is a heuristic, not a sandbox:
//! it tokenizes the line (quotes, separators, redirections) and inspects each
//! simple command, including the command string handed to `sh -c`, `eval` or
//! `xargs`. Being conservative here only ever costs the user a blocked command.

/// Paths whose removal is a system-level loss, no matter what else is allowed.
/// Sub-paths are ordinary files and folders (`/etc/nginx/nginx.conf` is a
/// config file, not the system), so only the roots themselves are listed.
const SYSTEM_ROOTS: &[&str] = &[
    "/", "/boot", "/etc", "/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/libx32",
    "/sys", "/proc", "/dev", "/var", "/var/lib", "/home", "/root",
];

/// Programs that partition or reformat storage.
const DISK_PROGRAMS: &[&str] = &[
    "wipefs", "blkdiscard", "mkswap", "fdisk", "sfdisk", "gdisk", "sgdisk", "cfdisk", "parted",
];

/// Programs that take the machine down (the app, and the user's session, die
/// with it). Power actions belong to the power panel, not to a shell command.
const POWER_PROGRAMS: &[&str] = &["shutdown", "reboot", "poweroff", "halt", "kexec"];

/// Files that must never be overwritten through a redirect.
const CRITICAL_FILES: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
    "/etc/gshadow",
    "/etc/group",
    "/etc/sudoers",
    "/etc/fstab",
    "/etc/crypttab",
];

/// Why a command must not run at all, or `None` when it may be offered to the
/// user for approval.
pub fn block_reason(command: &str) -> Option<&'static str> {
    check(command, 0)
}

fn check(command: &str, depth: usize) -> Option<&'static str> {
    if depth > 3 {
        return None;
    }
    let lower = command.to_lowercase();

    // Classic fork bomb, with or without the spaces.
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains(":(){:|:&};:")
        || (compact.contains(":(){") && compact.contains("|:&") && compact.contains("};:"))
    {
        return Some("it is a fork bomb");
    }
    if lower.contains("--no-preserve-root") {
        return Some("it disables the root filesystem's safety check (--no-preserve-root)");
    }

    let segments = tokenize(&lower);
    // A `cd /etc && rm -rf *` would otherwise look like a harmless relative
    // delete: follow the working directory each simple command sets, so a
    // relative target is resolved against it.
    let mut cwd: Option<String> = None;
    for tokens in &segments {
        if let Some(reason) = check_segment(tokens, depth, cwd.as_deref()) {
            return Some(reason);
        }
        if let Some(reason) = check_redirection(tokens) {
            return Some(reason);
        }
        if let Some(dir) = cd_target(tokens) {
            cwd = Some(dir.to_string());
        }
    }
    None
}

fn check_segment(tokens: &[String], depth: usize, cwd: Option<&str>) -> Option<&'static str> {
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if is_assignment(token)
            || matches!(
                token.as_str(),
                "sudo" | "doas" | "env" | "command" | "nohup" | "time" | "exec" | "builtin"
            )
        {
            i += 1;
            continue;
        }
        break;
    }
    let program = basename(tokens.get(i)?);
    let args = &tokens[i + 1..];

    if program.starts_with("mkfs") || DISK_PROGRAMS.contains(&program) {
        return Some("it repartitions or reformats a disk");
    }
    if POWER_PROGRAMS.contains(&program) {
        return Some("it powers off or reboots the machine");
    }
    if program == "systemctl"
        && args.iter().any(|a| {
            matches!(
                a.as_str(),
                "poweroff" | "reboot" | "halt" | "shutdown" | "kexec" | "soft-reboot"
            )
        })
    {
        return Some("it powers off or reboots the machine");
    }
    if program == "loginctl" && args.iter().any(|a| matches!(a.as_str(), "poweroff" | "reboot")) {
        return Some("it powers off or reboots the machine");
    }
    if matches!(program, "init" | "telinit") && args.iter().any(|a| a == "0" || a == "6") {
        return Some("it powers off or reboots the machine");
    }

    // `dd` and `cp` may only write to a device in a specific position: `of=`
    // for dd, the destination for cp. Reading *from* a disk is a backup, not a
    // destructive write, so it stays allowed.
    if program == "dd"
        && args
            .iter()
            .any(|a| a.strip_prefix("of=").is_some_and(is_disk_device))
    {
        return Some("it writes directly to a disk device");
    }
    if program == "cp" {
        if let Some(destination) = args.iter().rev().find(|a| !a.starts_with('-')) {
            if is_disk_device(destination) {
                return Some("it writes directly to a disk device");
            }
        }
    }
    if program == "tee" && args.iter().any(|a| is_disk_device(a) || is_critical_file(a)) {
        return Some("it overwrites a disk device or a critical system file");
    }

    if program == "rm" {
        let system_cwd = cwd.filter(|dir| is_system_root(dir));
        for target in args.iter().filter(|a| !a.starts_with('-')) {
            if is_system_root(target)
                || system_cwd.is_some_and(|dir| relative_hits_system_root(dir, target))
            {
                return Some("it removes a system directory");
            }
        }
    }
    if program == "mv" {
        if let Some(source) = args.iter().find(|a| !a.starts_with('-')) {
            if is_system_root(source) {
                return Some("it moves a system directory");
            }
        }
    }
    if matches!(program, "chmod" | "chown" | "chgrp") {
        let recursive = args
            .iter()
            .any(|a| a.starts_with('-') && (a.contains('r') || a.contains('R')));
        if recursive && args.iter().any(|a| is_system_root(a)) {
            return Some("it changes permissions across a system directory");
        }
    }
    if program == "find"
        && args.iter().any(|a| a == "-delete")
        && args.iter().any(|a| is_system_root(a))
    {
        return Some("it deletes files under a system directory");
    }

    if let Some(pos) = args.iter().position(|a| a == "-c") {
        if matches!(
            program,
            "sh" | "bash" | "zsh" | "dash" | "fish" | "ash" | "ksh"
        ) {
            if let Some(script) = args.get(pos + 1) {
                if let Some(reason) = check(script, depth + 1) {
                    return Some(reason);
                }
            }
        }
    }
    if program == "eval" {
        if let Some(reason) = check(&args.join(" "), depth + 1) {
            return Some(reason);
        }
    }
    if matches!(program, "xargs" | "parallel") {
        let nested: Vec<&str> = args
            .iter()
            .map(String::as_str)
            .filter(|a| !a.starts_with('-'))
            .collect();
        if !nested.is_empty() {
            if let Some(reason) = check(&nested.join(" "), depth + 1) {
                return Some(reason);
            }
        }
    }
    if let Some(pos) = tokens
        .iter()
        .position(|t| t == "-exec" || t == "-execdir")
    {
        if let Some(reason) = check_segment(&tokens[pos + 1..], depth + 1, None) {
            return Some(reason);
        }
    }

    None
}

/// `cd`'s target in this simple command (`~` when it goes home).
fn cd_target(tokens: &[String]) -> Option<&str> {
    let mut i = 0;
    while i < tokens.len() && (is_assignment(&tokens[i]) || tokens[i] == "sudo") {
        i += 1;
    }
    if tokens.get(i).map(String::as_str) != Some("cd") {
        return None;
    }
    match tokens[i + 1..].iter().find(|a| !a.starts_with('-')) {
        Some(target) => Some(target),
        None => Some("~"),
    }
}

fn relative_hits_system_root(cwd: &str, target: &str) -> bool {
    if matches!(target, "." | ".." | "*" | "./*") {
        return true;
    }
    // A glob cannot be inspected: from a system directory it can match the
    // directory's contents, which is exactly what must not be deleted.
    if target.contains('*') || target.contains('?') || target.contains('[') {
        return true;
    }
    let base = cwd.trim_end_matches('/');
    let target = target.trim_start_matches("./");
    let resolved = if base.is_empty() {
        format!("/{target}")
    } else {
        format!("{base}/{target}")
    };
    is_system_root(&resolved)
}

fn check_redirection(tokens: &[String]) -> Option<&'static str> {
    for pair in tokens.windows(2) {
        if pair[0] != ">" && pair[0] != ">>" {
            continue;
        }
        let target = &pair[1];
        if is_disk_device(target) {
            return Some("it writes directly to a disk device");
        }
        if is_critical_file(target) {
            return Some("it overwrites a critical system file");
        }
    }
    None
}

fn tokenize(input: &str) -> Vec<Vec<String>> {
    let mut segments: Vec<Vec<String>> = Vec::new();
    let mut tokens: Vec<String> = Vec::new();
    let mut token = String::new();
    let mut quote: Option<char> = None;
    let mut chars = input.chars().peekable();

    fn end_token(token: &mut String, tokens: &mut Vec<String>) {
        if !token.is_empty() {
            tokens.push(std::mem::take(token));
        }
    }
    fn end_segment(tokens: &mut Vec<String>, segments: &mut Vec<Vec<String>>) {
        if !tokens.is_empty() {
            segments.push(std::mem::take(tokens));
        }
    }

    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                token.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            ';' | '|' | '&' | '(' | ')' | '\n' | '\r' => {
                end_token(&mut token, &mut tokens);
                end_segment(&mut tokens, &mut segments);
            }
            '>' => {
                end_token(&mut token, &mut tokens);
                if chars.peek() == Some(&'>') {
                    chars.next();
                    tokens.push(">>".into());
                } else {
                    tokens.push(">".into());
                }
            }
            '<' => {
                end_token(&mut token, &mut tokens);
                tokens.push("<".into());
            }
            c if c.is_whitespace() => end_token(&mut token, &mut tokens),
            '\\' => {
                if let Some(next) = chars.next() {
                    token.push(next);
                }
            }
            c => token.push(c),
        }
    }
    end_token(&mut token, &mut tokens);
    end_segment(&mut tokens, &mut segments);
    segments
}

fn basename(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

fn is_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_system_root(raw: &str) -> bool {
    let mut path = raw.trim().trim_matches(|c| c == '\'' || c == '"');
    if path == "/" {
        return true;
    }
    while path.ends_with('*') || path.ends_with('/') || path.ends_with('.') {
        path = &path[..path.len() - 1];
        if path == "/" {
            return true;
        }
    }
    if path.is_empty() {
        return false;
    }
    if matches!(path, "~" | "$home" | "${home}") {
        // The whole home directory, not a folder inside it.
        return true;
    }
    SYSTEM_ROOTS.contains(&path)
}

fn is_disk_device(path: &str) -> bool {
    let path = path.trim().trim_matches(|c| c == '\'' || c == '"');
    [
        "/dev/sd",
        "/dev/nvme",
        "/dev/mmcblk",
        "/dev/vd",
        "/dev/hd",
        "/dev/xvd",
        "/dev/disk/",
        "/dev/mapper/",
        "/dev/md",
        "/proc/",
        "/sys/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

fn is_critical_file(path: &str) -> bool {
    let path = path.trim().trim_matches(|c| c == '\'' || c == '"');
    CRITICAL_FILES.contains(&path)
}

#[cfg(test)]
mod tests {
    use super::block_reason;

    #[test]
    fn blocks_destroying_the_root_tree() {
        for command in [
            "rm -rf /",
            "rm -rf /*",
            "sudo rm -rf /etc",
            "rm -r --no-preserve-root /",
            "rm -rf ~",
            "rm -rf $HOME",
            "mv /etc /tmp",
            "chmod -R 000 /",
            "find / -delete",
            "cd / && rm -rf *",
            "cd / && rm -rf home",
            "cd /var && rm -rf lib",
            "cd && rm -rf *",
        ] {
            assert!(block_reason(command).is_some(), "should block: {command}");
        }
    }

    #[test]
    fn allows_deleting_ordinary_files_and_folders() {
        for command in [
            "rm -rf ./build",
            "rm -rf ~/Downloads/archive",
            "rm notes.txt",
            "rmdir empty-dir",
            "rm -rf /home/alice/project/target",
            "cd /etc && rm nginx.conf",
            "cd ~/projects && rm -rf build",
            "dd if=/dev/sda of=/tmp/backup.img",
            "cp /dev/sda /tmp/backup.img",
            "echo 'rm -rf /'",
        ] {
            assert_eq!(block_reason(command), None, "should allow: {command}");
        }
    }

    #[test]
    fn blocks_raw_disk_writes_and_partitioning() {
        for command in [
            "dd if=/dev/zero of=/dev/sda bs=1M",
            "mkfs.ext4 /dev/sdb1",
            "wipefs -a /dev/nvme0n1",
            "parted /dev/sda mklabel gpt",
            "echo x > /dev/sda",
            "cat image.iso > /dev/mmcblk0",
            "tee /dev/sda",
        ] {
            assert!(block_reason(command).is_some(), "should block: {command}");
        }
    }

    #[test]
    fn blocks_power_actions_and_critical_files() {
        for command in [
            "shutdown -h now",
            "sudo reboot",
            "systemctl poweroff",
            "init 0",
            "echo root:x:0:0 > /etc/passwd",
            "printf '' > /etc/sudoers",
        ] {
            assert!(block_reason(command).is_some(), "should block: {command}");
        }
    }

    #[test]
    fn looks_through_shell_wrappers() {
        for command in [
            "bash -c 'rm -rf /'",
            "sh -c \"dd if=/dev/zero of=/dev/sda\"",
            "eval 'rm -rf /'",
            "find /tmp -name x -exec rm -rf / ;",
            "printf x | xargs rm -rf /",
        ] {
            assert!(block_reason(command).is_some(), "should block: {command}");
        }
    }

    #[test]
    fn blocks_fork_bombs() {
        assert!(block_reason(":(){ :|:& };:").is_some());
        assert!(block_reason(":(){:|:&};:").is_some());
    }
}
