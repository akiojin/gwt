//! Project open / clone / tab lifecycle + migration surfacing split out of
//! `app_runtime/mod.rs` for SPEC-3064 Phase 1 (Pass 2).
//!
//! Owns:
//! - Open Project / Reopen Recent / clone-project flows
//!   ([`AppRuntime::open_project_dialog_events`],
//!   [`AppRuntime::clone_project_start_events`],
//!   [`AppRuntime::open_project_path_events`], ...)
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

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use super::startup::prepare_open_project_window_restores;
use super::{
    combined_window_id, load_restored_workspace_state, normalize_recent_project_path,
    resolve_project_target, same_worktree_path, AppRuntime, BackendEvent, OutboundEvent,
    PreparedMigrationSnapshot, PreparedProjectOpen, PreparedProjectSwitch, ProjectIncarnation,
    ProjectNavigationPayload, ProjectNavigationPrepared, ProjectNavigationRequest,
    ProjectNavigationSource, ProjectOpenTarget, ProjectTabRuntime, UserEvent, Uuid,
    WindowCanvasState,
};

pub(crate) fn initial_project_tab_incarnations(
    tabs: &[ProjectTabRuntime],
) -> (HashMap<String, ProjectIncarnation>, u64) {
    let mut next_generation = 1_u64;
    let incarnations = tabs
        .iter()
        .map(|tab| {
            let generation = next_generation;
            next_generation = next_generation.saturating_add(1);
            (
                tab.id.clone(),
                ProjectIncarnation {
                    project_key: gwt_core::paths::resolve_project_scope(&tab.project_root).hash,
                    generation,
                    project_root: tab.project_root.clone(),
                    migration_pending: tab.migration_pending,
                },
            )
        })
        .collect();
    (incarnations, next_generation)
}

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

fn prepare_migration_snapshot(target: &ProjectOpenTarget) -> Option<PreparedMigrationSnapshot> {
    target.needs_migration.then(|| PreparedMigrationSnapshot {
        branch: read_head_branch(&target.project_root),
        has_dirty: detect_dirty(&target.project_root),
        has_locked: detect_locked_worktrees(&target.project_root),
        has_submodules: target.project_root.join(".gitmodules").is_file(),
        has_backup: target
            .project_root
            .join(gwt_core::migration::backup::BACKUP_DIR_NAME)
            .is_dir(),
    })
}

fn prepare_project_open(
    path: PathBuf,
    sessions_dir: PathBuf,
) -> Result<ProjectNavigationPayload, String> {
    let target = resolve_project_target(&path)?;
    let project_key = gwt_core::paths::resolve_project_scope(&target.project_root).hash;
    let workspace =
        load_restored_workspace_state(&target.project_root).map_err(|error| error.to_string())?;
    let window_restores = prepare_open_project_window_restores(
        &workspace,
        &sessions_dir,
        target.kind,
        target.needs_migration,
    );
    let recent_path = normalize_recent_project_path(&target.project_root);
    let migration = prepare_migration_snapshot(&target);
    Ok(ProjectNavigationPayload::Open(PreparedProjectOpen {
        target,
        project_key,
        workspace,
        window_restores,
        recent_path,
        migration,
    }))
}

fn prepare_project_switch(
    tab_id: String,
    project_root: PathBuf,
) -> Result<ProjectNavigationPayload, String> {
    Ok(ProjectNavigationPayload::Switch(PreparedProjectSwitch {
        tab_id,
        project_key: gwt_core::paths::resolve_project_scope(&project_root).hash,
    }))
}

impl AppRuntime {
    pub(crate) fn open_project_dialog_events(&mut self) -> Vec<OutboundEvent> {
        let selected = rfd::FileDialog::new().pick_folder();
        self.open_project_dialog_selection_events(selected)
    }

    pub(crate) fn open_project_dialog_selection_events(
        &mut self,
        selected: Option<PathBuf>,
    ) -> Vec<OutboundEvent> {
        let Some(path) = selected else {
            self.invalidate_project_navigation();
            return Vec::new();
        };
        self.request_project_open(path, ProjectNavigationSource::Open)
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
        self.request_project_open(path, ProjectNavigationSource::Open)
    }

    pub(crate) fn handle_clone_project_done(
        &mut self,
        workspace_home: &Path,
    ) -> Vec<OutboundEvent> {
        self.request_project_open(
            workspace_home.to_path_buf(),
            ProjectNavigationSource::Clone {
                workspace_home: workspace_home.to_path_buf(),
            },
        )
    }

    fn next_project_navigation_request_id(&mut self) -> u64 {
        self.project_navigation_request =
            self.project_navigation_request.checked_add(1).unwrap_or(1);
        self.project_navigation_request
    }

    fn reserve_project_navigation(
        &mut self,
        source: ProjectNavigationSource,
        target_incarnation: Option<ProjectIncarnation>,
    ) -> ProjectNavigationRequest {
        let expected_active_tab_id = self.active_tab_id.clone();
        let expected_active_incarnation = expected_active_tab_id
            .as_ref()
            .and_then(|tab_id| self.project_tab_incarnations.get(tab_id))
            .cloned();
        let request = ProjectNavigationRequest {
            id: self.next_project_navigation_request_id(),
            source,
            expected_active_tab_id,
            expected_active_incarnation,
            target_incarnation,
        };
        self.pending_project_navigation = Some(request.clone());
        request
    }

    fn invalidate_project_navigation(&mut self) {
        self.next_project_navigation_request_id();
        self.pending_project_navigation = None;
    }

    fn request_project_open(
        &mut self,
        path: PathBuf,
        source: ProjectNavigationSource,
    ) -> Vec<OutboundEvent> {
        let request = self.reserve_project_navigation(source, None);
        let request_for_worker = request.clone();
        let proxy = self.proxy.clone();
        let sessions_dir = self.sessions_dir.clone();
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            proxy.send(UserEvent::ProjectNavigationPrepared(Box::new(
                ProjectNavigationPrepared {
                    request: request_for_worker,
                    result: prepare_project_open(path, sessions_dir),
                },
            )));
        }) {
            if self
                .pending_project_navigation
                .as_ref()
                .is_some_and(|pending| pending.id == request.id)
            {
                self.pending_project_navigation = None;
            }
            return self.project_open_error_events(&request.source, error);
        }
        Vec::new()
    }

    fn project_open_error_events(
        &self,
        source: &ProjectNavigationSource,
        message: String,
    ) -> Vec<OutboundEvent> {
        let event = match source {
            ProjectNavigationSource::Clone { .. } => BackendEvent::CloneProjectError { message },
            ProjectNavigationSource::Open | ProjectNavigationSource::Switch { .. } => {
                BackendEvent::ProjectOpenError { message }
            }
        };
        vec![OutboundEvent::broadcast(event)]
    }

    fn project_navigation_request_is_current(&self, request: &ProjectNavigationRequest) -> bool {
        if self.pending_project_navigation.as_ref() != Some(request)
            || self.active_tab_id != request.expected_active_tab_id
        {
            return false;
        }
        let active_incarnation = request
            .expected_active_tab_id
            .as_ref()
            .and_then(|tab_id| self.project_tab_incarnations.get(tab_id));
        if active_incarnation != request.expected_active_incarnation.as_ref() {
            return false;
        }
        request.target_incarnation.as_ref().is_none_or(|expected| {
            let ProjectNavigationSource::Switch { tab_id } = &request.source else {
                return false;
            };
            self.project_tab_incarnations.get(tab_id) == Some(expected)
        })
    }

    pub(crate) fn handle_project_navigation_prepared(
        &mut self,
        prepared: ProjectNavigationPrepared,
    ) -> Vec<OutboundEvent> {
        if !self.project_navigation_request_is_current(&prepared.request) {
            return Vec::new();
        }
        match prepared.result {
            Err(error) => {
                self.pending_project_navigation = None;
                self.project_open_error_events(&prepared.request.source, error)
            }
            Ok(ProjectNavigationPayload::Open(open)) => {
                if matches!(
                    prepared.request.source,
                    ProjectNavigationSource::Switch { .. }
                ) {
                    return Vec::new();
                }
                self.pending_project_navigation = None;
                self.commit_prepared_project_open(open, prepared.request.source)
            }
            Ok(ProjectNavigationPayload::Switch(switch)) => {
                let ProjectNavigationSource::Switch { tab_id } = &prepared.request.source else {
                    return Vec::new();
                };
                if tab_id != &switch.tab_id
                    || prepared
                        .request
                        .target_incarnation
                        .as_ref()
                        .is_none_or(|incarnation| incarnation.project_key != switch.project_key)
                {
                    return Vec::new();
                }
                self.pending_project_navigation = None;
                if let Some(tab) = self.tab(&switch.tab_id) {
                    self.spawn_work_events_ingest_for_project_key(
                        tab.project_root.clone(),
                        switch.project_key,
                        false,
                    );
                }
                Vec::new()
            }
        }
    }

    fn commit_prepared_project_open(
        &mut self,
        prepared: PreparedProjectOpen,
        source: ProjectNavigationSource,
    ) -> Vec<OutboundEvent> {
        self.remember_prepared_recent_project(&prepared);
        let existing_tab_id =
            self.project_tab_incarnations
                .iter()
                .find_map(|(tab_id, incarnation)| {
                    (incarnation.project_key == prepared.project_key).then(|| tab_id.clone())
                });
        let (tab_id, new_tab) = if let Some(tab_id) = existing_tab_id {
            (tab_id, false)
        } else {
            let tab_id = format!("project-{}", Uuid::new_v4().simple());
            self.tabs.push(ProjectTabRuntime {
                id: tab_id.clone(),
                title: prepared.target.title.clone(),
                project_root: prepared.target.project_root.clone(),
                kind: prepared.target.kind,
                workspace: WindowCanvasState::from_persisted(prepared.workspace),
                migration_pending: prepared.target.needs_migration,
                main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
            });
            let generation = self.next_project_incarnation;
            self.next_project_incarnation = self.next_project_incarnation.saturating_add(1);
            self.project_tab_incarnations.insert(
                tab_id.clone(),
                ProjectIncarnation {
                    project_key: prepared.project_key.clone(),
                    generation,
                    project_root: prepared.target.project_root.clone(),
                    migration_pending: prepared.target.needs_migration,
                },
            );
            (tab_id, true)
        };

        let wizard_closed = self.set_active_tab(tab_id.clone());
        if let ProjectNavigationSource::Clone { workspace_home } = &source {
            self.remember_recent_clone_workspace_home(workspace_home);
        }
        let _ = self.persist();

        let mut events = vec![self.workspace_state_broadcast_process_free()];
        if new_tab {
            events.extend(
                self.restore_prepared_open_project_windows(&tab_id, prepared.window_restores),
            );
            events.extend(self.ensure_pm_agent_for_tab(
                &tab_id,
                crate::app_runtime::pm::PmEnsureTrigger::Automatic,
            ));
            self.spawn_work_events_ingest_for_project_key(
                prepared.target.project_root.clone(),
                prepared.project_key,
                true,
            );
        }
        if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
            events.push(event);
        }
        if wizard_closed {
            events.push(self.launch_wizard_state_broadcast(None));
        }
        if let Some(snapshot) = prepared.migration.as_ref() {
            events.push(OutboundEvent::broadcast(BackendEvent::MigrationDetected {
                tab_id: tab_id.clone(),
                project_root: prepared.target.project_root.display().to_string(),
                branch: snapshot.branch.clone(),
                has_dirty: snapshot.has_dirty,
                has_locked: snapshot.has_locked,
                has_submodules: snapshot.has_submodules,
            }));
            if snapshot.has_backup {
                let tab = self.tab(&tab_id).expect("opened project tab");
                events.push(OutboundEvent::broadcast(
                    self.migration_backup_error_event_for(tab),
                ));
            }
        }
        if let ProjectNavigationSource::Clone { workspace_home } = source {
            events.push(OutboundEvent::broadcast(BackendEvent::CloneProjectDone {
                workspace_home: workspace_home.display().to_string(),
            }));
        }
        events
    }

    fn remember_prepared_recent_project(&mut self, prepared: &PreparedProjectOpen) {
        self.recent_projects
            .retain(|entry| !same_worktree_path(&entry.path, &prepared.recent_path));
        self.recent_projects.insert(
            0,
            gwt::RecentProjectEntry {
                path: prepared.recent_path.clone(),
                title: gwt::project_title_from_path(&prepared.recent_path),
                kind: prepared.target.kind,
            },
        );
        self.recent_projects.truncate(12);
    }

    pub(crate) fn refresh_project_tab_incarnation(&mut self, tab_id: &str) {
        let Some(tab) = self.tab(tab_id).cloned() else {
            self.project_tab_incarnations.remove(tab_id);
            return;
        };
        let generation = self.next_project_incarnation;
        self.next_project_incarnation = self.next_project_incarnation.saturating_add(1);
        self.project_tab_incarnations.insert(
            tab_id.to_string(),
            ProjectIncarnation {
                project_key: gwt_core::paths::resolve_project_scope(&tab.project_root).hash,
                generation,
                project_root: tab.project_root,
                migration_pending: tab.migration_pending,
            },
        );
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
    #[cfg(test)]
    pub(crate) fn migration_detected_broadcasts(&self) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| tab.migration_pending)
            .map(|tab| OutboundEvent::broadcast(self.migration_detected_event_for(tab)))
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

    pub(crate) fn select_project_tab_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        let Some(target_incarnation) = self.project_tab_incarnations.get(tab_id).cloned() else {
            return Vec::new();
        };
        let project_root = target_incarnation.project_root.clone();
        let wizard_closed = self.set_active_tab(tab_id.to_string());
        let request = self.reserve_project_navigation(
            ProjectNavigationSource::Switch {
                tab_id: tab_id.to_string(),
            },
            Some(target_incarnation),
        );
        let _ = self.persist();
        let request_for_worker = request.clone();
        let tab_id_for_worker = tab_id.to_string();
        let proxy = self.proxy.clone();
        let prepare_error = self
            .blocking_tasks
            .try_spawn(move || {
                proxy.send(UserEvent::ProjectNavigationPrepared(Box::new(
                    ProjectNavigationPrepared {
                        request: request_for_worker,
                        result: prepare_project_switch(tab_id_for_worker, project_root),
                    },
                )));
            })
            .err();
        if prepare_error.is_some()
            && self
                .pending_project_navigation
                .as_ref()
                .is_some_and(|pending| pending.id == request.id)
        {
            self.pending_project_navigation = None;
        }
        let mut events = vec![self.workspace_state_broadcast_process_free()];
        if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
            events.push(event);
        }
        if wizard_closed {
            events.push(self.launch_wizard_state_broadcast(None));
        }
        if let Some(error) = prepare_error {
            events.extend(self.project_open_error_events(&request.source, error));
        }
        events
    }

    pub(crate) fn close_project_tab_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return Vec::new();
        };
        let closing_project_root = self.tabs[index].project_root.clone();

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
            self.queue_accepted_window_close_finalizer(
                window_id,
                Some(closing_project_root.clone()),
                true,
                None,
            );
            self.window_lookup.remove(window_id);
        }

        self.tabs.remove(index);
        self.project_tab_incarnations.remove(tab_id);
        self.invalidate_project_navigation();
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
        events
    }
}
