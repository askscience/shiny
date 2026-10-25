//! Client for the privileged installer helper.
//!
//! The plugin runs as the live user. It can only send a typed JSON request to
//! `/run/shiny/installer.sock`; it never runs a shell and never touches a disk.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use shiny_installer_protocol::{RpcRequest, RpcResponse};
use shiny_plugin_sdk::errors::AppError;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;

fn socket_path() -> PathBuf {
    std::env::var_os("SHINY_INSTALLER_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/shiny/installer.sock"))
}

pub async fn call(method: &str, params: Value) -> Result<Value, AppError> {
    let path = socket_path();
    let stream = timeout(Duration::from_secs(5), UnixStream::connect(&path))
        .await
        .map_err(|_| {
            AppError::Internal(format!(
                "installer helper did not answer at {}",
                path.display()
            ))
        })?
        .map_err(|error| {
            AppError::Internal(format!(
                "installer helper is not running at {}: {error}",
                path.display()
            ))
        })?;

    let (reader, mut writer) = stream.into_split();
    let request = RpcRequest::new(uuid::Uuid::new_v4().to_string(), method, params);
    let mut line = serde_json::to_vec(&request).map_err(|error| {
        AppError::Internal(format!("could not encode installer request: {error}"))
    })?;
    line.push(b'\n');
    writer.write_all(&line).await.map_err(|error| {
        AppError::Internal(format!("could not send installer request: {error}"))
    })?;
    writer.flush().await.map_err(|error| {
        AppError::Internal(format!("could not flush installer request: {error}"))
    })?;

    let mut lines = BufReader::new(reader).lines();
    let reply = timeout(Duration::from_secs(60), lines.next_line())
        .await
        .map_err(|_| AppError::Internal("installer helper timed out".into()))?
        .map_err(|error| AppError::Internal(format!("could not read installer reply: {error}")))?;
    let Some(reply) = reply else {
        return Err(AppError::Internal(
            "installer helper closed the connection".into(),
        ));
    };

    let response: RpcResponse = serde_json::from_str(&reply)
        .map_err(|error| AppError::Internal(format!("invalid installer reply: {error}")))?;
    if !response.ok {
        return Err(AppError::Internal(
            response
                .error
                .unwrap_or_else(|| "installer helper returned an error".into()),
        ));
    }
    Ok(response.data.unwrap_or(Value::Null))
}
