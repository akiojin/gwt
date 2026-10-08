//! Live remote snapshots for PR handoffs (Issue #4979).
use std::{collections::BTreeSet, io, path::Path};

use serde::{Deserialize, Serialize};

pub(crate) const MARKER: &str = "<!-- gwt-pr-head-verification v1\n";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadCheck {
    pub verified_head: String,
    pub remote_head: String,
    pub base_head: String,
    pub head_branch: String,
    pub base_branch: String,
    pub classification: String,
    pub product_commits: Vec<String>,
    pub product_files: Vec<String>,
    #[serde(default)]
    pub record_id: Option<String>,
    #[serde(default)]
    pub diagnostic: Option<String>,
}

impl HeadCheck {
    pub fn allowed(&self) -> bool {
        matches!(
            self.classification.as_str(),
            "matched" | "bookkeeping_or_base_sync"
        )
    }

    pub fn report(&self) -> io::Result<String> {
        serde_json::to_string(self).map_err(io::Error::other)
    }

    pub fn body_note(&self) -> io::Result<String> {
        Ok(format!("\n\n{MARKER}{}\n-->\n", self.report()?))
    }

    pub fn refusal(&self) -> String {
        format!(
            "Ready PR refused: remote HEAD {} compared with verified HEAD {} ({}). Product commits: [{}]; product files: [{}]. {} Only .gwt/ bookkeeping and target-base synchronization without additional product changes are exempt. Fetch the named head branch {}, fast-forward local with `git merge --ff-only {}`, then register the affected verification matrix with `verify.plan` and execute it with `verify.run` before retrying pr.create. A divergent branch must be reconciled before fast-forwarding.",
            self.remote_head, self.verified_head, self.classification,
            self.product_commits.join(", "), self.product_files.join(", "),
            self.diagnostic.as_deref().unwrap_or(""),
            self.head_branch, self.remote_head,
        )
    }
}

pub(crate) fn from_body(body: &str) -> Option<HeadCheck> {
    let (_, rest) = body.rsplit_once(MARKER)?;
    let (json, _) = rest.split_once("\n-->")?;
    serde_json::from_str(json).ok()
}

/// Diagnose a canonical run against the live remote independently of Ready
/// admission. Base-sync classification never refreshes verification evidence.
pub(super) fn read<E: crate::cli::CliEnv>(
    env: &mut E,
    base: &str,
    head: Option<&str>,
    out: &mut String,
) -> Result<i32, gwt_github::SpecOpsError> {
    use crate::cli::verification_record as verification;

    let repo = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
    let record = match verification::load(&repo) {
        Ok(Some(record)) => record,
        Ok(None) => {
            return unprovable(
                out,
                "missing",
                "No canonical verification record exists in this worktree.",
                None,
            )
        }
        Err(error) => return unprovable(out, "unreadable", &error.to_string(), None),
    };
    if record.content_hash.is_empty() || !verification::integrity_ok(&record) {
        return unprovable(
            out,
            "integrity_failed",
            "The canonical run has no valid integrity hash.",
            Some(&record),
        );
    }
    if record.lifecycle.is_some() {
        return unprovable(
            out,
            "incomplete",
            "The canonical verification run is unfinished.",
            Some(&record),
        );
    }
    if !record.all_passed
        || record
            .commands
            .iter()
            .any(|command| command.exit_code != 0 || command.terminated_by_signal.is_some())
    {
        return unprovable(
            out,
            "failed",
            "The canonical verification run has non-passing commands.",
            Some(&record),
        );
    }
    let Some(verified) = record
        .verified_head
        .as_deref()
        .filter(|head| !head.trim().is_empty())
    else {
        return unprovable(
            out,
            "missing_verified_head",
            "The canonical record has no verified HEAD provenance.",
            Some(&record),
        );
    };
    let plan = match verification::load_plan(&repo) {
        Ok(plan) => plan,
        Err(error) => return unprovable(out, "unreadable", &error.to_string(), Some(&record)),
    };
    let evidence = verification::evaluate_evidence_snapshot(
        &repo,
        &record.session_id,
        record.owner_number,
        plan.as_ref(),
        &record,
    );
    if matches!(
        evidence,
        verification::EvidenceStatus::Tampered | verification::EvidenceStatus::Unreadable
    ) {
        return unprovable(out, "integrity_failed", &evidence.describe(), Some(&record));
    }
    let local_head = match git_text(&repo, &["rev-parse", "HEAD"]) {
        Ok(head) => head,
        Err(error) => return unprovable(out, "passed", &error.to_string(), Some(&record)),
    };
    let mut check = match env.inspect_pr_head(base, head, verified) {
        Ok(Some(check)) => check,
        Ok(None) => {
            return unprovable(
                out,
                "passed",
                "The remote head comparison is unavailable.",
                Some(&record),
            )
        }
        Err(error) => return unprovable(out, "passed", &error.to_string(), Some(&record)),
    };
    check.record_id = Some(record.record_id.clone());
    let mut report = serde_json::to_value(check)
        .map_err(|error| gwt_github::client::ApiError::Unexpected(error.to_string()))?;
    report["schema_version"] = serde_json::json!(1);
    report["diagnostic_only"] = serde_json::json!(true);
    report["canonical_record_state"] = serde_json::json!("passed");
    report["compared_head"] = serde_json::json!("remote_head");
    report["local_head"] = serde_json::json!(local_head);
    report["local_verification_fresh"] = serde_json::json!(matches!(
        evidence,
        verification::EvidenceStatus::Fresh | verification::EvidenceStatus::FreshWithQuarantine
    ));
    report["local_verification_reason"] = serde_json::json!(evidence.describe());
    out.push_str(&report.to_string());
    out.push('\n');
    Ok(0)
}

fn unprovable(
    out: &mut String,
    state: &str,
    diagnostic: &str,
    record: Option<&crate::cli::verification_record::VerificationRunRecord>,
) -> Result<i32, gwt_github::SpecOpsError> {
    out.push_str(
        &serde_json::json!({
            "schema_version": 1,
            "diagnostic_only": true,
            "canonical_record_state": state,
            "record_id": record.map(|record| &record.record_id),
            "verified_head": record.and_then(|record| record.verified_head.as_deref()),
            "classification": "unprovable",
            "local_verification_fresh": false,
            "diagnostic": diagnostic,
        })
        .to_string(),
    );
    out.push('\n');
    Ok(2)
}

fn git(repo: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let output = gwt_core::process::hidden_command("git")
        .args(args)
        .current_dir(repo)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "PR head comparison failed (git {}): {}. Restore access to the exact remote branch and verification history before retrying.",
            args.first().unwrap_or(&""), String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn git_text(repo: &Path, args: &[&str]) -> io::Result<String> {
    String::from_utf8(git(repo, args)?)
        .map(|text| text.trim().to_string())
        .map_err(io::Error::other)
}

fn live_head(repo: &Path, remote: &str, branch: &str) -> io::Result<String> {
    git(repo, &["check-ref-format", "--branch", branch])?;
    let reference = format!("refs/heads/{branch}");
    let refs = git_text(repo, &["ls-remote", "--heads", remote, &reference])?;
    refs.lines().find_map(|line| {
        let (sha, name) = line.split_once('\t')?;
        (name == reference && matches!(sha.len(), 40 | 64)
            && sha.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| sha.to_string())
    }).ok_or_else(|| io::Error::other(format!("PR head comparison: remote branch {reference} is unavailable; push the verified branch before retrying.")))
}

fn product_paths(repo: &Path, before: &str, after: &str) -> io::Result<Vec<String>> {
    Ok(git(
        repo,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            before,
            after,
            "--",
            ".",
            ":(exclude).gwt",
        ],
    )?
    .split(|byte| *byte == 0)
    .filter(|path| !path.is_empty())
    .map(|path| String::from_utf8_lossy(path).into_owned())
    .collect())
}

pub(crate) fn compare(
    repo: &Path,
    remote: &str,
    head: &str,
    base: &str,
    verified: &str,
) -> io::Result<HeadCheck> {
    compare_remotes(repo, remote, remote, head, base, verified)
}

pub(crate) fn compare_remotes(
    repo: &Path,
    head_remote: &str,
    base_remote: &str,
    head: &str,
    base: &str,
    verified: &str,
) -> io::Result<HeadCheck> {
    // Diff pathspecs, including the bookkeeping exclusion, are root-relative.
    let root = git_text(repo, &["rev-parse", "--show-toplevel"])?;
    let repo = Path::new(&root);
    let remote_head = live_head(repo, head_remote, head)?;
    let base_head = live_head(repo, base_remote, base)?;
    let mut check = HeadCheck {
        verified_head: verified.to_string(),
        remote_head,
        base_head,
        head_branch: head.to_string(),
        base_branch: base.to_string(),
        classification: "matched".to_string(),
        product_commits: Vec::new(),
        product_files: Vec::new(),
        record_id: None,
        diagnostic: None,
    };
    let proof = (|| {
        // Fetch objects only: never overwrite FETCH_HEAD or move local refs.
        git(
            repo,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                head_remote,
                &check.remote_head,
            ],
        )?;
        git(
            repo,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                base_remote,
                &check.base_head,
            ],
        )?;
        git(repo, &["cat-file", "-e", &format!("{verified}^{{commit}}")])?;
        classify(repo, &mut check)?;
        if live_head(repo, head_remote, head)? != check.remote_head
            || live_head(repo, base_remote, base)? != check.base_head
        {
            return Err(io::Error::other(
                "Remote moved during comparison; retry against its current snapshot.",
            ));
        }
        Ok::<_, io::Error>(())
    })();
    if let Err(error) = proof {
        check.classification = "unprovable".to_string();
        check.diagnostic = Some(error.to_string());
    }
    Ok(check)
}

fn projected_tree(repo: &Path, left: &str, right: &str) -> io::Result<(String, Vec<String>)> {
    let projection = gwt_core::process::hidden_command("git")
        .args([
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            "-z",
            left,
            right,
        ])
        .current_dir(repo)
        .output()?;
    let fields: Vec<_> = projection.stdout.split(|byte| *byte == 0).collect();
    let tree = fields
        .first()
        .and_then(|tree| std::str::from_utf8(tree).ok())
        .filter(|tree| {
            matches!(tree.len(), 40 | 64) && tree.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .ok_or_else(|| io::Error::other("Cannot read the projected base synchronization tree."))?;
    match projection.status.code() {
        Some(0) => Ok((tree.to_string(), Vec::new())),
        Some(1) => {
            let conflicts: Vec<_> = fields.iter().skip(1).take_while(|path| !path.is_empty()).collect();
            if conflicts.is_empty() { return Err(io::Error::other("Merge projection reported conflicts without paths.")); }
            Ok((tree.to_string(), conflicts.into_iter()
                .filter(|path| **path != b".gwt" && !path.starts_with(b".gwt/"))
                .map(|path| String::from_utf8_lossy(path).into_owned()).collect()))
        }
        _ => Err(io::Error::other("Cannot project base synchronization; synchronize locally and rerun verify.plan / verify.run.")),
    }
}

fn classify(repo: &Path, check: &mut HeadCheck) -> io::Result<()> {
    let verified = check.verified_head.clone();
    if check.remote_head != verified {
        let ancestry = gwt_core::process::hidden_command("git")
            .args(["merge-base", "--is-ancestor", &verified, &check.remote_head])
            .current_dir(repo)
            .status()?;
        if !ancestry.success() {
            check.classification = "divergent_or_behind".to_string();
            check.product_files = product_paths(repo, &verified, &check.remote_head)?;
            return Ok(());
        }
        let mut files = BTreeSet::new();
        // Exclude actual target-base commits, not just the merge commit itself.
        // Inspect every commit, including merge resolutions, so a later
        // revert cannot hide unverified product history.
        let commits = git_text(
            repo,
            &[
                "rev-list",
                "--reverse",
                &check.remote_head,
                &format!("^{verified}"),
                &format!("^{}", check.base_head),
            ],
        )?;
        for commit in commits.lines() {
            let parents = git_text(repo, &["rev-list", "--parents", "-n", "1", commit])?;
            let parents: Vec<_> = parents.split_whitespace().collect();
            let paths = match parents.as_slice() {
                [_, parent] => product_paths(repo, parent, commit)?,
                [_, first, second] => {
                    let (tree, conflicts) = projected_tree(repo, first, second)?;
                    if conflicts.is_empty() { product_paths(repo, &tree, commit)? } else { conflicts }
                }
                _ => return Err(io::Error::other("Cannot prove a multi-parent merge contains only bookkeeping/base synchronization.")),
            };
            if !paths.is_empty() {
                check.product_commits.push(commit.to_string());
                files.extend(paths);
                check.product_files = files.iter().cloned().collect();
            }
        }
        if !check.product_files.is_empty() {
            check.classification = "unverified_product".to_string();
            return Ok(());
        }
        let bases = git_text(
            repo,
            &["merge-base", "--all", &check.remote_head, &check.base_head],
        )?;
        if bases.lines().count() != 1 {
            return Err(io::Error::other("PR head comparison cannot prove a unique target-base synchronization; reconcile the histories and rerun verify.plan / verify.run."));
        }
        let (tree, conflicts) = projected_tree(repo, &verified, &bases)?;
        if !conflicts.is_empty() {
            check.product_files = conflicts;
            return Err(io::Error::other("Product conflicts in projected base synchronization; resolve locally and rerun verify.plan / verify.run."));
        }
        let resolution_paths = product_paths(repo, &tree, &check.remote_head)?;
        if !resolution_paths.is_empty() && check.product_commits.is_empty() {
            check.product_commits.push(check.remote_head.clone());
        }
        files.extend(resolution_paths);
        check.product_files = files.into_iter().collect();
        check.classification = if check.product_files.is_empty() {
            "bookkeeping_or_base_sync"
        } else {
            "unverified_product"
        }
        .to_string();
    }
    Ok(())
}

#[cfg(test)]
mod tests;
