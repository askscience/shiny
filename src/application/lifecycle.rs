//! Runtime-owned workers. Dropping a worker set aborts only tasks we started;
//! detached speech daemons and external system services are not owned here.
#[derive(Default)]
pub struct Workers(Vec<tokio::task::JoinHandle<()>>);

impl Workers {
    pub fn track(&mut self, task: Option<tokio::task::JoinHandle<()>>) {
        if let Some(task) = task {
            self.0.push(task);
        }
    }

    pub async fn shutdown(&mut self) {
        for task in &self.0 {
            task.abort();
        }
        for task in self.0.drain(..) {
            if let Err(error) = task.await {
                if !error.is_cancelled() {
                    tracing::warn!("Background worker failed: {error}");
                }
            }
        }
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

/// Both terminal Ctrl-C and systemd's SIGTERM take the same shutdown path.
pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    result = tokio::signal::ctrl_c() => {
                        if let Err(error) = result { tracing::error!("Signal handler failed: {error}"); }
                    }
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => {
                tracing::error!("SIGTERM handler failed: {error}");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn shutdown_aborts_and_joins_only_owned_workers() {
        let mut workers = Workers::default();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        workers.track(Some(tokio::spawn(async move {
            let _tx = tx;
            std::future::pending::<()>().await;
        })));
        tokio::task::yield_now().await;
        workers.shutdown().await;
        assert!(rx.await.is_err());
        workers.shutdown().await;
    }
}
