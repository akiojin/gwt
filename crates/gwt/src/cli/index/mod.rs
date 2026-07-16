//! `index.*` JSON operation family module (SPEC-1942 SC-027 split).
//!
//! - `mod.rs` (this file): argv `parse`, top-level `run` dispatch, the
//!   command implementations (`run_status` / `run_rebuild`), the family
//!   `tests` block, and shared error helpers (`io_error` / `runtime_error`).
//! - `runtime.rs`: [`runtime::IndexContext`] + Python runner integration
//!   (`run_runner_status` / `run_runner_rebuild` / `parse_runner_json` /
//!   `render_index_status` / `format_runner_failure`).
//! - `audit.rs`: append-only JSONL audit logging for `index status` and
//!   `index rebuild`.

mod audit;
pub mod runtime;

use gwt_github::{client::ApiError, SpecOpsError};

use super::{CliEnv, CliParseError};

use audit::{
    audit_log_dir, audit_rebuild_result, audit_rebuild_start, audit_runner_progress, audit_status,
    audit_status_failure,
};
use runtime::{
    format_runner_failure, parse_runner_json, rebuild_actions, render_index_status,
    resolve_index_context, run_runner_rebuild, run_runner_status,
};

/// SPEC-1942 command model for `index.*` JSON operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexCommand {
    /// `index.status`.
    Status,
    /// `index.rebuild`.
    Rebuild { scope: IndexScope },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexScope {
    All,
    Issues,
    Specs,
    Memory,
    Discussions,
    Board,
    Files,
    FilesDocs,
}

pub fn parse(args: &[String]) -> Result<IndexCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    match head.as_str() {
        "status" => {
            if !rest.is_empty() {
                return Err(CliParseError::Usage);
            }
            Ok(IndexCommand::Status)
        }
        "rebuild" => Ok(IndexCommand::Rebuild {
            scope: parse_rebuild_scope(rest)?,
        }),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn parse_rebuild_scope(args: &[String]) -> Result<IndexScope, CliParseError> {
    if args.is_empty() {
        return Ok(IndexScope::All);
    }
    if args.len() != 2 || args[0] != "--scope" {
        return Err(CliParseError::Usage);
    }
    match args[1].as_str() {
        "all" => Ok(IndexScope::All),
        "issues" => Ok(IndexScope::Issues),
        "specs" => Ok(IndexScope::Specs),
        "memory" => Ok(IndexScope::Memory),
        "discussions" => Ok(IndexScope::Discussions),
        "board" => Ok(IndexScope::Board),
        "files" => Ok(IndexScope::Files),
        "files-docs" => Ok(IndexScope::FilesDocs),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

pub fn run<E: CliEnv>(
    env: &mut E,
    cmd: IndexCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match cmd {
        IndexCommand::Status => run_status(env, out),
        IndexCommand::Rebuild { scope } => run_rebuild(env, scope, out),
    }
}

fn run_status<E: CliEnv>(env: &mut E, out: &mut String) -> Result<i32, SpecOpsError> {
    let context = resolve_index_context(env.repo_path())?;
    let report = gwt_core::runtime::ensure_project_index_runtime().map_err(runtime_error)?;
    let output = run_runner_status(&context)?;
    if !output.status.success() {
        out.push_str("runtime: error\n");
        out.push_str(&format_runner_failure(&output));
        let log_dir = audit_log_dir(&context);
        let _ = audit_status_failure(
            &log_dir,
            &context,
            &format_runner_failure(&output),
            output.status.code().unwrap_or(1),
        );
        return Ok(1);
    }

    let payload = parse_runner_json(&output.stdout)?;
    render_index_status(out, &report, &payload);
    let log_dir = audit_log_dir(&context);
    let _ = audit_status(&log_dir, &context, &report, &payload, 0);
    Ok(0)
}

fn run_rebuild<E: CliEnv>(
    env: &mut E,
    scope: IndexScope,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let context = resolve_index_context(env.repo_path())?;
    let report = gwt_core::runtime::ensure_project_index_runtime().map_err(runtime_error)?;
    out.push_str(&format!(
        "runtime: ready asset={} smoke={}\n",
        report.runner_hash,
        if report.runner_smoke_tested {
            "passed"
        } else {
            "skipped"
        }
    ));

    let mut ok = true;
    let log_dir = audit_log_dir(&context);
    for action in rebuild_actions(scope) {
        let _ = audit_rebuild_start(&log_dir, &context, action.label);
        let coordinator_worktree = action
            .needs_worktree_hash
            .then(|| context.worktree_hash.clone());
        // Manual rebuilds coordinate host-wide like every other index build
        // (SPEC #1939 Phase 70 FR-379/FR-383): at most one heavy runner tree,
        // manual priority above background repair.
        let run = crate::index_worker::run_coordinated_index_job(
            context.repo_hash.as_str(),
            action.label,
            coordinator_worktree.as_deref(),
            gwt_core::index_coordinator::JobPriority::ManualRebuild,
            || {
                let output = run_runner_rebuild(&context, action).map_err(|err| err.to_string())?;
                let _ = audit_runner_progress(&log_dir, &context, action.label, &output.stderr);
                let _ = audit_rebuild_result(&log_dir, &context, action.label, &output);
                if output.status.success() {
                    Ok(())
                } else {
                    Err(format_runner_failure(&output))
                }
            },
        );
        match run {
            Ok(_) => out.push_str(&format!("{}: ok\n", action.label)),
            Err(error) => {
                ok = false;
                out.push_str(&format!("{}: error\n", action.label));
                out.push_str(&error);
                if !error.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }

    Ok(if ok { 0 } else { 1 })
}

fn runtime_error(err: gwt_core::GwtError) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(err.to_string()))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::runtime::IndexContext;
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn run_git_at(path: &Path, args: &[&str]) {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(path)
            .output()
            .unwrap_or_else(|err| panic!("git {args:?}: {err}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn make_bare_workspace_with_worktree(home: &Path) -> PathBuf {
        let bare = home.join("gwt.git");
        let bootstrap = home.join(".bootstrap");
        let develop = home.join("develop");
        std::fs::create_dir_all(home).expect("workspace home");
        run_git_at(home, &["init", "--bare", bare.to_str().unwrap()]);
        run_git_at(
            &bare,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/example/gwt.git",
            ],
        );
        run_git_at(home, &["clone", bare.to_str().unwrap(), ".bootstrap"]);
        run_git_at(&bootstrap, &["config", "user.email", "test@example.com"]);
        run_git_at(&bootstrap, &["config", "user.name", "Test User"]);
        run_git_at(&bootstrap, &["checkout", "-b", "develop"]);
        run_git_at(&bootstrap, &["commit", "--allow-empty", "-m", "init"]);
        run_git_at(&bootstrap, &["push", "origin", "develop"]);
        run_git_at(
            &bare,
            &["worktree", "add", develop.to_str().unwrap(), "develop"],
        );
        std::fs::remove_dir_all(&bootstrap).expect("remove bootstrap");
        develop
    }

    #[test]
    fn parses_index_status_and_rebuild_scope() {
        assert_eq!(parse(&s(&["status"])).unwrap(), IndexCommand::Status);
        assert_eq!(
            parse(&s(&["rebuild", "--scope", "files-docs"])).unwrap(),
            IndexCommand::Rebuild {
                scope: IndexScope::FilesDocs
            }
        );
        assert_eq!(
            parse(&s(&["rebuild"])).unwrap(),
            IndexCommand::Rebuild {
                scope: IndexScope::All
            }
        );
    }

    #[test]
    fn parses_index_rebuild_memory_scope() {
        assert_eq!(
            parse(&s(&["rebuild", "--scope", "memory"])).unwrap(),
            IndexCommand::Rebuild {
                scope: IndexScope::Memory
            }
        );
    }

    #[test]
    fn parses_index_rebuild_discussions_scope() {
        assert_eq!(
            parse(&s(&["rebuild", "--scope", "discussions"])).unwrap(),
            IndexCommand::Rebuild {
                scope: IndexScope::Discussions
            }
        );
    }

    #[test]
    fn parses_index_rebuild_board_scope() {
        assert_eq!(
            parse(&s(&["rebuild", "--scope", "board"])).unwrap(),
            IndexCommand::Rebuild {
                scope: IndexScope::Board
            }
        );
    }

    #[test]
    fn index_context_uses_active_worktree_for_workspace_home() {
        let temp = tempfile::tempdir().expect("tempdir");
        let develop = make_bare_workspace_with_worktree(temp.path());

        let context = super::runtime::resolve_index_context(temp.path()).expect("index context");

        assert_eq!(
            dunce::canonicalize(&context.project_root).expect("context root"),
            dunce::canonicalize(&develop).expect("develop root")
        );
        assert_eq!(
            context.repo_hash.as_str(),
            gwt_core::repo_hash::compute_repo_hash("https://github.com/example/gwt.git").as_str()
        );
        assert_eq!(
            context.worktree_hash,
            gwt_core::worktree_hash::compute_worktree_hash(&develop)
                .expect("develop hash")
                .to_string()
        );
    }

    #[test]
    fn audit_log_dir_uses_project_scoped_gwt_log_directory() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path().join("repo");
        std::fs::create_dir_all(&project_root).unwrap();
        let project_hash = gwt_core::repo_hash::compute_path_hash(&project_root);
        let context = IndexContext {
            project_root,
            repo_hash: gwt_core::repo_hash::compute_repo_hash(
                "https://github.com/example/project.git",
            ),
            worktree_hash: "feedfacecafebeef".to_string(),
            python: PathBuf::from("python"),
            runner: PathBuf::from("runner.py"),
        };

        let log_dir = audit_log_dir(&context);

        assert!(log_dir.ends_with(
            PathBuf::from("projects")
                .join(project_hash.as_str())
                .join("logs")
        ));
        assert!(!log_dir.ends_with(PathBuf::from(".gwt").join("logs")));
    }

    #[test]
    fn renders_runtime_and_scope_health() {
        let report = gwt_core::runtime::ProjectIndexRuntimeReport {
            runner_hash: "aaaaaaaaaaaaaaaa".to_string(),
            requirements_hash: "bbbbbbbbbbbbbbbb".to_string(),
            runner_smoke_tested: true,
            ..Default::default()
        };
        let payload = serde_json::json!({
            "runtime": {
                "healthy": true,
                "reason": "ready",
                "asset_hash": "cccccccccccccccc",
                "smoke_test": "passed"
            },
            "status": {
                "files": {
                    "healthy": false,
                    "repair_required": true,
                    "reason": "manifest_missing",
                    "document_count": 3
                }
            }
        });
        let mut out = String::new();
        render_index_status(&mut out, &report, &payload);
        assert!(out.contains("runtime: ready asset=cccccccccccccccc smoke=passed"));
        assert!(out
            .contains("files: unhealthy reason=manifest_missing documents=3 repair_required=true"));
    }

    #[test]
    fn renders_memory_scope_health() {
        let report = gwt_core::runtime::ProjectIndexRuntimeReport {
            runner_hash: "aaaaaaaaaaaaaaaa".to_string(),
            requirements_hash: "bbbbbbbbbbbbbbbb".to_string(),
            runner_smoke_tested: true,
            ..Default::default()
        };
        let payload = serde_json::json!({
            "runtime": {
                "healthy": true,
                "reason": "ready",
                "asset_hash": "cccccccccccccccc",
                "smoke_test": "passed"
            },
            "status": {
                "memory": {
                    "healthy": true,
                    "repair_required": false,
                    "reason": "ready",
                    "document_count": 234
                }
            }
        });
        let mut out = String::new();
        render_index_status(&mut out, &report, &payload);
        assert!(
            out.contains("memory: ready reason=ready documents=234 repair_required=false"),
            "render output missing memory line:\n{out}"
        );
    }

    #[test]
    fn audit_status_writes_to_unified_gwt_log_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let context = IndexContext {
            project_root: dir.path().join("repo"),
            repo_hash: gwt_core::repo_hash::compute_repo_hash(
                "https://github.com/example/project.git",
            ),
            worktree_hash: "feedfacecafebeef".to_string(),
            python: PathBuf::from("python"),
            runner: PathBuf::from("runner.py"),
        };
        let report = gwt_core::runtime::ProjectIndexRuntimeReport {
            runner_hash: "aaaaaaaaaaaaaaaa".to_string(),
            requirements_hash: "bbbbbbbbbbbbbbbb".to_string(),
            runner_smoke_tested: true,
            runner_updated: true,
            ..Default::default()
        };
        let payload = serde_json::json!({
            "runtime": {
                "reason": "ready",
                "asset_hash": "aaaaaaaaaaaaaaaa",
                "smoke_test": "passed"
            },
            "status": {
                "files-docs": {
                    "healthy": true,
                    "repair_required": false,
                    "reason": "ready",
                    "document_count": 16,
                    "last_repair_at": "2026-04-24T06:15:20Z"
                }
            }
        });

        audit_status(dir.path(), &context, &report, &payload, 0).unwrap();

        let log_path = gwt_core::logging::current_log_file(dir.path());
        let content = std::fs::read_to_string(log_path).unwrap();
        assert!(content.contains("\"target\":\"gwt::index\""), "{content}");
        assert!(
            content.contains("\"message\":\"project_index_status\""),
            "{content}"
        );
        assert!(content.contains("\"status\":\"ready\""), "{content}");
        assert!(content.contains("\"repair_required\":false"), "{content}");
        assert!(content.contains("\"runner\":true"), "{content}");
        assert!(content.contains("\"files-docs\""), "{content}");
        assert!(
            content.contains("\"last_repair_at\":\"2026-04-24T06:15:20Z\""),
            "{content}"
        );
        assert!(!dir.path().join("index.log").exists());
    }

    #[test]
    fn audit_status_marks_unhealthy_scope_as_repair_required() {
        let dir = tempfile::tempdir().unwrap();
        let context = IndexContext {
            project_root: dir.path().join("repo"),
            repo_hash: gwt_core::repo_hash::compute_repo_hash(
                "https://github.com/example/project.git",
            ),
            worktree_hash: "feedfacecafebeef".to_string(),
            python: PathBuf::from("python"),
            runner: PathBuf::from("runner.py"),
        };
        let report = gwt_core::runtime::ProjectIndexRuntimeReport {
            runner_hash: "aaaaaaaaaaaaaaaa".to_string(),
            requirements_hash: "bbbbbbbbbbbbbbbb".to_string(),
            runner_smoke_tested: true,
            ..Default::default()
        };
        let payload = serde_json::json!({
            "status": {
                "specs": {
                    "healthy": false,
                    "repair_required": true,
                    "reason": "count_mismatch",
                    "document_count": 1
                },
                "files": {
                    "healthy": true,
                    "repair_required": false,
                    "reason": "ready",
                    "document_count": 310
                }
            }
        });

        audit_status(dir.path(), &context, &report, &payload, 0).unwrap();

        let log_path = gwt_core::logging::current_log_file(dir.path());
        let content = std::fs::read_to_string(log_path).unwrap();
        assert!(
            content.contains("\"status\":\"repair_required\""),
            "{content}"
        );
        assert!(content.contains("\"repair_required\":true"), "{content}");
        assert!(
            content.contains("\"unhealthy_scopes\":[\"specs\"]"),
            "{content}"
        );
        assert!(
            content.contains("\"reason\":\"count_mismatch\""),
            "{content}"
        );
        assert!(!dir.path().join("index.log").exists());
    }

    #[test]
    fn audit_runner_progress_translates_stderr_ndjson_to_unified_log() {
        let dir = tempfile::tempdir().unwrap();
        let context = IndexContext {
            project_root: dir.path().join("repo"),
            repo_hash: gwt_core::repo_hash::compute_repo_hash(
                "https://github.com/example/project.git",
            ),
            worktree_hash: "feedfacecafebeef".to_string(),
            python: PathBuf::from("python"),
            runner: PathBuf::from("runner.py"),
        };
        let stderr = br#"{"phase":"indexing","scope":"files-docs","done":0,"total":16}
not json
{"phase":"complete","scope":"files-docs","indexed":16,"total":16}
"#;

        audit_runner_progress(dir.path(), &context, "files-docs", stderr).unwrap();

        let log_path = gwt_core::logging::current_log_file(dir.path());
        let content = std::fs::read_to_string(log_path).unwrap();
        assert_eq!(content.lines().count(), 2);
        assert!(content.contains("\"phase\":\"indexing\""), "{content}");
        assert!(content.contains("\"phase\":\"complete\""), "{content}");
        assert!(content.contains("\"scope\":\"files-docs\""), "{content}");
        assert!(!content.contains("not json"), "{content}");
        assert!(!dir.path().join("index.log").exists());
    }
}
