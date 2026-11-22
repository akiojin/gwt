//! Issue #7702: bound one turn of GUI-thread work so a stalled event loop
//! cannot hold WebView2, and therefore IME, for a second or more.

use std::time::{Duration, Instant};

use super::{AppRuntime, OutboundEvent, ProjectContext};

/// Upper bound for one slice of cooperative GUI work. Observed stalls were
/// 369–1323ms (FrontendReady, inventory, PM ensure, LoadBoard).
pub(crate) const GUI_EVENT_LOOP_SLICE_BUDGET: Duration = Duration::from_millis(100);

const _: () = assert!(GUI_EVENT_LOOP_SLICE_BUDGET.as_millis() < 1000);

pub(crate) struct GuiSliceBudget {
    started: Instant,
    limit: Duration,
    processed: usize,
}

impl GuiSliceBudget {
    pub(crate) fn production() -> Self {
        Self::new(production_limit())
    }

    /// Process one item, then stop. `Duration::ZERO` fails the second check
    /// without sleeping, so yield tests do not depend on the wall clock.
    #[cfg(test)]
    pub(crate) fn exhausted() -> Self {
        Self::new(Duration::ZERO)
    }

    fn new(limit: Duration) -> Self {
        Self {
            started: Instant::now(),
            limit,
            processed: 0,
        }
    }

    pub(crate) fn allow_another(&self) -> bool {
        self.processed == 0 || self.started.elapsed() < self.limit
    }

    pub(crate) fn record(&mut self) {
        self.processed = self.processed.saturating_add(1);
    }
}

fn production_limit() -> Duration {
    // Existing handler tests use `production()` and expect the whole queue in
    // one call. The binary built without `cfg(test)` keeps the 100ms slice.
    #[cfg(test)]
    {
        Duration::from_secs(60)
    }
    #[cfg(not(test))]
    {
        GUI_EVENT_LOOP_SLICE_BUDGET
    }
}

pub(crate) struct PendingFrontendSnapshots {
    client_id: String,
    context: ProjectContext,
    window_ids: Vec<String>,
}

impl AppRuntime {
    pub(crate) fn queue_gui_slice_continue(&mut self) {
        if self.gui_slice_continue_queued {
            return;
        }
        self.gui_slice_continue_queued = true;
        self.proxy.send(crate::UserEvent::GuiSliceContinue);
    }

    pub(crate) fn continue_gui_slice(&mut self) -> Vec<OutboundEvent> {
        self.gui_slice_continue_queued = false;
        let mut budget = GuiSliceBudget::production();
        let mut events = self.startup_pm_ensure_slice(&mut budget);
        events.extend(self.emit_deferred_frontend_snapshots(&mut budget));
        if self.gui_slice_has_remainder() {
            self.queue_gui_slice_continue();
        }
        events
    }

    /// Keep the ids that fit in `budget`. The rest wait for a fresh budget on
    /// [`Self::continue_gui_slice`], which does not spend a record on the view.
    pub(crate) fn take_frontend_ready_snapshot_ids(
        &mut self,
        client_id: &str,
        context: &ProjectContext,
        mut window_ids: Vec<String>,
        budget: &mut GuiSliceBudget,
    ) -> Vec<String> {
        let mut ready = Vec::new();
        while let Some(id) = window_ids.pop() {
            if !budget.allow_another() {
                window_ids.push(id);
                break;
            }
            budget.record();
            ready.push(id);
        }
        self.store_deferred_frontend_snapshots(client_id, context, window_ids);
        ready
    }

    fn store_deferred_frontend_snapshots(
        &mut self,
        client_id: &str,
        context: &ProjectContext,
        window_ids: Vec<String>,
    ) {
        self.pending_frontend_ready_snapshots
            .retain(|pending| pending.client_id != client_id);
        if window_ids.is_empty() {
            return;
        }
        self.pending_frontend_ready_snapshots
            .push(PendingFrontendSnapshots {
                client_id: client_id.to_string(),
                context: context.clone(),
                window_ids,
            });
        self.queue_gui_slice_continue();
    }

    fn emit_deferred_frontend_snapshots(
        &mut self,
        budget: &mut GuiSliceBudget,
    ) -> Vec<OutboundEvent> {
        let mut events = Vec::new();
        let pending = std::mem::take(&mut self.pending_frontend_ready_snapshots);
        let mut kept = Vec::new();
        for mut batch in pending {
            if !self.project_context_is_current(&batch.context) {
                continue;
            }
            let client_id = batch.client_id.clone();
            while let Some(id) = batch.window_ids.pop() {
                if !budget.allow_another() {
                    batch.window_ids.push(id);
                    break;
                }
                budget.record();
                events.extend(self.request_owner_snapshot(&id, Some(client_id.clone())));
            }
            if !batch.window_ids.is_empty() {
                kept.push(batch);
            }
        }
        self.pending_frontend_ready_snapshots = kept;
        events
    }

    fn gui_slice_has_remainder(&self) -> bool {
        !self.pending_startup_pm_tabs.is_empty()
            || !self.pending_frontend_ready_snapshots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::GuiSliceBudget;

    #[test]
    fn exhausted_budget_runs_one_item_then_stops() {
        let mut budget = GuiSliceBudget::exhausted();
        assert!(budget.allow_another());
        budget.record();
        assert!(!budget.allow_another());
    }
}
