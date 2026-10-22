//! SPEC #2920 / #3287 — Tray menu actions and platform-independent model.
//!
//! The tray opens the Hub, its Open Project / Clone intake, and Projects.
//! Copy URL and Quit remain available.
//! Settings, autostart toggle, Logs, and Update controls live in the browser UI
//! Settings page (Phase 8 / FR-007).
//!
//! OS menu construction and mutation stay on the tao event-loop thread.

use std::collections::{HashMap, HashSet};

use gwt_core::repo_hash::ProjectKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayProjectEntry {
    pub project_key: ProjectKey,
    pub title: String,
    pub open: bool,
    pub running_count: usize,
    pub error_count: usize,
}

/// Bounded runtime projection; contains no filesystem paths or OS handles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraySnapshot {
    pub projects: Vec<TrayProjectEntry>,
}

impl TraySnapshot {
    pub fn new(open: Vec<TrayProjectEntry>, recent: Vec<TrayProjectEntry>) -> Self {
        let mut seen = HashSet::new();
        Self {
            projects: open
                .into_iter()
                .chain(recent)
                .filter(|entry| seen.insert(entry.project_key.clone()))
                .collect(),
        }
    }

    pub fn has_error(&self) -> bool {
        self.projects
            .iter()
            .any(|entry| entry.open && entry.error_count > 0)
    }
}

#[derive(Debug, Clone)]
pub struct TrayProjectAction {
    pub project_key: ProjectKey,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct TrayProjectItem {
    pub id: String,
    pub label: String,
}

/// Only this generation's ids resolve. A failed platform update cannot publish
/// its action map, so delayed events can never address a different project.
pub struct TrayMenuGeneration {
    sequence: u64,
    dirty: bool,
    pub snapshot: TraySnapshot,
    pub items: Vec<TrayProjectItem>,
    actions: HashMap<String, TrayProjectAction>,
}

impl TrayMenuGeneration {
    pub fn new(sequence: u64, snapshot: TraySnapshot, browser_url: &str) -> Self {
        let mut actions = HashMap::new();
        let items = snapshot
            .projects
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let id = format!("gwt.tray.project.{sequence}.{index}");
                actions.insert(
                    id.clone(),
                    TrayProjectAction {
                        project_key: entry.project_key.clone(),
                        url: format!(
                            "{}/p/{}",
                            browser_url.trim_end_matches('/'),
                            entry.project_key
                        ),
                    },
                );
                let recent = if entry.open { "" } else { " (Recent)" };
                TrayProjectItem {
                    id,
                    label: format!(
                        "{}{recent} — RUN {} / ERROR {}",
                        entry.title.replace('&', "&&"),
                        entry.running_count,
                        entry.error_count
                    ),
                }
            })
            .collect();
        Self {
            sequence,
            dirty: false,
            snapshot,
            items,
            actions,
        }
    }

    pub fn resolve(&self, id: &str) -> Option<&TrayProjectAction> {
        self.actions.get(id)
    }

    /// Apply first, then atomically publish the new snapshot and action map.
    pub fn rebuild<E>(
        &mut self,
        snapshot: TraySnapshot,
        browser_url: &str,
        apply: impl FnOnce(&Self) -> Result<(), E>,
    ) -> Result<bool, E> {
        if !self.dirty && snapshot == self.snapshot {
            return Ok(false);
        }
        // Even a failed native mutation can have emitted an event. Never reuse
        // its ids on the next attempt, whose projects may have changed.
        self.sequence = self
            .sequence
            .checked_add(1)
            .expect("tray generation exhausted");
        let next = Self::new(self.sequence, snapshot, browser_url);
        self.dirty = true;
        apply(&next)?;
        *self = next;
        Ok(true)
    }
}

/// Stable identifiers for tray menu actions. Stored as `&'static str` so
/// the Phase 4 event loop can match on tray-icon `MenuEvent::id()` without
/// stringly-typed allocations.
pub mod ids {
    pub const OPEN: &str = "gwt.tray.open";
    pub const COPY_URL: &str = "gwt.tray.copy_url";
    pub const QUIT: &str = "gwt.tray.quit";
    pub const OPEN_PROJECT: &str = "gwt.tray.open_project";
    pub const CLONE: &str = "gwt.tray.clone";
}

/// Logical menu action used by the Phase 4 event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Open,
    CopyUrl,
    Quit,
    OpenProject,
    Clone,
}

impl MenuAction {
    /// Resolve browser-opening actions to the running Hub or its intake.
    pub fn browser_url(self, browser_url: &str) -> Option<String> {
        let action = match self {
            Self::Open => return Some(browser_url.to_string()),
            Self::OpenProject => "open-folder",
            Self::Clone => "clone",
            Self::CopyUrl | Self::Quit => return None,
        };
        Some(format!(
            "{}/?action={action}",
            browser_url.trim_end_matches('/')
        ))
    }

    /// Map a tray-icon `MenuEvent` id back to a typed action.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            ids::OPEN => Some(Self::Open),
            ids::COPY_URL => Some(Self::CopyUrl),
            ids::QUIT => Some(Self::Quit),
            ids::OPEN_PROJECT => Some(Self::OpenProject),
            ids::CLONE => Some(Self::Clone),
            _ => None,
        }
    }
}

/// Visible tray label for the Copy URL action.
///
/// The label includes the exact root browser URL copied by the handler so
/// the active ephemeral port is visible before the user clicks.
pub fn copy_url_label_for_browser_url(browser_url: &str) -> String {
    format!("Copy URL ({browser_url})")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(
        key: &str,
        title: &str,
        open: bool,
        running: usize,
        errors: usize,
    ) -> TrayProjectEntry {
        TrayProjectEntry {
            project_key: gwt_core::repo_hash::ProjectKey::parse(key).unwrap(),
            title: title.into(),
            open,
            running_count: running,
            error_count: errors,
        }
    }

    #[test]
    fn projects_prioritize_open_and_escape_labels_without_paths() {
        let open = project("aaaaaaaaaaaaaaaa", "A & B", true, 2, 1);
        let recent = project("bbbbbbbbbbbbbbbb", "Recent", false, 0, 0);
        let snapshot = TraySnapshot::new(
            vec![open.clone()],
            vec![open.clone(), recent.clone(), recent.clone()],
        );
        assert_eq!(snapshot.projects, vec![open, recent]);
        assert_eq!(snapshot, snapshot.clone());
        let model = TrayMenuGeneration::new(1, snapshot, "http://127.0.0.1:1234/");
        assert_eq!(model.items.len(), 2);
        assert_eq!(model.items[0].label, "A && B — RUN 2 / ERROR 1");
        assert_eq!(model.items[1].label, "Recent (Recent) — RUN 0 / ERROR 0");
        assert_eq!(
            model.resolve(&model.items[0].id).unwrap().url,
            "http://127.0.0.1:1234/p/aaaaaaaaaaaaaaaa"
        );
        assert!(model.snapshot.has_error());
    }

    #[test]
    fn rebuild_commits_only_after_success_and_rejects_stale_ids() {
        let entry = project("aaaaaaaaaaaaaaaa", "A", true, 1, 1);
        let mut current = TrayMenuGeneration::new(
            1,
            TraySnapshot::new(vec![entry], vec![]),
            "http://localhost:1234/",
        );
        let old_id = current.items[0].id.clone();
        let recovered =
            TraySnapshot::new(vec![project("aaaaaaaaaaaaaaaa", "A", true, 0, 0)], vec![]);
        let failed = current.rebuild(recovered.clone(), "http://localhost:1234/", |_| {
            Err("platform unavailable")
        });
        assert_eq!(failed, Err("platform unavailable"));
        assert!(current.resolve(&old_id).is_some());
        assert!(current.snapshot.has_error());
        assert_eq!(
            current.rebuild(recovered, "http://localhost:1234/", |_| Ok::<_, &str>(())),
            Ok(true)
        );
        assert!(current.resolve(&old_id).is_none());
        assert!(!current.snapshot.has_error());
        assert_eq!(
            current.rebuild(
                current.snapshot.clone(),
                "http://localhost:1234/",
                |_| panic!("equal snapshot must not mutate platform")
            ),
            Ok::<_, &str>(false)
        );
    }

    #[test]
    fn failed_partial_install_ids_are_never_reused() {
        let mut current = TrayMenuGeneration::new(0, TraySnapshot::default(), "http://localhost/");
        let snapshot =
            TraySnapshot::new(vec![project("aaaaaaaaaaaaaaaa", "A", true, 0, 0)], vec![]);
        let mut failed_id = String::new();
        assert_eq!(
            current.rebuild(snapshot.clone(), "http://localhost/", |next| {
                failed_id = next.items[0].id.clone();
                Err("partial menu update")
            }),
            Err("partial menu update")
        );
        current
            .rebuild(snapshot, "http://localhost/", |_| Ok::<_, &str>(()))
            .unwrap();
        assert!(current.resolve(&failed_id).is_none());
    }

    #[test]
    fn failed_install_is_repaired_even_if_runtime_returns_to_previous_snapshot() {
        let mut current = TrayMenuGeneration::new(0, TraySnapshot::default(), "http://localhost/");
        let changed = TraySnapshot::new(vec![project("aaaaaaaaaaaaaaaa", "A", true, 0, 0)], vec![]);
        assert!(current
            .rebuild(changed, "http://localhost/", |_| Err("partial install"))
            .is_err());
        let mut repaired = false;
        current
            .rebuild(TraySnapshot::default(), "http://localhost/", |_| {
                repaired = true;
                Ok::<_, &str>(())
            })
            .unwrap();
        assert!(repaired);
    }

    #[test]
    fn menu_action_round_trip_through_ids() {
        assert_eq!(MenuAction::from_id(ids::OPEN), Some(MenuAction::Open));
        assert_eq!(
            MenuAction::from_id(ids::COPY_URL),
            Some(MenuAction::CopyUrl)
        );
        assert_eq!(MenuAction::from_id(ids::QUIT), Some(MenuAction::Quit));
        assert_eq!(MenuAction::from_id("gwt.tray.about"), None);
        assert_eq!(
            MenuAction::from_id(ids::OPEN_PROJECT),
            Some(MenuAction::OpenProject)
        );
        assert_eq!(MenuAction::from_id(ids::CLONE), Some(MenuAction::Clone));
        assert_eq!(MenuAction::from_id("unknown"), None);
    }

    #[test]
    fn menu_action_ids_are_stable_and_namespaced() {
        // The Phase 4 event loop persists these ids into tray-icon Menu
        // entries; a rename would silently break click dispatch. Pin the
        // exact strings so future edits surface as test failures.
        assert_eq!(ids::OPEN, "gwt.tray.open");
        assert_eq!(ids::COPY_URL, "gwt.tray.copy_url");
        assert_eq!(ids::QUIT, "gwt.tray.quit");
        assert_eq!(ids::OPEN_PROJECT, "gwt.tray.open_project");
        assert_eq!(ids::CLONE, "gwt.tray.clone");
    }

    #[test]
    fn tray_intake_ids_resolve_to_the_direct_hub_action() {
        for (id, expected) in [
            (ids::OPEN, "http://127.0.0.1:54321/"),
            (
                ids::OPEN_PROJECT,
                "http://127.0.0.1:54321/?action=open-folder",
            ),
            (ids::CLONE, "http://127.0.0.1:54321/?action=clone"),
        ] {
            assert_eq!(
                MenuAction::from_id(id)
                    .unwrap()
                    .browser_url("http://127.0.0.1:54321/")
                    .as_deref(),
                Some(expected)
            );
        }
        assert_eq!(MenuAction::CopyUrl.browser_url("http://localhost/"), None);
        assert_eq!(MenuAction::Quit.browser_url("http://localhost/"), None);
    }

    #[test]
    fn copy_url_label_includes_browser_url() {
        assert_eq!(
            copy_url_label_for_browser_url("http://127.0.0.1:54321/"),
            "Copy URL (http://127.0.0.1:54321/)"
        );
    }
}
