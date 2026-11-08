//! Warning-only, session-local evidence of a PM turn's successful git fetch.

use std::{
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{segments::split_command_segments, HookEvent};

#[derive(Deserialize, Serialize)]
struct TurnReceipt {
    schema_version: u32,
    fetched: bool,
}

/// Return advisory text only. Optional bookkeeping failures must never deny a
/// tool call or turn boundary, and a provider's payload is never PM identity.
pub(crate) fn handle_with_input(
    event: &str,
    input: &str,
    worktree_root: &Path,
    current_session: Option<&str>,
) -> Option<String> {
    if !matches!(
        event,
        "SessionStart" | "UserPromptSubmit" | "PreToolUse" | "PostToolUse"
    ) {
        return None;
    }
    let session = current_session?;
    let repository_root = repository_root_for_cwd(worktree_root);
    let identity_root = repository_root.as_deref().unwrap_or(worktree_root);
    let project_store = if crate::pm_registry::is_canonical_pm_worktree(identity_root) {
        // HOME may retain /var while the resolved cwd uses /private/var.
        identity_root.parent()?.parent()?.to_path_buf()
    } else {
        crate::board_read_receipts::project_store(identity_root)
    };
    let prefs_path = project_store.join("project-state/pm.json");
    if !crate::pm_registry::session_is_registered_pm(&prefs_path, session) {
        if let Err(error) = crate::pm_registry::load_pm_prefs(&prefs_path) {
            return Some(unavailable(worktree_root, error));
        }
        return None;
    }
    let Some(repository_root) = repository_root else {
        return Some(unavailable(worktree_root, "repository identity is unknown"));
    };
    let worktree_root = repository_root.as_path();
    let Some(common_dir) = gwt_core::repo_hash::repository_common_dir(worktree_root) else {
        return Some(unavailable(worktree_root, "repository identity is unknown"));
    };
    // Neither session IDs nor repository spellings become path components.
    // A common-dir key also separates independent clones with the same origin.
    let path = project_store
        .join("pm/branch-freshness")
        .join(digest(common_dir.as_os_str().as_encoded_bytes()))
        .join(format!("{}.json", digest(session.as_bytes())));
    if matches!(event, "SessionStart" | "UserPromptSubmit") {
        return save_receipt(&path, false)
            .err()
            .map(|error| unavailable(worktree_root, error));
    }
    let payload: Value = match serde_json::from_str(input) {
        Ok(payload) => payload,
        Err(error) => return Some(unavailable(worktree_root, error)),
    };
    let parsed: HookEvent = match serde_json::from_value(payload.clone()) {
        Ok(parsed) => parsed,
        Err(error) => return Some(unavailable(worktree_root, error)),
    };
    if !matches!(
        parsed.tool_name.as_deref(),
        Some("Bash" | "exec_command" | "functions.exec_command")
    ) {
        return None;
    }
    let command = parsed.command()?;
    let cwd = parsed.command_cwd(worktree_root);
    let in_repository = |cwd: &Path| {
        repository_root_for_cwd(cwd)
            .and_then(|root| gwt_core::repo_hash::repository_common_dir(&root))
            .is_some_and(|dir| dir == common_dir)
    };
    if event == "PostToolUse" {
        // A compound command's final status cannot in general prove fetch
        // succeeded (`git fetch || true`). Keep uncertain forms advisory-only.
        if command.contains([';', '&', '|', '\n', '\r', '$', '`', '<', '>', '(', ')']) {
            return None;
        }
        let invocation = git_invocation(command, &cwd)?;
        let fetch_scope = invocation
            .args
            .iter()
            .map(String::as_str)
            .filter(|arg| {
                !matches!(
                    *arg,
                    "--prune"
                        | "-p"
                        | "--tags"
                        | "--quiet"
                        | "-q"
                        | "--verbose"
                        | "-v"
                        | "--force"
                        | "-f"
                        | "--atomic"
                )
            })
            .collect::<Vec<_>>();
        if invocation.subcommand != "fetch"
            || !invocation.receipt_safe
            || !matches!(fetch_scope.as_slice(), ["origin"] | ["--all"])
            || !in_repository(&invocation.cwd)
            || !successful_response(&payload)
        {
            return None;
        }
        return save_receipt(&path, true)
            .err()
            .map(|error| unavailable(worktree_root, error));
    }
    let inspection = split_command_segments(command).iter().any(|segment| {
        git_invocation(segment, &cwd).is_some_and(|invocation| {
            matches!(
                invocation.subcommand.as_str(),
                "status" | "log" | "diff" | "rev-list" | "rev-parse" | "branch" | "show"
            ) && in_repository(&invocation.cwd)
        })
    });
    if !inspection {
        return None;
    }
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<TurnReceipt>(&bytes) {
            Ok(receipt) if receipt.schema_version == 1 && receipt.fetched => None,
            Ok(receipt) if receipt.schema_version == 1 => Some(warning(worktree_root)),
            Ok(_) => Some(unavailable(worktree_root, "unsupported receipt schema")),
            Err(error) => Some(unavailable(worktree_root, error)),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Some(warning(worktree_root)),
        Err(error) => Some(unavailable(worktree_root, error)),
    }
}

/// Resolve an existing cwd to its nearest actual Git boundary. Layout roots
/// containing sibling bare repositories are not themselves command targets.
fn repository_root_for_cwd(cwd: &Path) -> Option<PathBuf> {
    let cwd = dunce::canonicalize(cwd).ok()?;
    if !cwd.is_dir() {
        return None;
    }
    for candidate in cwd.ancestors() {
        let has_marker = match std::fs::symlink_metadata(candidate.join(".git")) {
            Ok(_) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(_) => return None,
        };
        if has_marker
            || (candidate.join("HEAD").is_file()
                && candidate.join("objects").is_dir()
                && candidate.join("refs").is_dir())
        {
            // Even a broken marker fences an enclosing repository: resolving
            // it must not credit a fetch against that outer repository.
            return Some(candidate.to_path_buf());
        }
        if let Some(worktree) = crate::pm_registry::pm_worktree_for_runtime_dir(candidate) {
            return Some(worktree);
        }
    }
    None
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn save_receipt(path: &Path, fetched: bool) -> io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("receipt has parent"))?;
    let bytes = serde_json::to_vec(&TurnReceipt {
        schema_version: 1,
        fetched,
    })?;
    gwt_core::atomic_file::write_atomic(path, &bytes)
}

fn warning(worktree: &Path) -> String {
    let quoted = worktree.to_string_lossy().replace('\'', "'\\''");
    format!(
        "Branch freshness warning: this PM turn has no confirmed successful `git fetch origin` \
         for this repository. Run `git -C '{quoted}' fetch origin`, then repeat the branch \
         inspection. Execution is allowed."
    )
}

fn unavailable(worktree: &Path, error: impl std::fmt::Display) -> String {
    eprintln!("gwtd hook branch-freshness: optional evidence unavailable: {error}");
    format!(
        "Branch freshness warning: optional evidence is unavailable ({error}); freshness is \
         unknown. {}",
        warning(worktree)
    )
}

fn successful_response(payload: &Value) -> bool {
    let Some(response) = payload
        .get("tool_response")
        .or_else(|| payload.get("toolResponse"))
    else {
        return false;
    };
    if ["interrupted", "is_error", "isError", "timed_out"]
        .iter()
        .any(|field| response.get(field).and_then(Value::as_bool) == Some(true))
        || response.get("success").and_then(Value::as_bool) == Some(false)
    {
        return false;
    }
    response
        .get("exit_code")
        .or_else(|| response.get("exitCode"))
        .and_then(Value::as_i64)
        == Some(0)
}

struct GitInvocation {
    cwd: PathBuf,
    subcommand: String,
    args: Vec<String>,
    receipt_safe: bool,
}

/// Recognize direct git invocations only. Uncertain shell syntax may cause an
/// extra warning, but must never manufacture a successful fetch receipt.
fn git_invocation(command: &str, cwd: &Path) -> Option<GitInvocation> {
    let words = crate::cli::verification_record::split_command_line(command).ok()?;
    if !matches!(words.first().map(String::as_str), Some("git" | "git.exe")) {
        return None;
    }
    let mut target = cwd.to_path_buf();
    let mut index = 1;
    let mut receipt_safe = true;
    loop {
        match words.get(index)?.as_str() {
            "-C" => {
                target = target.join(words.get(index + 1)?);
                index += 2;
            }
            "--no-pager" | "--no-optional-locks" => index += 1,
            "-c" | "--git-dir" | "--work-tree" | "--config-env" | "--namespace" => {
                words.get(index + 1)?;
                receipt_safe = false;
                index += 2;
            }
            option if option.starts_with('-') => {
                receipt_safe = false;
                index += 1;
            }
            subcommand => {
                return Some(GitInvocation {
                    cwd: target,
                    subcommand: subcommand.to_string(),
                    args: words[index + 1..].to_vec(),
                    receipt_safe,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedGwtHome;
    use serde_json::json;

    fn repository(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        assert!(gwt_core::process::hidden_command("git")
            .args(["init", "--quiet"])
            .arg(path)
            .status()
            .unwrap()
            .success());
    }

    fn register_pm(repo: &Path, session: &str) {
        let prefs = crate::pm_registry::PmPrefs {
            registration: Some(crate::pm_registry::PmRegistration {
                session_id: session.to_string(),
                agent_id: "codex".to_string(),
                worktree_path: repo.to_string_lossy().into_owned(),
                created_at: None,
                consecutive_crashes: 0,
                next_not_before: None,
            }),
            ..Default::default()
        };
        crate::pm_registry::save_pm_prefs(
            &crate::pm_registry::pm_prefs_path_for_repo_path(repo),
            &prefs,
        )
        .unwrap();
    }

    fn payload(command: &str, cwd: &Path, response: serde_json::Value) -> String {
        json!({
            "tool_name": "Bash", "tool_input": { "command": command },
            "cwd": cwd, "tool_response": response,
        })
        .to_string()
    }

    #[test]
    fn branch_inspection_without_current_turn_fetch_warns_with_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repo");
        repository(&repo);
        register_pm(&repo, "pm-a");

        let warning = handle_with_input(
            "PreToolUse",
            &payload("git status --short", &repo, json!(null)),
            &repo,
            Some("pm-a"),
        )
        .expect("missing fetch must produce an advisory");
        assert!(warning.contains("git fetch origin"));
        assert!(warning.contains("warning"));
    }

    #[test]
    fn successful_fetch_is_current_turn_only_and_reset_by_both_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repo");
        repository(&repo);
        register_pm(&repo, "pm-a");

        for boundary in ["SessionStart", "UserPromptSubmit"] {
            assert!(handle_with_input(boundary, "{}", &repo, Some("pm-a")).is_none());
            assert!(handle_with_input(
                "PostToolUse",
                &payload("git fetch origin", &repo, json!({"exit_code": 0})),
                &repo,
                Some("pm-a"),
            )
            .is_none());
            assert!(handle_with_input(
                "PreToolUse",
                &payload("git log -1", &repo, json!(null)),
                &repo,
                Some("pm-a"),
            )
            .is_none());
            handle_with_input(boundary, "{}", &repo, Some("pm-a"));
            assert!(handle_with_input(
                "PreToolUse",
                &payload("git log -1", &repo, json!(null)),
                &repo,
                Some("pm-a"),
            )
            .is_some());
        }
    }

    #[test]
    fn failed_unknown_or_interrupted_fetch_never_creates_success_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repo");
        repository(&repo);
        register_pm(&repo, "pm-a");

        for response in [
            json!({"exit_code": 1}),
            json!({"stdout": "success"}),
            json!({"exit_code": "0"}),
            json!({"exit_code": 0, "interrupted": true}),
            json!(null),
        ] {
            handle_with_input("UserPromptSubmit", "{}", &repo, Some("pm-a"));
            handle_with_input(
                "PostToolUse",
                &payload("git fetch origin", &repo, response.clone()),
                &repo,
                Some("pm-a"),
            );
            assert!(
                handle_with_input(
                    "PreToolUse",
                    &payload("git diff origin/develop", &repo, json!(null)),
                    &repo,
                    Some("pm-a"),
                )
                .is_some(),
                "unconfirmed response: {response}"
            );
        }
    }

    #[test]
    fn quoted_echo_compound_dry_run_and_foreign_repository_fetch_do_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repo");
        let foreign = dir.path().join("foreign");
        repository(&repo);
        repository(&foreign);
        register_pm(&repo, "pm-a");

        for (command, cwd) in [
            ("echo 'git fetch origin'".to_string(), repo.as_path()),
            ("printf \"git fetch origin\"".to_string(), repo.as_path()),
            ("git fetch origin || true".to_string(), repo.as_path()),
            ("git fetch --dry-run origin".to_string(), repo.as_path()),
            (
                format!("git -C '{}' fetch origin", foreign.display()),
                repo.as_path(),
            ),
            ("git fetch origin".to_string(), foreign.as_path()),
            (
                "cd ../foreign && git fetch origin".to_string(),
                repo.as_path(),
            ),
        ] {
            handle_with_input("UserPromptSubmit", "{}", &repo, Some("pm-a"));
            handle_with_input(
                "PostToolUse",
                &payload(&command, cwd, json!({"exit_code": 0})),
                &repo,
                Some("pm-a"),
            );
            assert!(
                handle_with_input(
                    "PreToolUse",
                    &payload("git rev-list --count HEAD", &repo, json!(null)),
                    &repo,
                    Some("pm-a"),
                )
                .is_some(),
                "false success receipt for {command}"
            );
        }
    }

    #[test]
    fn advisory_requires_exact_registered_pm_and_a_branch_inspection() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repo");
        repository(&repo);
        register_pm(&repo, "pm-a");

        for session in [None, Some("agent-a"), Some(" pm-a"), Some("pm-a ")] {
            assert!(handle_with_input(
                "PreToolUse",
                &payload("git status", &repo, json!(null)),
                &repo,
                session,
            )
            .is_none());
        }
        for command in ["echo 'git status'", "cargo test", "git fetch origin"] {
            assert!(handle_with_input(
                "PreToolUse",
                &payload(command, &repo, json!(null)),
                &repo,
                Some("pm-a"),
            )
            .is_none());
        }
    }

    #[test]
    fn receipts_are_session_and_repository_scoped_with_path_safe_session_keys() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repo");
        let other = dir.path().join("other");
        repository(&repo);
        repository(&other);
        for repository in [&repo, &other] {
            assert!(gwt_core::process::hidden_command("git")
                .arg("-C")
                .arg(repository)
                .args([
                    "config",
                    "remote.origin.url",
                    "https://example.com/shared.git"
                ])
                .status()
                .unwrap()
                .success());
        }
        let first_session = "../../outside/session";
        register_pm(&repo, first_session);
        register_pm(&other, first_session);
        handle_with_input(
            "PostToolUse",
            &payload("git fetch origin", &repo, json!({"exit_code": 0})),
            &repo,
            Some(first_session),
        );
        assert!(handle_with_input(
            "PreToolUse",
            &payload("git branch --all", &repo, json!(null)),
            &repo,
            Some(first_session),
        )
        .is_none());
        assert!(handle_with_input(
            "PreToolUse",
            &payload("git branch --all", &other, json!(null)),
            &other,
            Some(first_session),
        )
        .is_some());
        register_pm(&repo, "replacement-pm");
        assert!(handle_with_input(
            "PreToolUse",
            &payload("git rev-parse HEAD", &repo, json!(null)),
            &repo,
            Some("replacement-pm"),
        )
        .is_some());
        assert!(!dir.path().join("outside").exists());
    }

    #[test]
    fn git_c_and_codex_workdir_are_resolved_before_fetch_is_credited() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repo");
        let foreign = dir.path().join("foreign");
        repository(&repo);
        repository(&foreign);
        register_pm(&repo, "pm-a");

        handle_with_input(
            "PostToolUse",
            &payload(
                &format!("git -C '{}' fetch origin", repo.display()),
                &foreign,
                json!({"exitCode":0}),
            ),
            &repo,
            Some("pm-a"),
        );
        assert!(handle_with_input(
            "PreToolUse",
            &payload("git status", &repo, json!(null)),
            &repo,
            Some("pm-a"),
        )
        .is_none());
        handle_with_input("UserPromptSubmit", "{}", &repo, Some("pm-a"));
        handle_with_input(
            "PostToolUse",
            &json!({"tool_name":"exec_command", "tool_input":{"cmd":"git fetch origin", "workdir":foreign},
                "cwd":repo, "tool_response":{"exit_code":0}}).to_string(),
            &repo, Some("pm-a"),
        );
        assert!(handle_with_input(
            "PreToolUse",
            &payload("git status", &repo, json!(null)),
            &repo,
            Some("pm-a"),
        )
        .is_some());
    }

    #[test]
    fn corrupt_or_unwritable_receipts_return_diagnostic_advisories() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repo");
        repository(&repo);
        register_pm(&repo, "pm-a");
        handle_with_input("UserPromptSubmit", "{}", &repo, Some("pm-a"));

        let root =
            gwt_core::paths::gwt_project_dir_for_repo_path(&repo).join("pm/branch-freshness");
        let repository_dir = std::fs::read_dir(&root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let receipt = std::fs::read_dir(repository_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(receipt.file_stem().unwrap().to_string_lossy().len(), 64);
        std::fs::write(&receipt, b"not json").unwrap();
        let input = payload("git status", &repo, json!(null));
        let warning = handle_with_input("PreToolUse", &input, &repo, Some("pm-a")).unwrap();
        assert!(warning.contains("unavailable"));
        assert!(warning.contains("git fetch origin"));

        std::fs::remove_file(&receipt).unwrap();
        std::fs::create_dir(&receipt).unwrap();
        assert!(
            handle_with_input("UserPromptSubmit", "{}", &repo, Some("pm-a"))
                .unwrap()
                .contains("unavailable")
        );
        assert!(handle_with_input("PreToolUse", &input, &repo, Some("pm-a"))
            .unwrap()
            .contains("unavailable"));
    }

    #[test]
    fn other_remote_or_restricted_refspec_does_not_refresh_origin_branches() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repo");
        repository(&repo);
        register_pm(&repo, "pm-a");
        let inspection = payload("git log origin/develop -1", &repo, json!(null));
        for command in ["git fetch other-remote", "git fetch origin unrelated-ref"] {
            handle_with_input("UserPromptSubmit", "{}", &repo, Some("pm-a"));
            handle_with_input(
                "PostToolUse",
                &payload(command, &repo, json!({"exit_code":0})),
                &repo,
                Some("pm-a"),
            );
            assert!(
                handle_with_input("PreToolUse", &inspection, &repo, Some("pm-a")).is_some(),
                "unrelated fetch {command}"
            );
        }
        handle_with_input(
            "PostToolUse",
            &payload("git fetch --all --prune", &repo, json!({"exit_code":0})),
            &repo,
            Some("pm-a"),
        );
        assert!(handle_with_input("PreToolUse", &inspection, &repo, Some("pm-a")).is_none());
    }

    #[test]
    fn advertised_recovery_is_recognized_for_a_repository_path_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repository with spaces");
        repository(&repo);
        register_pm(&repo, "pm-a");
        let inspection = payload("git status", &repo, json!(null));
        let advisory = handle_with_input("PreToolUse", &inspection, &repo, Some("pm-a")).unwrap();
        let recovery = advisory
            .split("Run `")
            .nth(1)
            .unwrap()
            .split('`')
            .next()
            .unwrap();
        handle_with_input(
            "PostToolUse",
            &payload(recovery, &repo, json!({"exit_code":0})),
            &repo,
            Some("pm-a"),
        );
        assert!(handle_with_input("PreToolUse", &inspection, &repo, Some("pm-a")).is_none());
    }

    #[test]
    fn canonical_pm_worktree_nested_cwd_and_runtime_share_the_registered_turn() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let store = gwt_core::paths::gwt_projects_dir().join("canonical-pm-store");
        let repo = store.join("pm/worktree");
        repository(&repo);
        let nested = repo.join("src");
        std::fs::create_dir(&nested).unwrap();
        let runtime = crate::pm_registry::ensure_pm_runtime_dir_for_pm_worktree(&repo).unwrap();
        let prefs = store.join("project-state/pm.json");
        std::fs::create_dir_all(prefs.parent().unwrap()).unwrap();
        std::fs::write(
            prefs,
            json!({"registration":{"session_id":"pm-a", "agent_id":"codex", "worktree_path":repo}})
                .to_string(),
        )
        .unwrap();
        let inspection = payload("git status", &repo, json!(null));
        for cwd in [&repo, &nested, &runtime] {
            handle_with_input("UserPromptSubmit", "{}", cwd, Some("pm-a"));
            assert!(
                handle_with_input("PreToolUse", &inspection, cwd, Some("pm-a")).is_some(),
                "PM cwd {}",
                cwd.display()
            );
            handle_with_input(
                "PostToolUse",
                &payload("git fetch origin", &nested, json!({"exit_code":0})),
                cwd,
                Some("pm-a"),
            );
            assert!(handle_with_input("PreToolUse", &inspection, cwd, Some("pm-a")).is_none());
        }
    }

    #[test]
    fn nested_fetch_respects_the_nearest_repository_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = dir.path().join("repo");
        repository(&repo);
        register_pm(&repo, "pm-a");
        let nested = repo.join("src");
        std::fs::create_dir(&nested).unwrap();
        let foreign = repo.join("foreign");
        repository(&foreign);
        let malformed = repo.join("malformed");
        std::fs::create_dir(&malformed).unwrap();
        std::fs::write(malformed.join(".git"), "invalid gitdir").unwrap();
        let layout = dir.path().join("layout");
        std::fs::create_dir(&layout).unwrap();
        assert!(gwt_core::process::hidden_command("git")
            .args(["init", "--bare", "--quiet"])
            .arg(layout.join("sibling.git"))
            .status()
            .unwrap()
            .success());
        let plain = layout.join("plain");
        std::fs::create_dir(&plain).unwrap();
        let inspection = payload("git status", &repo, json!(null));
        for cwd in [foreign, malformed, repo.join("missing"), plain] {
            handle_with_input("UserPromptSubmit", "{}", &repo, Some("pm-a"));
            handle_with_input(
                "PostToolUse",
                &payload("git fetch origin", &cwd, json!({"exit_code":0})),
                &repo,
                Some("pm-a"),
            );
            assert!(
                handle_with_input("PreToolUse", &inspection, &repo, Some("pm-a")).is_some(),
                "false fetch receipt for {}",
                cwd.display()
            );
        }
        handle_with_input(
            "PostToolUse",
            &payload(
                &format!("git -C '{}' fetch origin", nested.display()),
                &repo,
                json!({"exit_code":0}),
            ),
            &repo,
            Some("pm-a"),
        );
        assert!(handle_with_input("PreToolUse", &inspection, &repo, Some("pm-a")).is_none());
    }

    #[test]
    fn runtime_mapping_does_not_cross_an_inner_git_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(dir.path().join("home"));
        let repo = gwt_core::paths::gwt_projects_dir().join("pm-store/pm/worktree");
        repository(&repo);
        let runtime = crate::pm_registry::ensure_pm_runtime_dir_for_pm_worktree(&repo).unwrap();
        let foreign = runtime.join("foreign");
        repository(&foreign);
        assert_eq!(
            repository_root_for_cwd(&foreign),
            Some(dunce::canonicalize(&foreign).unwrap())
        );
    }
}
