//! Injectable, bounded subprocess execution for short host commands. Streaming
//! processes (PTYs and pactl subscribe) retain their explicit lifecycle owners.
use async_trait::async_trait;
use std::{process::Output, time::Duration};

#[derive(Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    pub timeout: Duration,
}

#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, spec: CommandSpec) -> std::io::Result<Output>;
}

pub struct SystemCommandRunner;
#[async_trait]
impl CommandRunner for SystemCommandRunner {
    async fn run(&self, spec: CommandSpec) -> std::io::Result<Output> {
        let started = std::time::Instant::now();
        let mut command = tokio::process::Command::new(&spec.program);
        command
            .args(&spec.args)
            .envs(spec.env.iter().map(|(key, value)| (key, value)))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().expect("stdout pipe configured");
        let stderr = child.stderr.take().expect("stderr pipe configured");
        let work = async {
            let (stdout, stderr, status) = tokio::try_join!(
                read_limited(stdout, 4 * 1024 * 1024),
                read_limited(stderr, 128 * 1024),
                child.wait(),
            )?;
            Ok::<_, std::io::Error>(Output {
                status,
                stdout,
                stderr,
            })
        };
        // Cancellation and timeout drop the child with kill_on_drop enabled.
        let result = tokio::time::timeout(spec.timeout, work)
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "Host command timed out")
            })?;
        tracing::debug!(program = %spec.program, elapsed_ms = started.elapsed().as_millis(),
            success = result.as_ref().map(|output| output.status.success()).unwrap_or(false), "Host command completed");
        result
    }
}

async fn read_limited(
    reader: impl tokio::io::AsyncRead + Unpin,
    limit: u64,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut output = Vec::new();
    reader.take(limit + 1).read_to_end(&mut output).await?;
    if output.len() as u64 > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Host command output limit exceeded",
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn output_limit_is_enforced_without_discarding_the_boundary_byte() {
        assert_eq!(read_limited(&b"abc"[..], 3).await.unwrap(), b"abc");
        assert_eq!(
            read_limited(&b"abcd"[..], 3).await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn slow_commands_are_bounded() {
        let result = SystemCommandRunner
            .run(CommandSpec {
                program: "/bin/sleep".into(),
                args: vec!["5".into()],
                env: Vec::new(),
                timeout: Duration::from_millis(10),
            })
            .await;
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    }
}
