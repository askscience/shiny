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
//! plugin's own Tokio services its IO on one thread, with the jobs running as
//! concurrent tasks there (a slow job never holds up the others).
//!
//! The worker and its runtime belong to **this plugin's copy of the SDK** (each
//! cdylib links the SDK statically), so all bridged tools/routes of one plugin
//! share one thread while different plugins run independently. A panicking job
//! must never take the worker down: each job is awaited under `catch_unwind`,
//! the loop keeps running, and the awaiting host receives an `AppError` instead
//! of a panic or a hang.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use futures::FutureExt;
use tokio::sync::mpsc;

use crate::errors::AppError;

type Job = Pin<Box<dyn Future<Output = ()> + Send>>;

/// This plugin's sender queueing jobs onto its single worker thread. Every
/// `bridge` call from this plugin queues here; the jobs run concurrently as
/// tasks of that one thread. Other plugins have their own copy of this queue.
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
                    // Each job is a task on this one thread: jobs interleave at
                    // their await points instead of queueing behind each other.
                    // Awaiting each in turn meant one slow route (a news fetch
                    // waiting on the network for seconds) held up every other
                    // route of the plugin, down to saving a bookmark. Still one
                    // thread, so nothing SQLite-related ever crosses threads.
                    tokio::spawn(async move {
                        // Isolate panics: a panicking job is logged and its
                        // awaiting caller gets an error; the worker keeps going.
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
                    });
                }
            });
        });
        tx
    })
}

/// Queue `fut` on the plugin-owned runtime and return immediately.
///
/// Use this from lifecycle hooks (`on_load`) and any other code the host
/// invokes directly: those run on the *host's* executor, so the plugin's own
/// reactor is not installed on the calling thread and a bare `tokio::spawn`
/// panics with *"there is no reactor running"* — and, unwinding across the
/// `dlopen` boundary, aborts the whole process. The job is serviced by the
/// same serial worker as [`bridge`], and a panicking job is logged without
/// wedging later jobs.
pub fn spawn<F>(fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let job: Job = Box::pin(fut);
    if plugin_tx().send(job).is_err() {
        tracing::error!("plugin runtime is unavailable; background job dropped");
    }
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

    /// A job waiting on another job must not wedge the plugin: with jobs
    /// awaited one after another this never completes.
    #[tokio::test]
    async fn jobs_run_concurrently() {
        let (tx, rx) = tokio::sync::oneshot::channel::<u32>();
        let waiting = tokio::spawn(bridge(async move { rx.await.unwrap_or(0) }));
        // Let the waiting job reach the worker first.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        bridge(async move {
            let _ = tx.send(5);
        })
        .await
        .expect("second job runs while the first waits");
        let value = tokio::time::timeout(std::time::Duration::from_secs(2), waiting)
            .await
            .expect("the first job finished")
            .unwrap()
            .unwrap();
        assert_eq!(value, 5);
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
