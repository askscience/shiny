//! Runtime bridge for plugin code.
//!
//! A plugin cdylib statically links its *own* copy of Tokio — its thread-local
//! runtime context is not the host's. If host code polls a plugin future that
//! touches sqlx/reqwest/tokio-time directly, the plugin's Tokio TLS is empty
//! and the process aborts ("this functionality requires a Tokio context").
//!
//! The fix: drive plugin futures on a runtime the plugin owns, and hand the
//! caller an executor-agnostic `futures::channel::oneshot` to await. The host
//! worker never blocks; the plugin copy of Tokio services its own IO.

use std::future::Future;
use std::sync::OnceLock;

fn plugin_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("shiny-plugin-rt")
            .build()
            .expect("failed to build plugin runtime")
    })
}

/// Run `fut` on the plugin-owned runtime; the returned future may be awaited
/// from any executor (no Tokio context required on the caller side).
pub async fn bridge<F, T>(fut: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = futures::channel::oneshot::channel();
    plugin_runtime().spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.await.expect("plugin runtime task dropped")
}
