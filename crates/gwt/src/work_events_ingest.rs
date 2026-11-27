//! Runtime scheduling for the shared project Work event intake engine.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::app_runtime::ProjectContext;

pub(crate) use gwt::project_work_events::ingest_project_work_events_paths_with_inventory;

// Preserve the binary runtime tests' paths-injected entry point across the
// library extraction; Cargo does not enable cfg(test) on library dependencies.
#[cfg(test)]
pub(crate) fn ingest_project_work_events_paths(
    project_root: &std::path::Path,
    work_items_path: &std::path::Path,
    state_path: &std::path::Path,
) -> gwt::project_work_events::WorkEventsIngestSummary {
    ingest_project_work_events_paths_with_inventory(project_root, work_items_path, state_path, None)
}

/// One worker and one coalesced follow-up per project, including forced opens.
#[derive(Default)]
pub(crate) struct WorkEventsIngestQueue {
    pending: Mutex<HashMap<gwt_core::repo_hash::ProjectKey, Option<ProjectContext>>>,
}

pub(crate) struct WorkEventsIngestWorker {
    queue: Arc<WorkEventsIngestQueue>,
    project: gwt_core::repo_hash::ProjectKey,
    active: bool,
}

impl WorkEventsIngestQueue {
    pub(crate) fn request(
        self: &Arc<Self>,
        context: ProjectContext,
    ) -> Option<WorkEventsIngestWorker> {
        let project = context.project_key.clone();
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(requested) = pending.get_mut(&project) {
            *requested = Some(context);
            return None;
        }
        pending.insert(project.clone(), None);
        Some(WorkEventsIngestWorker {
            queue: Arc::clone(self),
            project,
            active: true,
        })
    }
}

impl WorkEventsIngestWorker {
    pub(crate) fn finish_pass(&mut self) -> Option<ProjectContext> {
        let mut pending = self
            .queue
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(context) = pending.get_mut(&self.project).and_then(Option::take) {
            Some(context)
        } else {
            pending.remove(&self.project);
            self.active = false;
            None
        }
    }
}

impl Drop for WorkEventsIngestWorker {
    fn drop(&mut self) {
        if self.active {
            self.queue
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&self.project);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn project_ingest_coalesces_bursts_and_releases_failed_workers() {
        let queue = Arc::new(WorkEventsIngestQueue::default());
        let project = gwt_core::repo_hash::ProjectKey::parse("0123456789abcdef").unwrap();
        let other = gwt_core::repo_hash::ProjectKey::parse("fedcba9876543210").unwrap();
        let context = |project: &gwt_core::repo_hash::ProjectKey, generation| {
            crate::app_runtime::ProjectContext {
                tab_id: format!("tab-{generation}"),
                project_key: project.clone(),
                generation,
                project_root: PathBuf::from(format!("root-{generation}")),
            }
        };
        let mut worker = queue.request(context(&project, 0)).expect("first worker");
        for generation in 1..=100 {
            assert!(queue.request(context(&project, generation)).is_none());
        }
        let independent = queue
            .request(context(&other, 0))
            .expect("independent project");
        assert_eq!(
            worker.finish_pass(),
            Some(context(&project, 100)),
            "one follow-up preserves the latest reopened tab, root and generation"
        );
        assert!(
            worker.finish_pass().is_none(),
            "the burst never becomes 100 workers"
        );
        let replacement = queue
            .request(context(&project, 101))
            .expect("completed admission");
        drop(worker);
        assert!(
            queue.request(context(&project, 102)).is_none(),
            "old guard cannot release replacement"
        );
        drop(replacement);
        let failed = queue
            .request(context(&project, 103))
            .expect("new admission");
        drop(failed);
        assert!(
            queue.request(context(&project, 104)).is_some(),
            "failure releases admission"
        );
        drop(independent);
    }
}
