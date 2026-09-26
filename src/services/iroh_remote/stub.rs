//! Stub Iroh service for builds without the `iroh` feature.
//!
//! Keeps `AppState` and the `/api/remote/*` routes compiling: `status` reports
//! disabled and `start`/`rotate` fail with a clear message.

use serde::Serialize;

use crate::errors::AppError;

#[derive(Clone, Serialize)]
pub struct IrohStatus {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    pub paired: usize,
    pub pairing: bool,
}

impl IrohStatus {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ticket: None,
            paired: 0,
            pairing: false,
        }
    }
}

#[derive(Clone, Default)]
pub struct IrohRemote;

impl IrohRemote {
    pub fn new() -> Self {
        Self
    }

    pub async fn start(&self, _port: u16) -> Result<IrohStatus, AppError> {
        Err(AppError::BadRequest(
            "this build was compiled without iroh support".into(),
        ))
    }

    pub async fn stop(&self) -> Result<(), AppError> {
        Ok(())
    }

    pub async fn rotate(&self, _port: u16) -> Result<IrohStatus, AppError> {
        Err(AppError::BadRequest(
            "this build was compiled without iroh support".into(),
        ))
    }

    pub fn begin_pairing(&self) -> u64 {
        0
    }

    pub fn unpair_all(&self) -> usize {
        0
    }

    pub async fn status(&self) -> IrohStatus {
        IrohStatus::disabled()
    }
}
