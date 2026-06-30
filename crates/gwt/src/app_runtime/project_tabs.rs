//! Project open / clone / tab lifecycle + migration surfacing split out of
//! `app_runtime/mod.rs` for SPEC-3064 Phase 1 (Pass 2).
//!
//! Owns:
//! - Open Project / Reopen Recent / clone-project flows
//!   ([`AppRuntime::open_project_dialog_events`],
//!   [`AppRuntime::clone_project_start_events`],
//!   [`AppRuntime::open_project_path`], ...)
//! - GitHub repository search for the clone dialog
//!   (`search_github_repositories`, `parse_github_repository_search_results`)
//! - Project tab selection / close ([`AppRuntime::select_project_tab_events`],
//!   [`AppRuntime::close_project_tab_events`]) and recent-project bookkeeping
//! - SPEC-1934 migration detection broadcasts / replies
//!   (`recovery_state_label` stays re-exported through `mod.rs` for
//!   `migration.rs`)
//!
//! Behavior-preserving move: `ProjectTabRuntime` / `ProjectOpenTarget` stay
//! in `mod.rs` and are reached via `super`.

use std::path::{Path, PathBuf};

use super::{
    combined_window_id, load_restored_workspace_state, normalize_recent_project_path,
    resolve_project_target, same_worktree_path, AppRuntime, BackendEvent, OutboundEvent,
    ProjectOpenTarget, ProjectTabRuntime, UserEvent, Uuid, WindowCanvasState,
};

pub(super) fn recovery_state_label(recovery: gwt_core::migration::RecoveryState) -> &'static str {
    use gwt_core::migration::RecoveryState;
    match recovery {
        RecoveryState::Untouched => "untouched",
        RecoveryState::RolledBack => "rolled_back",
        RecoveryState::Partial => "partial",
    }
}

/// Best-effort `git symbolic-ref --short HEAD` for the migration modal
/// preview. Returns `None` for detached HEAD or unreadable repositories so
/// the frontend can fall back to a neutral label.
fn read_head_branch(project_root: &Path) -> Option<String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(project_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

#[derive(Debug, serde::Deserialize)]
struct GhRepositorySearchRecord {
    #[serde(rename = "fullName")]
    full_name: Option<String>,
    description: Option<String>,
    url: Option<String>,
    #[serde(rename = "defaultBranch")]
    default_branch: Option<String>,
    visibility: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
}

pub(crate) fn parse_github_repository_search_results(
    raw: &str,
) -> Result<Vec<gwt::GitHubRepositorySearchResultView>, String> {
    let records: Vec<GhRepositorySearchRecord> =
        serde_json::from_str(raw).map_err(|error| format!("parse gh search JSON: {error}"))?;
    let mut repositories = Vec::new();
    for record in records {
        let Some(full_name) = record.full_name.filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        let Some(url) = record.url.filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        repositories.push(gwt::GitHubRepositorySearchResultView {
            full_name,
            description: record.description.filter(|value| !value.trim().is_empty()),
            url,
            default_branch: record
                .default_branch
                .filter(|value| !value.trim().is_empty()),
            visibility: record.visibility.filter(|value| !value.trim().is_empty()),
            updated_at: record.updated_at.filter(|value| !value.trim().is_empty()),
        });
    }
    Ok(repositories)
}

fn search_github_repositories(
    query: &str,
    limit: usize,
) -> Result<Vec<gwt::GitHubRepositorySearchResultView>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("repository search query is required".to_string());
    }
    let hub = gwt_core::process_console::global();
    let limit_str = limit.to_string();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &[
            "search",
            "repos",
            trimmed,
            "--json",
            "fullName,description,url,defaultBranch,visibility,updatedAt",
            "--limit",
            limit_str.as_str(),
        ],
        gwt_core::process_console::SpawnOptions::new("gh search repos"),
    )
    .map_err(|error| format!("gh search repos: {error}"))?;
    if !output.success() {
        let stderr = output.stderr.trim().to_string();
        return Err(if stderr.is_empty() {
            "gh search repos failed".to_string()
        } else {
            stderr
        });
    }
    parse_github_repository_search_results(&output.stdout)
}

fn detect_dirty(project_root: &Path) -> bool {
    gwt_core::process::hidden_command("git")
        .args(["status", "--porcelain"])
        .current_dir(project_root)
        .output()
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false)
}

/// `true` when any worktree under `project_root` is locked. Mirrors the more
/// thorough check inside `gwt_core::migration::validator::check_locked_worktrees`.
fn detect_locked_worktrees(project_root: &Path) -> bool {
    let Ok(output) = gwt_core::process::hidden_command("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(project_root)
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.starts_with("locked"))
}

impl AppRuntime {
    pub(crate) fn open_project_dialog_events(&mut self) -> Vec<OutboundEvent> {
        let selected = rfd::FileDialog::new().pick_folder();
        let Some(path) = selected else {
            return Vec::new();
        };
        self.open_project_path_events(path)
    }

    pub(crate) fn select_clone_project_parent_events(
        &mut self,
        client_id: &str,
    ) -> Vec<OutboundEvent> {
        let selected = rfd::FileDialog::new().pick_folder();
        let Some(path) = selected else {
            return Vec::new();
        };
        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::CloneProjectParentSelected {
                path: path.display().to_string(),
            },
        )]
    }

    pub(crate) fn github_repository_search_events(
        &mut self,
        client_id: &str,
        query: &str,
    ) -> Vec<OutboundEvent> {
        match search_github_repositories(query, 20) {
            Ok(repositories) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::GithubRepositorySearchResults {
                    query: query.to_string(),
                    repositories,
                },
            )],
            Err(message) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::GithubRepositorySearchError {
                    query: query.to_string(),
                    message,
                },
            )],
        }
    }

    pub(crate) fn clone_project_start_events(
        &mut self,
        client_id: &str,
        url: &str,
        parent_path: &str,
    ) -> Vec<OutboundEvent> {
        let trimmed_url = url.trim();
        if trimmed_url.is_empty() {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::CloneProjectError {
                    message: "repository URL is required".to_string(),
                },
            )];
        }
        let trimmed_parent = parent_path.trim();
        if trimmed_parent.is_empty() {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::CloneProjectError {
                    message: "destination parent folder is required".to_string(),
                },
            )];
        }

        let proxy = self.proxy.clone();
        let url = trimmed_url.to_string();
        let parent = PathBuf::from(trimmed_parent);
        self.blocking_tasks.spawn(move || {
            proxy.send(UserEvent::CloneProjectProgress {
                message: "Cloning repository...".to_string(),
            });
            match gwt_git::clone_project_as_nested_bare(&url, &parent) {
                Ok(outcome) => proxy.send(UserEvent::CloneProjectDone {
                    workspace_home: outcome.workspace_home,
                }),
                Err(error) => proxy.send(UserEvent::CloneProjectError {
                    message: error.to_string(),
                }),
            }
        });

        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::CloneProjectProgress {
                message: "Cloning repository...".to_string(),
            },
        )]
    }

    pub(crate) fn open_project_path_events(&mut self, path: PathBuf) -> Vec<OutboundEvent> {
        match self.open_project_path(path) {
            Ok(wizard_closed) => {
                let mut events = vec![self.workspace_state_broadcast()];
                // Issue #2942: restore the opened tab's process windows the
                // user did not explicitly close — resume agents (native session
                // id) and fresh-launch shells. The startup `bootstrap` queue
                // only covers tabs open at launch, so projects opened via this
                // path (Open Project / Reopen Recent) were never restored and
                // their agent panes stayed `Stopped`.
                if let Some(active_tab_id) = self.active_tab_id.clone() {
                    events.extend(self.restore_open_project_windows(&active_tab_id));
                }
                // SPEC-2359 W-16 (FR-387): run the cross-machine intake for
                // the opened project; its completion event reconciles the
                // worktrees (intake → reconcile order) and kicks the merge
                // scan, then rebroadcasts the projection.
                if let Some(project_root) = self
                    .active_tab_id
                    .as_ref()
                    .and_then(|id| self.tabs.iter().find(|tab| &tab.id == id))
                    .map(|tab| tab.project_root.clone())
                {
                    self.spawn_work_events_ingest(project_root, true);
                }
                if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
                    events.push(event);
                }
                if wizard_closed {
                    events.push(self.launch_wizard_state_broadcast(None));
                }
                // SPEC-1934 US-6.1: when a tab was opened on a Normal Git
                // layout, surface the confirmation modal alongside the regular
                // workspace broadcast.
                events.extend(self.migration_detected_broadcasts());
                events.extend(self.migration_recovery_broadcasts());
                events
            }
            Err(error) => vec![OutboundEvent::broadcast(BackendEvent::ProjectOpenError {
                message: error,
            })],
        }
    }

    pub(crate) fn handle_clone_project_done(
        &mut self,
        workspace_home: &Path,
    ) -> Vec<OutboundEvent> {
        match self.open_project_path(workspace_home.to_path_buf()) {
            Ok(wizard_closed) => {
                self.remember_recent_clone_workspace_home(workspace_home);
                let _ = self.persist();
                let mut events = vec![
                    self.workspace_state_broadcast(),
                    OutboundEvent::broadcast(BackendEvent::CloneProjectDone {
                        workspace_home: workspace_home.display().to_string(),
                    }),
                ];
                if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
                    events.push(event);
                }
                if wizard_closed {
                    events.push(self.launch_wizard_state_broadcast(None));
                }
                events
            }
            Err(error) => vec![OutboundEvent::broadcast(BackendEvent::CloneProjectError {
                message: error,
            })],
        }
    }

    fn remember_recent_clone_workspace_home(&mut self, workspace_home: &Path) {
        let canonical_home =
            dunce::canonicalize(workspace_home).unwrap_or_else(|_| workspace_home.to_path_buf());
        self.recent_projects
            .retain(|entry| !same_worktree_path(&entry.path, &canonical_home));
        self.recent_projects.insert(
            0,
            gwt::RecentProjectEntry {
                path: canonical_home.clone(),
                title: gwt::project_title_from_path(&canonical_home),
                kind: gwt::ProjectKind::Git,
            },
        );
        if self.recent_projects.len() > 12 {
            self.recent_projects.truncate(12);
        }
    }

    pub(crate) fn open_project_path(&mut self, path: PathBuf) -> Result<bool, String> {
        let target = resolve_project_target(&path)?;
        if let Some(existing) = self
            .tabs
            .iter()
            .find(|tab| same_worktree_path(&tab.project_root, &target.project_root))
            .map(|tab| tab.id.clone())
        {
            let wizard_closed = self.set_active_tab(existing);
            self.remember_recent_project(&target);
            self.persist().map_err(|error| error.to_string())?;
            return Ok(wizard_closed);
        }

        let tab_id = format!("project-{}", Uuid::new_v4().simple());
        self.tabs.push(ProjectTabRuntime {
            id: tab_id.clone(),
            title: target.title.clone(),
            project_root: target.project_root.clone(),
            kind: target.kind,
            workspace: WindowCanvasState::from_persisted({
                load_restored_workspace_state(&target.project_root)
                    .map_err(|error| error.to_string())?
            }),
            migration_pending: target.needs_migration,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        });
        self.active_tab_id = Some(tab_id);
        self.remember_recent_project(&target);
        let wizard_closed = self.clear_launch_wizard().is_some();
        self.persist().map_err(|error| error.to_string())?;
        Ok(wizard_closed)
    }

    fn migration_detected_event_for(&self, tab: &ProjectTabRuntime) -> BackendEvent {
        BackendEvent::MigrationDetected {
            tab_id: tab.id.clone(),
            project_root: tab.project_root.display().to_string(),
            branch: read_head_branch(&tab.project_root),
            has_dirty: detect_dirty(&tab.project_root),
            has_locked: detect_locked_worktrees(&tab.project_root),
            has_submodules: tab.project_root.join(".gitmodules").is_file(),
        }
    }

    fn has_migration_backup(tab: &ProjectTabRuntime) -> bool {
        tab.project_root
            .join(gwt_core::migration::backup::BACKUP_DIR_NAME)
            .is_dir()
    }

    fn migration_backup_error_event_for(&self, tab: &ProjectTabRuntime) -> BackendEvent {
        let backup_path = tab
            .project_root
            .join(gwt_core::migration::backup::BACKUP_DIR_NAME);
        BackendEvent::MigrationError {
            tab_id: tab.id.clone(),
            phase: gwt_core::migration::MigrationPhase::Backup
                .as_str()
                .to_string(),
            message: format!(
                "Previous migration backup found at {}. A migration may have been interrupted before cleanup; inspect or restore the backup before starting another migration.",
                backup_path.display()
            ),
            recovery: recovery_state_label(gwt_core::migration::RecoveryState::Partial)
                .to_string(),
        }
    }

    /// SPEC-1934 US-6.1 broadcast variant: used by `open_project_path_events`
    /// to inform every connected frontend that a tab needs migration.
    pub(crate) fn migration_detected_broadcasts(&self) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| tab.migration_pending)
            .map(|tab| OutboundEvent::broadcast(self.migration_detected_event_for(tab)))
            .collect()
    }

    /// SPEC-1934 US-6.6/T-085: if a previous migration was interrupted after
    /// Backup, surface the leftover snapshot on launch so the user does not
    /// start another destructive migration over an unresolved backup.
    pub(crate) fn migration_recovery_broadcasts(&self) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| tab.migration_pending && Self::has_migration_backup(tab))
            .map(|tab| OutboundEvent::broadcast(self.migration_backup_error_event_for(tab)))
            .collect()
    }

    /// SPEC-1934 US-6.1 reply variant: used by `frontend_sync_events` so a
    /// freshly-connected frontend learns about pending migrations during
    /// state hydration without resending to other clients.
    pub(crate) fn migration_detected_replies(&self, client_id: &str) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| tab.migration_pending)
            .map(|tab| OutboundEvent::reply(client_id, self.migration_detected_event_for(tab)))
            .collect()
    }

    pub(crate) fn migration_recovery_replies(&self, client_id: &str) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| tab.migration_pending && Self::has_migration_backup(tab))
            .map(|tab| OutboundEvent::reply(client_id, self.migration_backup_error_event_for(tab)))
            .collect()
    }

    /// SPEC-1934 FR-019: user accepted the migration confirmation modal.
    ///
    /// Issue #2867: Recent Projects は同一プロジェクトの worktree で埋め尽く
    /// されないよう、`target.project_root` を workspace home に正規化してから
    /// 登録する。タブ open 時の direct-pick semantics は `target` 側で保持。
    pub(crate) fn remember_recent_project(&mut self, target: &ProjectOpenTarget) {
        let recent_path = normalize_recent_project_path(&target.project_root);
        let recent_title = if recent_path == target.project_root {
            target.title.clone()
        } else {
            gwt::project_title_from_path(&recent_path)
        };
        self.recent_projects
            .retain(|entry| !same_worktree_path(&entry.path, &recent_path));
        self.recent_projects.insert(
            0,
            gwt::RecentProjectEntry {
                path: recent_path,
                title: recent_title,
                kind: target.kind,
            },
        );
        if self.recent_projects.len() > 12 {
            self.recent_projects.truncate(12);
        }
    }

    pub(crate) fn select_project_tab_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        if !self.tabs.iter().any(|tab| tab.id == tab_id) {
            return Vec::new();
        }
        let wizard_closed = self.set_active_tab(tab_id.to_string());
        let _ = self.persist();
        // SPEC-2359 W-16 (FR-387): tab changes piggyback the cross-machine
        // intake, throttled to once per 30s per project.
        if let Some(project_root) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .map(|tab| tab.project_root.clone())
        {
            self.spawn_work_events_ingest(project_root, false);
        }
        let mut events = vec![self.workspace_state_broadcast()];
        if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
            events.push(event);
        }
        if wizard_closed {
            events.push(self.launch_wizard_state_broadcast(None));
        }
        events
    }

    pub(crate) fn close_project_tab_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return Vec::new();
        };

        let window_ids = self
            .tabs
            .get(index)
            .map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .map(|window| combined_window_id(&tab.id, &window.id))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for window_id in &window_ids {
            self.clear_agent_window_startup_restore(window_id);
            self.stop_window_runtime(window_id);
            self.remove_window_state_tracking(window_id);
            self.window_lookup.remove(window_id);
            self.profile_selections.remove(window_id);
        }

        // Return any Issue Monitor launched windows to pending before the tab is
        // removed, while the closing project is still the active root. Closing a
        // project pauses (does not complete) its in-flight work.
        let issue_monitor_events = self.issue_monitor_windows_closed_events(&window_ids);

        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.active_tab_id = None;
        } else if self.active_tab_id.as_deref() == Some(tab_id) {
            let next_index = index.saturating_sub(1).min(self.tabs.len() - 1);
            self.active_tab_id = self.tabs.get(next_index).map(|tab| tab.id.clone());
        }

        let wizard_closed = self
            .launch_wizard
            .as_ref()
            .is_some_and(|wizard| wizard.tab_id == tab_id);
        if wizard_closed {
            self.launch_wizard = None;
        }
        let _ = self.persist();

        let mut events = vec![self.workspace_state_broadcast()];
        if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
            events.push(event);
        }
        if wizard_closed {
            events.push(self.launch_wizard_state_broadcast(None));
        }
        events.extend(issue_monitor_events);
        events
    }
}
