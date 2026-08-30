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

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use tokio::sync::mpsc;

type Job = Pin<Box<dyn Future<Output = ()> + Send>>;

/// One `UnboundedSender` per plugin: every `bridge` call queues a job onto the
/// plugin's single worker thread.
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
                    job.await;
                }
            });
        });
        tx
    })
}

/// Run `fut` on the plugin-owned, single-threaded runtime; the returned future
/// may be awaited from any executor (no Tokio context required on the caller
/// side).
pub async fn bridge<F, T>(fut: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = futures::channel::oneshot::channel();
    let job: Job = Box::pin(async move {
        let _ = tx.send(fut.await);
    });
    plugin_tx()
        .send(job)
        .expect("plugin runtime thread dropped");
    rx.await.expect("plugin runtime task dropped")
}
