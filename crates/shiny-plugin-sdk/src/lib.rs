//! shiny Plugin SDK
//!
//! Defines the trait surface a plugin implements, the types it exchanges with the
//! core AI assistant, and a small set of helpers (parsing, parameter extraction,
//! migration runner). Plugins compile against this crate only; the binary loads
//! them via `libloading` at startup or on install.
//!
//! See `PLUGINS.md` at the repo root for the full authoring guide.

pub mod errors;
pub mod services;
pub mod artifacts;
pub mod navigation;
pub mod context;
pub mod outcome;
pub mod notification;
pub mod manifest;
pub mod rt;
pub mod tools;
pub mod routes;
pub mod crons;
pub mod migrations;
pub mod plugin;
pub mod odt;
pub mod ods;
pub mod odp;
pub mod upload;
pub mod db;

pub use errors::AppError;
pub use services::{OllamaClient, SearchService, SupertonicClient};
pub use artifacts::{Artifact, PlanDay, PlanDayItem, RouteMeta};
pub use navigation::NavigationSession;
pub use context::AgentContext;
pub use outcome::ActionOutcome;
pub use notification::{Notification, NotificationAction};
pub use manifest::Manifest;
pub use tools::{bridged, BridgedTool, Tool, ToolRequest, RegistryBuilder, ParamHelpers, parse_actions, strip_action_blocks, normalize_action_name};
pub use routes::{RouteSpec, HttpMethod, RouteHandler, bridged_route, UserId, TravelerId, user_id_from_request, traveler_id_from_request, path_params_from_request, USER_ID_HEADER, TRAVELER_ID_HEADER, PATH_PARAMS_HEADER, REMOTE_HEADER, is_remote_request, OsIdentity, os_identity_from_request, os_home_from_request, OS_USER_HEADER, OS_HOME_HEADER, OS_UID_HEADER};
pub use crons::{CronSpec, CronEntry};
pub use plugin::{Plugin, PluginEntry, PLUGIN_ENTRY_SYMBOL};
pub use odp::Slide;
pub use db::{Db, Value as DbValue};
pub use upload::field_bytes_capped;

/// The core API level. Plugins declare `api_level` in `plugin.toml`; the loader
/// refuses to load a plugin built against a newer API than the running core.
pub const CORE_API_LEVEL: u32 = 1;

/// Hard cap on the decompressed size of an ODF `content.xml`.
///
/// ODF documents are ZIP archives, and a tiny `content.xml` entry can inflate
/// to gigabytes (a zip bomb). The codecs live inside the host process, so an
/// unbounded decompression aborts the whole app; 32 MiB is far above any real
/// document's `content.xml`.
pub(crate) const MAX_ODF_CONTENT_XML: usize = 32 * 1024 * 1024;

/// Read one ZIP entry as UTF-8 with a hard uncompressed-size cap.
///
/// Shared by the ODT/ODS/ODP codecs so every document path enforces the same
/// limit. The entry's declared size is checked first, then the actual read is
/// capped as well (the declared size can lie).
pub(crate) fn read_zip_text_capped(
    bytes: &[u8],
    entry: &str,
    max: usize,
) -> Result<String, errors::AppError> {
    use std::io::Read;

    let reader = std::io::Cursor::new(bytes.to_vec());
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| errors::AppError::Internal(format!("not a valid ODF archive: {e}")))?;
    let mut file = archive
        .by_name(entry)
        .map_err(|e| errors::AppError::Internal(format!("missing {entry}: {e}")))?;
    if file.size() as usize > max {
        return Err(errors::AppError::BadRequest(format!(
            "{entry} is too large to open"
        )));
    }
    let mut buf = Vec::new();
    file.by_ref()
        .take(max as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| errors::AppError::Internal(format!("failed to read {entry}: {e}")))?;
    if buf.len() > max {
        return Err(errors::AppError::BadRequest(format!(
            "{entry} expands beyond the size limit"
        )));
    }
    String::from_utf8(buf)
        .map_err(|e| errors::AppError::Internal(format!("{entry} is not valid UTF-8: {e}")))
}