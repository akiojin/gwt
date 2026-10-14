//! Startup and hourly namespace GC shared by GUI and daemon lifetimes.

use std::{
    io,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, SystemTime},
};

/// Dropping the sender wakes the worker's periodic wait and ends its lifetime.
pub struct RuntimeNamespaceGcTask {
    _stop: mpsc::Sender<()>,
}

impl RuntimeNamespaceGcTask {
    pub fn start(sessions_dir: PathBuf) -> io::Result<Self> {
        Self::start_with(
            gwt_agent::runtime_gc::RUNTIME_NAMESPACE_GC_INTERVAL,
            move || {
                match gwt_agent::runtime_gc::collect_stale_runtime_namespaces(
                    &sessions_dir,
                    SystemTime::now(),
                    gwt_agent::runtime_gc::RUNTIME_NAMESPACE_MIN_AGE,
                    crate::process::host_process_liveness,
                ) {
                    Ok(report) => tracing::info!(target: "gwt::runtime_gc",
                        reclaimed = report.reclaimed, retained = report.retained,
                        reasons = ?report.reasons, "runtime namespace GC"),
                    Err(error) => {
                        // A sibling GUI/daemon may already be collecting this root.
                        if error.kind() == io::ErrorKind::WouldBlock {
                            tracing::debug!("runtime namespace GC already running");
                        } else {
                            tracing::warn!(%error, "runtime namespace GC retained uninspectable state");
                        }
                    }
                }
            },
        )
    }

    fn start_with(
        interval: Duration,
        mut collect: impl FnMut() + Send + 'static,
    ) -> io::Result<Self> {
        let (stop, wait) = mpsc::channel();
        std::thread::Builder::new()
            .name("runtime-namespace-gc".into())
            .spawn(move || loop {
                collect();
                match wait.recv_timeout(interval) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            })?;
        Ok(Self { _stop: stop })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_gc_runs_at_startup_periodically_and_stops_with_owner() {
        let (observed, events) = mpsc::channel();
        let (release, proceed) = mpsc::channel();
        let task = RuntimeNamespaceGcTask::start_with(Duration::from_secs(1), move || {
            observed.send(()).unwrap();
            proceed.recv().unwrap();
        })
        .unwrap();
        events
            .recv_timeout(Duration::from_secs(30))
            .expect("startup collection");
        release.send(()).unwrap();
        events
            .recv_timeout(Duration::from_secs(30))
            .expect("periodic collection");
        drop(task);
        release.send(()).unwrap();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(30)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }
}
