//! Persistent squash-merge verdicts for [`crate::branch::is_branch_merged_into`]
//! (Issue #6877 AC-3).
//!
//! `git cherry <base> <branch>` computes a patch id for every commit reachable
//! from `base` but not from `branch`. For a work branch that is thousands of
//! commits behind `develop` that is several seconds of CPU, and the GUI used to
//! run it for every local branch on every branch-list load.
//!
//! This cache keys the verdict by the branch tip SHA. While the tip is
//! unchanged, a base that merely advanced is handled incrementally: only the
//! commits the base gained since the last verdict can newly contain the
//! branch's patches, so their patch ids are compared against the branch's
//! still-unmatched patch ids. `git cherry` runs again only when the branch tip
//! itself changed (or the base was rewritten).

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

const CACHE_FILE_NAME: &str = "branch-merge-verdicts.json";
/// Entries kept per project before the least recently computed are evicted.
const MAX_ENTRIES: usize = 8192;
/// Base lineages remembered per branch tip (`origin/develop`, `origin/main`, ...).
const MAX_LINEAGES_PER_BRANCH: usize = 4;
/// A base that advanced by more commits than this is recomputed from scratch;
/// `git cherry` is cheaper than streaming that many patches.
const MAX_INCREMENTAL_BASE_COMMITS: usize = 2000;

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    #[serde(default)]
    version: u32,
    /// Branch tip SHA → verdicts against each known base lineage.
    #[serde(default)]
    entries: BTreeMap<String, Vec<Verdict>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Verdict {
    /// Tip of the base when this verdict was last brought up to date.
    base_sha: String,
    /// Patch ids of the branch commits not yet found in the base. Empty means
    /// the branch is fully merged.
    unmatched_patch_ids: Vec<String>,
    /// Unix seconds of the last computation, for eviction only.
    computed_at: i64,
}

/// Whether `branch_sha` is fully merged into `base_sha`, consulting and
/// updating the persistent verdict cache. `is_ancestor` must already have
/// been ruled out by the caller (that is the cheap, always-fresh path).
pub(crate) fn is_merged_cached(repo_path: &Path, branch_sha: &str, base_sha: &str) -> Result<bool> {
    let _guard = file_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = cache_path(repo_path);
    let mut cache = load(&path);
    let verdicts = cache.entries.entry(branch_sha.to_string()).or_default();

    if let Some(verdict) = verdicts.iter().find(|verdict| verdict.base_sha == base_sha) {
        return Ok(verdict.unmatched_patch_ids.is_empty());
    }

    let advanced = verdicts
        .iter()
        .position(|verdict| is_ancestor(repo_path, &verdict.base_sha, base_sha).unwrap_or(false));
    let refreshed = match advanced {
        Some(index) => {
            let previous = verdicts.remove(index);
            match advance_verdict(repo_path, previous, base_sha)? {
                Some(verdict) => verdict,
                None => compute_verdict(repo_path, branch_sha, base_sha)?,
            }
        }
        None => compute_verdict(repo_path, branch_sha, base_sha)?,
    };
    let merged = refreshed.unmatched_patch_ids.is_empty();
    verdicts.insert(0, refreshed);
    verdicts.truncate(MAX_LINEAGES_PER_BRANCH);
    evict(&mut cache);
    save(&path, &cache);
    Ok(merged)
}

/// Carry `previous` forward to `base_sha`: only the commits the base gained can
/// newly match the still-unmatched patches. `None` when the gained range is too
/// large to stream and a full `git cherry` is the cheaper choice.
fn advance_verdict(repo_path: &Path, previous: Verdict, base_sha: &str) -> Result<Option<Verdict>> {
    let gained = gained_patch_ids(repo_path, &previous.base_sha, base_sha)?;
    let Some(gained) = gained else {
        return Ok(None);
    };
    let unmatched_patch_ids = previous
        .unmatched_patch_ids
        .into_iter()
        .filter(|patch_id| !gained.contains(patch_id))
        .collect();
    Ok(Some(Verdict {
        base_sha: base_sha.to_string(),
        unmatched_patch_ids,
        computed_at: now_secs(),
    }))
}

/// Patch ids shared by every branch that advances across the same base range.
type PatchIdSet = std::sync::Arc<BTreeSet<String>>;
/// `(old_base, new_base)` → gained patch ids, or `None` when the range was too large.
type GainedMemo = Mutex<HashMap<(String, String), Option<PatchIdSet>>>;

/// Patch ids of the non-merge commits in `new_base ^old_base`, memoised per
/// process because one branch-list load asks the same question for every
/// branch that shares the previous base.
fn gained_patch_ids(
    repo_path: &Path,
    old_base: &str,
    new_base: &str,
) -> Result<Option<PatchIdSet>> {
    static GAINED: OnceLock<GainedMemo> = OnceLock::new();
    let key = (old_base.to_string(), new_base.to_string());
    let memo = GAINED.get_or_init(Default::default);
    if let Some(cached) = memo
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
    {
        return Ok(cached.clone());
    }
    let output = run_git(
        repo_path,
        &["rev-list", "--no-merges", new_base, &format!("^{old_base}")],
    )?;
    let shas: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let gained = if shas.len() > MAX_INCREMENTAL_BASE_COMMITS {
        None
    } else {
        Some(PatchIdSet::new(patch_ids_for_commits(repo_path, &shas)?))
    };
    let mut memo = memo
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if memo.len() >= 64 {
        memo.clear();
    }
    memo.insert(key, gained.clone());
    Ok(gained)
}

/// Full verdict through `git cherry`, keeping the patch ids of the commits it
/// reports as unmatched (`+`) so a later base advance can settle them.
fn compute_verdict(repo_path: &Path, branch_sha: &str, base_sha: &str) -> Result<Verdict> {
    let stdout = run_git(repo_path, &["cherry", base_sha, branch_sha])
        .map_err(|error| GwtError::Git(format!("cherry {base_sha} {branch_sha}: {error}")))?;
    let unmatched = unmatched_cherry_commits(&stdout);
    let unmatched_patch_ids = if unmatched.is_empty() {
        Vec::new()
    } else {
        patch_ids_for_commits(repo_path, &unmatched)?
            .into_iter()
            .collect()
    };
    Ok(Verdict {
        base_sha: base_sha.to_string(),
        unmatched_patch_ids,
        computed_at: now_secs(),
    })
}

/// The commits `git cherry` reports as `+` (no equivalent in the upstream side).
pub(crate) fn unmatched_cherry_commits(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix('+'))
        .map(str::trim)
        .filter(|sha| !sha.is_empty())
        .collect()
}

/// `git patch-id --stable` for each listed commit, two spawns total.
fn patch_ids_for_commits(repo_path: &Path, shas: &[&str]) -> Result<BTreeSet<String>> {
    if shas.is_empty() {
        return Ok(BTreeSet::new());
    }
    let stdin = shas.join("\n") + "\n";
    let patches = run_git_with_stdin(
        repo_path,
        &["diff-tree", "--stdin", "-p", "-r", "--no-color"],
        stdin.as_bytes(),
    )?;
    let ids = run_git_with_stdin(repo_path, &["patch-id", "--stable"], patches.as_bytes())?;
    Ok(ids
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect())
}

/// `git merge-base --is-ancestor <ancestor> <descendant>`.
pub(crate) fn is_ancestor(repo_path: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
    let output = gwt_core::process::run_git_logged(
        &["merge-base", "--is-ancestor", ancestor, descendant],
        Some(repo_path),
    )
    .map_err(|error| GwtError::Git(format!("merge-base: {error}")))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(GwtError::Git(format!(
            "merge-base --is-ancestor {ancestor} {descendant}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

/// Resolve `branch` and `base` to commit SHAs in one spawn; `None` when either
/// does not resolve.
pub(crate) fn resolve_commit_pair(
    repo_path: &Path,
    branch: &str,
    base: &str,
) -> Result<Option<(String, String)>> {
    let output = gwt_core::process::run_git_logged(
        // `--verify` accepts a single rev; a failed lookup fails the whole spawn.
        &[
            "rev-parse",
            &format!("{branch}^{{commit}}"),
            &format!("{base}^{{commit}}"),
        ],
        Some(repo_path),
    )
    .map_err(|error| GwtError::Git(format!("rev-parse: {error}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    match (lines.next(), lines.next()) {
        (Some(branch_sha), Some(base_sha)) => {
            Ok(Some((branch_sha.to_string(), base_sha.to_string())))
        }
        _ => Ok(None),
    }
}

fn run_git(repo_path: &Path, args: &[&str]) -> Result<String> {
    let output = gwt_core::process::run_git_logged(args, Some(repo_path))
        .map_err(|error| GwtError::Git(format!("git {}: {error}", args.join(" "))))?;
    if !output.status.success() {
        return Err(GwtError::Git(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_git_with_stdin(repo_path: &Path, args: &[&str], stdin: &[u8]) -> Result<String> {
    let output = gwt_core::process::run_git_logged_with_stdin(args, Some(repo_path), stdin)
        .map_err(|error| GwtError::Git(format!("git {}: {error}", args.join(" "))))?;
    if !output.status.success() {
        return Err(GwtError::Git(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn file_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Default::default)
}

/// `~/.gwt/projects/<hash>/branch-merge-verdicts.json` for the repository.
pub(crate) fn cache_path(repo_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path).join(CACHE_FILE_NAME)
}

fn load(path: &Path) -> CacheFile {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok())
        .filter(|cache| cache.version == 1)
        .unwrap_or(CacheFile {
            version: 1,
            entries: BTreeMap::new(),
        })
}

fn save(path: &Path, cache: &CacheFile) {
    // A lost cache only costs a recomputation; never fail the verdict on it.
    let Ok(bytes) = serde_json::to_vec(cache) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = gwt_core::atomic_file::write_atomic(path, &bytes);
}

fn evict(cache: &mut CacheFile) {
    let total: usize = cache.entries.values().map(Vec::len).sum();
    if total <= MAX_ENTRIES {
        return;
    }
    let mut ages: Vec<i64> = cache
        .entries
        .values()
        .flatten()
        .map(|verdict| verdict.computed_at)
        .collect();
    ages.sort_unstable();
    let cutoff = ages[total - MAX_ENTRIES / 2];
    for verdicts in cache.entries.values_mut() {
        verdicts.retain(|verdict| verdict.computed_at >= cutoff);
    }
    cache.entries.retain(|_, verdicts| !verdicts.is_empty());
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn cached_branch_tips(repo_path: &Path) -> Vec<String> {
    load(&cache_path(repo_path)).entries.into_keys().collect()
}
