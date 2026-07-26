//! AppError lives in the SDK so plugins and the binary share one type.
//! Re-exported here to keep existing `crate::errors::AppError` imports working.

pub use shiny_plugin_sdk::errors::AppError;