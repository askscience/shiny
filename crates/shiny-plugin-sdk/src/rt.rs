//! Runtime bridge for plugin code.
//!
//! A plugin cdylib statically links its *own* copy of Tokio and sqlx/libsqlite3.
//! Plugin futures must therefore run on a runtime the plugin owns — not the
//! host's — and, critically, on a **single thread**: SQLite values are not
//! thread-safe, so a `fetch_*` future that migrates across Tokio worker threads
//! can `sqlite3_value_free` on a different thread than it was created,
//! segfaulting the process.
//!
//! `bridge` sends each future to a dedicated thread that drives a
//! single-threaded (`current_thread`) runtime; the host awaits the result over
//! an executor-agnostic oneshot channel. The host worker never blocks; the
//! plugin's own Tokio services its IO serially on one thread.
//!
//! The worker and its runtime are **process-global** — one thread shared by
//! every bridged tool/route of every loaded plugin — so a panicking job must
//! never take the worker down: each job is awaited under `catch_unwind`, the
//! loop keeps running, and the awaiting host receives an `AppError` instead of
//! a panic or a hang.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use futures::FutureExt;
use tokio::sync::mpsc;

use crate::errors::AppError;

type Job = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The process-global sender queueing jobs onto the single plugin worker
/// thread. Every `bridge` call — from any plugin — queues here and is
/// serviced serially.
fn plugin_tx() -> &'static mpsc::UnboundedSender<Job> {
    static TX: OnceLock<mpsc::UnboundedSender<Job>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .thread_name("shiny-plugin-rt")
                .build()
                .expect("failed to build plugin runtime");
            rt.block_on(async move {
                while let Some(job) = rx.recv().await {
                    // Isolate panics: without this, one panicking plugin
                    // future would unwind the worker thread, drop the
                    // channel receiver, and wedge every later bridge call.
                    if let Err(panic) =
                        FutureExt::catch_unwind(std::panic::AssertUnwindSafe(job)).await
                    {
                        let msg = panic
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown panic".into());
                        tracing::error!("plugin runtime job panicked: {msg}");
                    }
                }
            });
        });
        tx
    })
}

/// Run `fut` on the plugin-owned, single-threaded runtime; the returned future
/// may be awaited from any executor (no Tokio context required on the caller
/// side). Returns `Err` — never panics or hangs — when the job panics or the
/// plugin runtime worker is gone.
pub async fn bridge<F, T>(fut: F) -> Result<T, AppError>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = futures::channel::oneshot::channel();
    let job: Job = Box::pin(async move {
        let _ = tx.send(fut.await);
    });
    if plugin_tx().send(job).is_err() {
        return Err(AppError::Internal(
            "plugin runtime is unavailable (worker thread exited)".into(),
        ));
    }
    rx.await.map_err(|_| {
        AppError::Internal("plugin tool panicked before returning a result".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bridge_returns_value() {
        let v: u32 = bridge(async { 41 + 1 }).await.expect("value");
        assert_eq!(v, 42);
    }

    #[tokio::test]
    async fn bridge_survives_panicking_job() {
        let first = bridge(async { panic!("boom") }).await;
        assert!(first.is_err(), "panicking job must surface an error");
        // The worker must still be alive for subsequent jobs.
        let second: u32 = bridge(async { 7 }).await.expect("worker alive");
        assert_eq!(second, 7);
    }
}
