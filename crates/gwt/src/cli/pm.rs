//! `pm.*` JSON operations (SPEC-3431): PM agent diagnostics and control.
//!
//! `pm.status` is the read-only diagnostic surface for the per-project PM
//! singleton (FR-001): it reports the durable registration, the auto-start
//! opt-out (FR-002), and a stale hint derived from the durable session store.
//! It is registered in the workflow-policy read-only and ownerless-safe
//! allowlists so any session can diagnose PM state before an owner is linked.
//!
//! `pm.stop` (Issue #3607) is the write side. Before it existed, a PM that had
//! been orphaned by a project-store split had no CLI route out at all —
//! `pane.close`, `issue.monitor.stop` and `pm.message.send` each refused for a
//! different reason, leaving GUI clicks as the only way to stop an agent that
//! was still rewriting the repository's Issue Monitor state. It is deliberately
//! durable-only: it takes PM authority away and makes the session
//! unrestorable, which is what stops the resident loop, without depending on a
//! running GUI or a live pane connection.

use std::path::{Path, PathBuf};

use gwt_core::paths::gwt_sessions_dir;
use gwt_github::{ApiError, SpecOpsError};
use serde::Serialize;

use crate::cli::env::CliEnv;
use crate::pm_registry;

/// Parsed `pm.*` command surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmCommand {
    /// `pm.status` — optional explicit `project_root`; defaults to the
    /// current repository path (container/bare setups must pass it
    /// explicitly, same convention as the Issue Monitor queue operations).
    Status { project_root: Option<String> },
    /// `pm.stop` — clear a PM registration in this repository and make its
    /// session unrestorable. `session_id` defaults to the caller's own
    /// registration, so a PM can always retire itself.
    Stop {
        project_root: Option<String>,
        session_id: Option<String>,
    },
}

/// Result of a `pm.stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PmStopReport {
    pub schema_version: u32,
    pub stopped_session_id: String,
    /// The project store that actually held the registration — for an orphan
    /// this is not the caller's store, which is the whole point.
    pub project_dir: String,
    pub stopped_self: bool,
    /// Whether the durable Session record was marked terminal. `false` means
    /// the record was already gone; the registration is cleared either way.
    pub session_record_updated: bool,
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: PmCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match command {
        PmCommand::Status { project_root } => {
            let repo_path = resolve_repo_path(env, project_root);
            let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&repo_path);
            let prefs = pm_registry::load_pm_prefs(&prefs_path).map_err(|error| {
                SpecOpsError::from(ApiError::Unexpected(format!(
                    "failed to load PM prefs from {}: {error}",
                    prefs_path.display()
                )))
            })?;
            // SPEC-3431 FR-009 diagnostic visibility: report whether THIS
            // caller holds PM privilege, so a refused Issue Monitor ON has a
            // one-command explanation.
            let caller_session = ambient_session_id();
            let mut report = pm_registry::pm_status_report_for_caller(
                &prefs,
                |session_id| {
                    gwt_sessions_dir()
                        .join(format!("{session_id}.toml"))
                        .exists()
                },
                caller_session.as_deref(),
            );
            report.repository_registrations =
                pm_registry::pm_repository_registration_views(&repo_path);
            let rendered = serde_json::to_string_pretty(&report).map_err(|error| {
                SpecOpsError::from(ApiError::Unexpected(format!(
                    "failed to serialize pm.status report: {error}"
                )))
            })?;
            out.push_str(&rendered);
            out.push('\n');
            Ok(0)
        }
        PmCommand::Stop {
            project_root,
            session_id,
        } => {
            let repo_path = resolve_repo_path(env, project_root);
            let report = stop_pm(&repo_path, session_id.as_deref())?;
            let rendered = serde_json::to_string_pretty(&report).map_err(|error| {
                SpecOpsError::from(ApiError::Unexpected(format!(
                    "failed to serialize pm.stop report: {error}"
                )))
            })?;
            out.push_str(&rendered);
            out.push('\n');
            Ok(0)
        }
    }
}

fn resolve_repo_path<E: CliEnv>(env: &E, project_root: Option<String>) -> PathBuf {
    project_root
        .map(PathBuf::from)
        .unwrap_or_else(|| env.repo_path().to_path_buf())
}

fn ambient_session_id() -> Option<String> {
    std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Issue #3607 AC-5/AC-6.
///
/// Authority is repository-scoped for the same reason the singleton is: the
/// orphan lives in a *different* project store, so a caller checked only
/// against its own store's `pm.json` could never reach it. Every refusal names
/// the operation that resolves it — a PM with no route out is precisely the
/// dead end this operation exists to remove.
fn stop_pm(
    repo_path: &Path,
    requested_session: Option<&str>,
) -> Result<PmStopReport, SpecOpsError> {
    let Some(repository_key) = pm_registry::pm_repository_key(repo_path) else {
        return Err(refusal(format!(
            "pm.stop could not resolve a repository for {}; run it from inside the repository or \
             pass `params.project_root` pointing at the repository or one of its worktrees",
            repo_path.display()
        )));
    };
    let Some(caller_session) = ambient_session_id() else {
        return Err(refusal(format!(
            "pm.stop is refused: {} is not set, so the caller cannot be identified as a registered \
             PM; relaunch the Session from gwt and retry, or stop the PM from its window in the GUI",
            gwt_agent::GWT_SESSION_ID_ENV
        )));
    };

    let registrations = pm_registry::pm_registrations_for_repository(&repository_key);
    if !registrations
        .iter()
        .any(|record| record.registration.session_id == caller_session)
    {
        return Err(refusal(format!(
            "pm.stop is refused: Session {caller_session} is not a registered PM for this \
             repository, and only a registered PM may retire one; run JSON operation `pm.status` \
             to see which Session holds the registration and ask that PM to run `pm.stop`"
        )));
    }

    let target_session = requested_session.unwrap_or(caller_session.as_str());
    let Some(stopped) =
        pm_registry::stop_pm_registration_in_repository(&repository_key, target_session)
    else {
        return Err(refusal(format!(
            "pm.stop found no PM registered as Session {target_session} in this repository; run \
             JSON operation `pm.status` and use a `session_id` from its \
             `repository_registrations`"
        )));
    };

    // Clearing the registration ends PM authority, but restore resolves a
    // window by session id alone — leaving the record restorable would hand the
    // same orphan back on the next startup.
    let session_record_updated =
        gwt_agent::update_session(&gwt_sessions_dir(), target_session, |session| {
            session.update_status(gwt_agent::AgentStatus::Stopped);
            session.restore_window_on_startup = false;
            Ok(())
        })
        .is_ok();

    Ok(PmStopReport {
        schema_version: 1,
        stopped_session_id: target_session.to_string(),
        project_dir: stopped.project_dir.display().to_string(),
        stopped_self: target_session == caller_session,
        session_record_updated,
    })
}

fn refusal(message: String) -> SpecOpsError {
    SpecOpsError::from(ApiError::PermissionDenied { message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
    use std::fs;

    /// Issue #3607 AC-4: two project stores over one repository, the shape the
    /// incident produced. The `.git` / `commondir` pair is written by hand so
    /// the fixture stays hermetic; it is exactly what `git worktree add`
    /// materializes.
    struct Fixture {
        _home: tempfile::TempDir,
        _repo_dir: tempfile::TempDir,
        repo: PathBuf,
        stores: Vec<PathBuf>,
        _home_guard: ScopedGwtHome,
    }

    impl Fixture {
        fn new(store_names: &[&str]) -> Self {
            let home = tempfile::tempdir().expect("home");
            let home_guard = ScopedGwtHome::set(home.path());
            let repo_dir = tempfile::tempdir().expect("repo dir");
            let repo = repo_dir.path().join("repo");
            let git_dir = repo.join(".git");
            fs::create_dir_all(git_dir.join("worktrees")).expect("git dir");
            fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD");
            fs::write(git_dir.join("config"), "[core]\n\tbare = false\n").expect("config");

            let stores = store_names
                .iter()
                .map(|name| {
                    let project_dir = gwt_core::paths::gwt_projects_dir().join(name);
                    let worktree = project_dir.join("pm/worktree");
                    fs::create_dir_all(&worktree).expect("pm worktree");
                    let admin = git_dir.join("worktrees").join(name);
                    fs::create_dir_all(&admin).expect("worktree admin");
                    fs::write(admin.join("commondir"), "../..\n").expect("commondir");
                    fs::write(
                        worktree.join(".git"),
                        format!("gitdir: {}\n", admin.display()),
                    )
                    .expect(".git file");
                    project_dir
                })
                .collect();

            Self {
                _home: home,
                _repo_dir: repo_dir,
                repo,
                stores,
                _home_guard: home_guard,
            }
        }

        fn register(&self, store_index: usize, session_id: &str) -> PathBuf {
            let project_dir = &self.stores[store_index];
            let prefs_path = project_dir.join("project-state/pm.json");
            pm_registry::save_pm_prefs(
                &prefs_path,
                &pm_registry::PmPrefs {
                    registration: Some(pm_registry::PmRegistration {
                        session_id: session_id.to_string(),
                        agent_id: "claude".to_string(),
                        worktree_path: project_dir.join("pm/worktree").display().to_string(),
                        created_at: Some("2026-08-16T02:04:10Z".to_string()),
                        consecutive_crashes: 0,
                        next_not_before: None,
                    }),
                    settings: pm_registry::PmSettings::default(),
                },
            )
            .expect("save prefs");
            prefs_path
        }

        fn save_session(&self, store_index: usize, session_id: &str) {
            let sessions_dir = gwt_sessions_dir();
            fs::create_dir_all(&sessions_dir).expect("sessions dir");
            let mut session = gwt_agent::Session::new(
                self.stores[store_index].join("pm/worktree"),
                "work",
                gwt_agent::AgentId::ClaudeCode,
            );
            session.id = session_id.to_string();
            session.restore_window_on_startup = true;
            session.save(&sessions_dir).expect("save session");
        }
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// AC-5: the live PM retires the orphan that a split store still holds,
    /// with no GUI involved.
    #[test]
    fn stop_clears_an_orphan_registration_held_by_another_store() {
        let _lock = env_lock();
        let fixture = Fixture::new(&["99a8660247f5bc49", "b19aac38305901f5"]);
        let live_prefs = fixture.register(0, "fedf798b-live");
        let orphan_prefs = fixture.register(1, "b0801016-orphan");
        fixture.save_session(1, "b0801016-orphan");
        let _caller = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "fedf798b-live");

        let report = stop_pm(&fixture.repo, Some("b0801016-orphan")).expect("stop the orphan");

        assert_eq!(report.stopped_session_id, "b0801016-orphan");
        assert!(!report.stopped_self);
        assert_eq!(report.project_dir, fixture.stores[1].display().to_string());
        assert!(report.session_record_updated);
        assert_eq!(
            pm_registry::load_pm_prefs(&orphan_prefs)
                .expect("orphan prefs")
                .registration,
            None,
            "the orphan must lose PM authority"
        );
        assert!(
            pm_registry::load_pm_prefs(&live_prefs)
                .expect("live prefs")
                .registration
                .is_some(),
            "the caller's own registration must survive"
        );

        let restored =
            gwt_agent::Session::load_and_migrate(&gwt_sessions_dir().join("b0801016-orphan.toml"))
                .expect("load stopped session");
        assert!(
            !restored.restore_window_on_startup,
            "a stopped PM must not come back through session restore"
        );
        assert_eq!(restored.status, gwt_agent::AgentStatus::Stopped);
    }

    /// AC-6: only a registered PM may stop one, and the refusal has to say what
    /// to do next.
    #[test]
    fn stop_refuses_a_caller_that_is_not_a_registered_pm() {
        let _lock = env_lock();
        let fixture = Fixture::new(&["store-a"]);
        let prefs = fixture.register(0, "the-pm");
        let _caller = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "some-other-agent");

        let error = stop_pm(&fixture.repo, Some("the-pm")).expect_err("must refuse");
        let message = error.to_string();

        assert!(
            message.contains("pm.status"),
            "the refusal must name the next available action: {message}"
        );
        assert!(
            pm_registry::load_pm_prefs(&prefs)
                .expect("prefs")
                .registration
                .is_some(),
            "a refused stop must not touch the registration"
        );
    }

    #[test]
    fn stop_refuses_when_the_caller_has_no_ambient_session_identity() {
        let _lock = env_lock();
        let fixture = Fixture::new(&["store-a"]);
        fixture.register(0, "the-pm");
        let _caller = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV);

        let error = stop_pm(&fixture.repo, Some("the-pm")).expect_err("must refuse");
        let message = error.to_string();

        assert!(
            message.contains(gwt_agent::GWT_SESSION_ID_ENV),
            "the refusal must name the missing identity: {message}"
        );
        assert!(
            message.contains("relaunch the Session"),
            "the refusal must name the next available action: {message}"
        );
    }

    /// A PM of *another* repository must not reach into this one.
    #[test]
    fn stop_refuses_a_session_registered_for_another_repository() {
        let _lock = env_lock();
        let fixture = Fixture::new(&["store-a"]);
        fixture.register(0, "the-pm");
        let _caller = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "the-pm");

        let error = stop_pm(&fixture.repo, Some("a-pm-somewhere-else")).expect_err("must refuse");

        assert!(
            error.to_string().contains("repository_registrations"),
            "the refusal must point at where valid session ids come from: {error}"
        );
    }

    /// AC-5: a PM can always retire itself, which is the orphan's own way out.
    #[test]
    fn stop_defaults_to_the_callers_own_registration() {
        let _lock = env_lock();
        let fixture = Fixture::new(&["store-a"]);
        let prefs = fixture.register(0, "self-retiring-pm");
        fixture.save_session(0, "self-retiring-pm");
        let _caller = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "self-retiring-pm");

        let report = stop_pm(&fixture.repo, None).expect("self stop");

        assert!(report.stopped_self);
        assert_eq!(report.stopped_session_id, "self-retiring-pm");
        assert_eq!(
            pm_registry::load_pm_prefs(&prefs)
                .expect("prefs")
                .registration,
            None
        );
    }

    /// `pm.status` is where the orphan's session id has to come from, so it
    /// must show registrations this store's own `pm.json` cannot see.
    #[test]
    fn status_lists_registrations_from_every_store_of_the_repository() {
        let _lock = env_lock();
        let fixture = Fixture::new(&["99a8660247f5bc49", "b19aac38305901f5"]);
        fixture.register(0, "fedf798b-live");
        fixture.register(1, "b0801016-orphan");

        let views = pm_registry::pm_repository_registration_views(&fixture.repo);

        let sessions: Vec<&str> = views.iter().map(|view| view.session_id.as_str()).collect();
        assert!(sessions.contains(&"fedf798b-live"), "{sessions:?}");
        assert!(sessions.contains(&"b0801016-orphan"), "{sessions:?}");
        assert!(
            views.iter().any(|view| !view.is_current_store),
            "the orphan's store must be flagged as a different store: {views:?}"
        );
    }
}
