//! Git repository discovery and inspection

use std::{
    fs,
    path::{Path, PathBuf},
};

use gwt_core::{config::BareProjectConfig, GwtError, Result};

/// The type of repository detected at a given path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoType {
    /// A normal (non-bare) git repository.
    ///
    /// `needs_migration` indicates that gwt would prefer to migrate this
    /// layout to the Nested Bare+Worktree convention before opening
    /// (SPEC-1934 US-6, FR-019). Currently every Normal layout is flagged.
    Normal {
        path: PathBuf,
        needs_migration: bool,
    },
    /// A bare git repository with an optional develop worktree path.
    Bare { develop_worktree: Option<PathBuf> },
    /// Not inside any git repository.
    NonRepo,
}

/// Detect the repository type at the given path.
///
/// Checks the path itself and walks parent directories to find a git repo.
/// Distinguishes between normal repos, bare repos, and non-repo directories.
pub fn detect_repo_type(path: &Path) -> RepoType {
    // Check if path itself has `.git/` (Normal repo) or a `.git` marker file
    // (linked worktree).
    let dot_git = path.join(".git");
    if dot_git.is_dir() {
        return RepoType::Normal {
            path: path.to_path_buf(),
            needs_migration: true,
        };
    }
    if dot_git.is_file() {
        return RepoType::Bare {
            develop_worktree: Some(path.to_path_buf()),
        };
    }
    // Check if path itself is a bare repo (has HEAD + objects + refs)
    if path.join("HEAD").exists() && path.join("objects").exists() && path.join("refs").exists() {
        return RepoType::Bare {
            develop_worktree: None,
        };
    }
    // Check child directories for bare repos, worktrees, or normal repos
    if let Ok(entries) = std::fs::read_dir(path) {
        let mut found_bare = false;
        for entry in entries.flatten() {
            let child = entry.path();
            if !child.is_dir() {
                continue;
            }
            if child.join("HEAD").exists()
                && child.join("objects").exists()
                && child.join("refs").exists()
            {
                found_bare = true;
            }
            if child.join(".git").is_file() {
                found_bare = true;
            }
            if child.join(".git").is_dir() {
                return RepoType::Normal {
                    path: child,
                    needs_migration: true,
                };
            }
        }
        if found_bare {
            // SPEC-1934 2026-05-20 Update FR-050: child scan で bare layout を
            // 検出した場合、`develop` / `main` 等の worktree を auto-select しない。
            // 呼び出し側 (`resolve_project_target`) が workspace home を返し、
            // 実際の worktree 選択は Workspace Overview に委ねる。
            return RepoType::Bare {
                develop_worktree: None,
            };
        }
    }
    // Walk parent directories
    let mut current = path.to_path_buf();
    while let Some(parent) = current.parent() {
        if parent == current {
            break;
        }
        let dot_git = parent.join(".git");
        if dot_git.is_dir() {
            return RepoType::Normal {
                path: parent.to_path_buf(),
                needs_migration: true,
            };
        }
        if dot_git.is_file() {
            // Subdir inside a linked worktree: resolve to the worktree root so
            // a path like `<worktree>/src/foo` opens the worktree, not the
            // workspace home (preserves SC-035 direct-pick semantics).
            return RepoType::Bare {
                develop_worktree: Some(parent.to_path_buf()),
            };
        }
        if parent.join("HEAD").exists()
            && parent.join("objects").exists()
            && parent.join("refs").exists()
        {
            return RepoType::Bare {
                develop_worktree: None,
            };
        }
        current = parent.to_path_buf();
    }
    RepoType::NonRepo
}

/// Derived filesystem target for creating a gwt project from a GitHub repo URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubProjectCloneTarget {
    pub repo_name: String,
    pub workspace_home: PathBuf,
    pub bare_repo_path: PathBuf,
}

/// Outcome of a successful direct Nested Bare+Worktree clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubProjectCloneOutcome {
    pub workspace_home: PathBuf,
    pub bare_repo_path: PathBuf,
}

/// Derive the gwt Workspace Home and nested bare repo path from a repository
/// URL and the user-selected parent directory.
pub fn derive_github_project_clone_target(
    url: &str,
    parent_dir: &Path,
) -> Result<GitHubProjectCloneTarget> {
    let repo_name = repository_name_from_url(url)?;
    let workspace_home = parent_dir.join(&repo_name);
    let bare_repo_path = workspace_home.join(format!("{repo_name}.git"));
    Ok(GitHubProjectCloneTarget {
        repo_name,
        workspace_home,
        bare_repo_path,
    })
}

fn repository_name_from_url(url: &str) -> Result<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(GwtError::Git("repository URL is required".to_string()));
    }

    let without_query = trimmed
        .split(['?', '#'])
        .next()
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    let path_part = if let Some((_prefix, rest)) = without_query.split_once("://") {
        rest.rsplit_once('/')
            .map(|(_parent, name)| name)
            .unwrap_or(rest)
    } else if let Some((_prefix, rest)) = without_query.rsplit_once(':') {
        rest.rsplit_once('/')
            .map(|(_parent, name)| name)
            .unwrap_or(rest)
    } else {
        without_query
            .rsplit_once('/')
            .map(|(_parent, name)| name)
            .unwrap_or(without_query)
    };

    let repo_name = path_part.trim_end_matches(".git").trim();
    let valid = !repo_name.is_empty()
        && repo_name != "."
        && repo_name != ".."
        && repo_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    if !valid {
        return Err(GwtError::Git(format!(
            "invalid repository URL: unable to derive repository name from '{trimmed}'"
        )));
    }
    Ok(repo_name.to_string())
}

/// Clone a GitHub repository directly into gwt's Nested Bare+Worktree layout.
///
/// The resulting directory structure is:
///
/// ```text
/// <parent>/<repo>/
/// ├── <repo>.git/
/// └── .gwt/project.toml
/// ```
pub fn clone_project_as_nested_bare(
    url: &str,
    parent_dir: &Path,
) -> Result<GitHubProjectCloneOutcome> {
    let target = derive_github_project_clone_target(url, parent_dir)?;
    if target.workspace_home.exists() {
        return Err(GwtError::Git(format!(
            "clone target already exists: {}",
            target.workspace_home.display()
        )));
    }
    if !parent_dir.is_dir() {
        return Err(GwtError::Git(format!(
            "clone destination parent is not a directory: {}",
            parent_dir.display()
        )));
    }

    fs::create_dir(&target.workspace_home).map_err(GwtError::Io)?;
    match clone_project_as_nested_bare_inner(url, &target) {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            let cleanup = fs::remove_dir_all(&target.workspace_home);
            if let Err(cleanup_error) = cleanup {
                return Err(GwtError::Git(format!(
                    "{error}; cleanup failed for {}: {cleanup_error}",
                    target.workspace_home.display()
                )));
            }
            Err(error)
        }
    }
}

fn clone_project_as_nested_bare_inner(
    url: &str,
    target: &GitHubProjectCloneTarget,
) -> Result<GitHubProjectCloneOutcome> {
    let bare_path = target.bare_repo_path.to_str().ok_or_else(|| {
        GwtError::Git(format!(
            "invalid bare repository path: {}",
            target.bare_repo_path.display()
        ))
    })?;
    let clone_output =
        gwt_core::process::run_git_logged(&["clone", "--bare", url, bare_path], None)
            .map_err(|error| GwtError::Git(format!("git clone --bare: {error}")))?;
    if !clone_output.status.success() {
        return Err(GwtError::Git(format!(
            "failed to clone repository as bare repo: {}",
            git_stderr(&clone_output)
        )));
    }

    crate::worktree::WorktreeManager::new(&target.bare_repo_path)
        .prepare_start_work_remote_develop()
        .map_err(|error| {
            GwtError::Git(format!(
                "failed to prepare origin/develop for Start Work: {error}"
            ))
        })?;

    install_develop_protection(&target.bare_repo_path)?;

    BareProjectConfig {
        bare_repo_name: format!("{}.git", target.repo_name),
        remote_url: Some(url.to_string()),
        created_at: chrono::Utc::now().to_rfc3339(),
        migrated_from: None,
    }
    .save(&target.workspace_home)?;

    Ok(GitHubProjectCloneOutcome {
        workspace_home: target.workspace_home.clone(),
        bare_repo_path: target.bare_repo_path.clone(),
    })
}

fn git_stderr(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        "git command failed without stderr".to_string()
    } else {
        stderr
    }
}

/// Clone a repository into the target directory using a normal shallow clone.
///
/// Attempts `git clone --depth=1 -b develop <url> <target_dir>` first.
/// Falls back to `git clone --depth=1 <url> <target_dir>` if develop branch
/// does not exist.
pub fn clone_repo(url: &str, target_dir: &Path) -> Result<PathBuf> {
    let target = target_dir
        .to_str()
        .ok_or_else(|| GwtError::Git(format!("Invalid target path: {}", target_dir.display())))?;

    // Try with -b develop first
    let output = gwt_core::process::run_git_logged(
        &["clone", "--depth=1", "-b", "develop", url, target],
        None,
    )
    .map_err(|e| GwtError::Git(format!("git clone: {e}")))?;

    if output.status.success() {
        return Ok(target_dir.to_path_buf());
    }

    // Fallback: clone without -b develop (uses default branch)
    let output = gwt_core::process::run_git_logged(&["clone", "--depth=1", url, target], None)
        .map_err(|e| GwtError::Git(format!("git clone: {e}")))?;

    if output.status.success() {
        Ok(target_dir.to_path_buf())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GwtError::Git(format!("git clone failed: {stderr}")))
    }
}

/// Marker comment for the gwt-managed pre-commit hook section.
///
/// The marker text intentionally retains the legacy wording so existing
/// repositories can rewrite an older "develop + main" block in place.
const GWT_HOOK_START: &str = "# >>> gwt-managed: protect develop and main branches";
const GWT_HOOK_END: &str = "# <<< gwt-managed";

/// The hook script content that blocks direct commits on main.
fn hook_script_block() -> String {
    format!(
        r#"{GWT_HOOK_START}
branch=$(git symbolic-ref --short HEAD 2>/dev/null)
if [ "$branch" = "main" ]; then
  echo "ERROR: Direct commits to $branch are blocked by gwt."
  echo "Create a feature branch: git checkout -b feature/<name>"
  exit 1
fi
{GWT_HOOK_END}"#
    )
}

fn upsert_managed_hook_block(existing: &str) -> String {
    if let Some(start) = existing.find(GWT_HOOK_START) {
        if let Some(end_rel) = existing[start..].find(GWT_HOOK_END) {
            let end = start + end_rel + GWT_HOOK_END.len();
            let mut rewritten = String::with_capacity(existing.len() + hook_script_block().len());
            rewritten.push_str(existing[..start].trim_end());
            if !rewritten.is_empty() {
                rewritten.push('\n');
            }
            rewritten.push_str(&hook_script_block());
            let suffix = existing[end..].trim_start_matches('\n');
            if !suffix.is_empty() {
                rewritten.push('\n');
                rewritten.push_str(suffix);
            }
            if !rewritten.ends_with('\n') {
                rewritten.push('\n');
            }
            return rewritten;
        }
    }

    if existing.is_empty() {
        format!("#!/bin/sh\n{}\n", hook_script_block())
    } else {
        format!("{}\n{}\n", existing.trim_end(), hook_script_block())
    }
}

/// Install a pre-commit hook that blocks direct commits on main.
///
/// Accepts both Normal Git layouts (`<repo>/.git/hooks/...`) and Bare layouts
/// (`<repo>.git/hooks/...` produced by Nested Bare+Worktree migration).
///
/// If a pre-commit hook already exists, the gwt block is appended (preserving
/// existing content). If the gwt block is already present, it is rewritten in
/// place so legacy develop protection is removed.
pub fn install_develop_protection(repo_path: &Path) -> Result<()> {
    let hooks_dir = if repo_path.join(".git").is_dir() {
        repo_path.join(".git").join("hooks")
    } else {
        // Bare layout: hooks live directly under the repo path.
        repo_path.join("hooks")
    };
    fs::create_dir_all(&hooks_dir).map_err(GwtError::Io)?;

    let hook_path = hooks_dir.join("pre-commit");
    let existing = if hook_path.exists() {
        fs::read_to_string(&hook_path).map_err(GwtError::Io)?
    } else {
        String::new()
    };

    let new_content = upsert_managed_hook_block(&existing);

    fs::write(&hook_path, new_content).map_err(GwtError::Io)?;

    // Set executable permission (unix only; no-op on Windows)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&hook_path, perms).map_err(GwtError::Io)?;
    }

    Ok(())
}

/// Initialize workspace scaffolding after a successful clone.
///
/// Creates `specs/`, `~/.gwt/config.toml`, and shared project-index runtime
/// assets if they do not already exist.
pub fn initialize_workspace(repo_path: &Path) -> Result<()> {
    initialize_workspace_with(repo_path, &gwt_core::paths::gwt_home(), |_| {
        gwt_core::runtime::ensure_project_index_runtime().map(|_| ())
    })
}

fn initialize_workspace_with<F>(repo_path: &Path, gwt_home: &Path, ensure_runtime: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    // Create specs/ directory (create_dir_all is idempotent)
    fs::create_dir_all(repo_path.join("specs")).map_err(GwtError::Io)?;

    fs::create_dir_all(gwt_home).map_err(GwtError::Io)?;
    let config_path = gwt_home.join("config.toml");
    if !config_path.exists() {
        let default_config = "[general]\n# gwt default configuration\n";
        fs::write(&config_path, default_config).map_err(GwtError::Io)?;
    }

    ensure_runtime(gwt_home)?;

    Ok(())
}

/// A thin wrapper around a Git repository path for discovery and inspection.
pub struct Repository {
    path: PathBuf,
}

impl Repository {
    /// Open a repository at the given path.
    ///
    /// The path must be a valid Git repository (contains `.git` directory
    /// or is a bare repository).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let git_dir = path.join(".git");
        let is_bare = path.join("HEAD").exists() && path.join("refs").exists();

        if !git_dir.exists() && !is_bare {
            return Err(GwtError::Git(format!(
                "Not a git repository: {}",
                path.display()
            )));
        }

        Ok(Self { path })
    }

    /// Discover a repository by walking up from the given path.
    pub fn discover(start: impl AsRef<Path>) -> Result<Self> {
        let start = start.as_ref();
        let mut current = start.to_path_buf();

        loop {
            if current.join(".git").exists() {
                return Ok(Self { path: current });
            }
            // Check bare repository
            if current.join("HEAD").exists() && current.join("refs").exists() {
                return Ok(Self { path: current });
            }
            if !current.pop() {
                break;
            }
        }

        Err(GwtError::Git(format!(
            "Not a git repository (or any parent): {}",
            start.display()
        )))
    }

    /// Return the repository root path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the current branch name (HEAD symbolic ref).
    ///
    /// Returns `None` for detached HEAD.
    pub fn current_branch(&self) -> Result<Option<String>> {
        let output = gwt_core::process::run_git_logged(
            &["symbolic-ref", "--short", "HEAD"],
            Some(&self.path),
        )
        .map_err(|e| GwtError::Git(format!("symbolic-ref: {e}")))?;

        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(Some(name))
        } else {
            // Detached HEAD
            Ok(None)
        }
    }

    /// List local and remote branch names.
    pub fn branches(&self) -> Result<Vec<String>> {
        let output = gwt_core::process::run_git_logged(
            &["branch", "-a", "--format=%(refname:short)"],
            Some(&self.path),
        )
        .map_err(|e| GwtError::Git(format!("branch: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(GwtError::Git(format!("branch: {stderr}")));
        }

        let branches = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        Ok(branches)
    }

    /// Check if this repository is bare.
    pub fn is_bare(&self) -> bool {
        let output = gwt_core::process::run_git_logged(
            &["rev-parse", "--is-bare-repository"],
            Some(&self.path),
        );

        match output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim() == "true",
            _ => false,
        }
    }

    /// Check if the current directory is inside a worktree.
    pub fn is_worktree(&self) -> bool {
        let output = gwt_core::process::run_git_logged(
            &["rev-parse", "--is-inside-work-tree"],
            Some(&self.path),
        );

        match output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim() == "true",
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- detect_repo_type tests ----

    #[test]
    fn detect_repo_type_returns_nonrepo_for_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(detect_repo_type(tmp.path()), RepoType::NonRepo);
    }

    #[test]
    fn detect_repo_type_returns_normal_for_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(matches!(
            detect_repo_type(tmp.path()),
            RepoType::Normal { .. }
        ));
    }

    #[test]
    fn detect_repo_type_treats_git_file_marker_as_worktree_not_normal() {
        let tmp = tempfile::tempdir().unwrap();
        let bare_dir = tmp.path().join("repo.git");
        let worktree = tmp.path().join("feature");
        std::fs::create_dir_all(bare_dir.join("worktrees").join("feature")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            "gitdir: ../repo.git/worktrees/feature\n",
        )
        .unwrap();

        assert_eq!(
            detect_repo_type(&worktree),
            RepoType::Bare {
                develop_worktree: Some(worktree)
            },
            "linked worktree markers are already gwt-compatible worktrees and must not request Normal migration"
        );
    }

    #[test]
    fn detect_repo_type_returns_bare_for_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", "--bare", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(matches!(
            detect_repo_type(tmp.path()),
            RepoType::Bare { .. }
        ));
    }

    #[test]
    fn detect_repo_type_walks_parents_to_find_normal() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();
        let subdir = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&subdir).unwrap();
        assert!(matches!(detect_repo_type(&subdir), RepoType::Normal { .. }));
    }

    #[test]
    fn detect_repo_type_walks_parents_to_find_worktree_marker_without_migration() {
        let tmp = tempfile::tempdir().unwrap();
        let bare_dir = tmp.path().join("repo.git");
        let worktree = tmp.path().join("feature");
        let nested = worktree.join("src").join("bin");
        std::fs::create_dir_all(bare_dir.join("worktrees").join("feature")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            worktree.join(".git"),
            "gitdir: ../repo.git/worktrees/feature\n",
        )
        .unwrap();

        assert_eq!(
            detect_repo_type(&nested),
            RepoType::Bare {
                develop_worktree: Some(worktree)
            },
            "opening a subdirectory inside a linked worktree must resolve the worktree without migration"
        );
    }

    #[test]
    fn detect_repo_type_finds_bare_in_child_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bare_dir = tmp.path().join("repo.git");
        gwt_core::process::hidden_command("git")
            .args(["init", "--bare", bare_dir.to_str().unwrap()])
            .output()
            .unwrap();
        // Parent directory (tmp) should detect Bare via child scan
        assert!(matches!(
            detect_repo_type(tmp.path()),
            RepoType::Bare { .. }
        ));
    }

    #[test]
    fn detect_repo_type_returns_bare_with_none_for_child_scan() {
        // SPEC-1934 2026-05-20 Update (FR-050): child scan で bare layout を
        // 検出した場合、`develop` / `main` / 任意の worktree を auto-select する
        // ロジックは撤回される。`Bare { develop_worktree: None }` を返し、
        // routing 先は呼び出し側 (`resolve_project_target`) に委ねる。
        let tmp = tempfile::tempdir().unwrap();
        let bare_dir = tmp.path().join("repo.git");
        let main_worktree = tmp.path().join("main");
        let develop_worktree = tmp.path().join("develop");
        gwt_core::process::hidden_command("git")
            .args(["init", "--bare", bare_dir.to_str().unwrap()])
            .output()
            .unwrap();
        for wt in [&main_worktree, &develop_worktree] {
            std::fs::create_dir_all(wt).unwrap();
            std::fs::write(
                wt.join(".git"),
                format!(
                    "gitdir: ../repo.git/worktrees/{}\n",
                    wt.file_name().unwrap().to_string_lossy()
                ),
            )
            .unwrap();
        }

        assert_eq!(
            detect_repo_type(tmp.path()),
            RepoType::Bare {
                develop_worktree: None
            },
            "child scan must not auto-select develop or main worktrees (SPEC-1934 FR-050)"
        );
    }

    #[test]
    fn detect_repo_type_finds_normal_in_child_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_dir = tmp.path().join("my-project");
        std::fs::create_dir_all(&repo_dir).unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", repo_dir.to_str().unwrap()])
            .output()
            .unwrap();
        // Parent directory should detect Normal via child scan
        assert!(matches!(
            detect_repo_type(tmp.path()),
            RepoType::Normal { .. }
        ));
    }

    // ---- clone_repo tests ----

    #[test]
    fn clone_repo_fails_with_invalid_url() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("clone-target");
        let result = clone_repo("https://invalid.example.com/no-such-repo.git", &target);
        assert!(result.is_err());
    }

    #[test]
    fn github_project_clone_target_derives_workspace_from_https_url() {
        let parent = Path::new("/tmp/projects");

        let target =
            derive_github_project_clone_target("https://github.com/akiojin/gwt.git", parent)
                .expect("derive target from https url");

        assert_eq!(target.repo_name, "gwt");
        assert_eq!(target.workspace_home, parent.join("gwt"));
        assert_eq!(target.bare_repo_path, parent.join("gwt").join("gwt.git"));
    }

    #[test]
    fn github_project_clone_target_derives_workspace_from_ssh_url() {
        let parent = Path::new("/tmp/projects");

        let target = derive_github_project_clone_target("git@github.com:akiojin/gwt.git", parent)
            .expect("derive target from ssh url");

        assert_eq!(target.repo_name, "gwt");
        assert_eq!(target.workspace_home, parent.join("gwt"));
        assert_eq!(target.bare_repo_path, parent.join("gwt").join("gwt.git"));
    }

    // ---- install_develop_protection tests ----

    #[test]
    fn install_develop_protection_creates_hook() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        install_develop_protection(tmp.path()).unwrap();

        let hook_path = tmp.path().join(".git/hooks/pre-commit");
        assert!(hook_path.exists());
        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("gwt-managed"));
        assert!(content.contains("main"));
        assert!(content.starts_with("#!/bin/sh"));
        assert!(!content.contains("\"$branch\" = \"develop\""));

        // Check executable permission (unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&hook_path).unwrap().permissions();
            assert!(perms.mode() & 0o111 != 0);
        }
    }

    #[test]
    fn install_develop_protection_preserves_existing_hook() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let hook_path = tmp.path().join(".git/hooks/pre-commit");
        let existing = "#!/bin/sh\necho 'existing hook'\n";
        std::fs::write(&hook_path, existing).unwrap();

        install_develop_protection(tmp.path()).unwrap();

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("existing hook"));
        assert!(content.contains("gwt-managed"));
    }

    #[test]
    fn install_develop_protection_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        install_develop_protection(tmp.path()).unwrap();
        let first = std::fs::read_to_string(tmp.path().join(".git/hooks/pre-commit")).unwrap();

        install_develop_protection(tmp.path()).unwrap();
        let second = std::fs::read_to_string(tmp.path().join(".git/hooks/pre-commit")).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn install_develop_protection_blocks_main_but_not_develop() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        install_develop_protection(tmp.path()).unwrap();

        let content = std::fs::read_to_string(tmp.path().join(".git/hooks/pre-commit")).unwrap();
        assert!(content.contains("\"$branch\" = \"main\""));
        assert!(
            !content.contains("\"$branch\" = \"develop\""),
            "develop should no longer be protected by the managed pre-commit hook"
        );
    }

    #[test]
    fn install_develop_protection_rewrites_legacy_managed_block() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let hook_path = tmp.path().join(".git/hooks/pre-commit");
        std::fs::write(
            &hook_path,
            format!(
                "#!/bin/sh\n{GWT_HOOK_START}\nbranch=$(git symbolic-ref --short HEAD 2>/dev/null)\nif [ \"$branch\" = \"develop\" ] || [ \"$branch\" = \"main\" ]; then\n  exit 1\nfi\n{GWT_HOOK_END}\n"
            ),
        )
        .unwrap();

        install_develop_protection(tmp.path()).unwrap();

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("\"$branch\" = \"main\""));
        assert!(!content.contains("\"$branch\" = \"develop\""));
    }

    // ---- initialize_workspace tests ----

    #[test]
    fn initialize_workspace_creates_specs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let gwt_home = tmp.path().join(".gwt-home");
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        initialize_workspace_with(tmp.path(), &gwt_home, |_| Ok(())).unwrap();
        assert!(tmp.path().join("specs").exists());
    }

    #[test]
    fn initialize_workspace_creates_config_in_supplied_gwt_home() {
        let tmp = tempfile::tempdir().unwrap();
        let gwt_home = tmp.path().join(".gwt-home");
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        initialize_workspace_with(tmp.path(), &gwt_home, |_| Ok(())).unwrap();

        let config = gwt_home.join("config.toml");
        assert!(config.exists());
        assert!(std::fs::read_to_string(config)
            .unwrap()
            .contains("[general]"));
    }

    #[test]
    fn initialize_workspace_invokes_project_index_runtime_bootstrap() {
        let tmp = tempfile::tempdir().unwrap();
        let gwt_home = tmp.path().join(".gwt-home");
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let called_clone = called.clone();
        let expected_home = gwt_home.clone();
        initialize_workspace_with(tmp.path(), &gwt_home, move |arg_home| {
            called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(arg_home, expected_home.as_path());
            Ok(())
        })
        .unwrap();

        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }

    // ---- Repository tests ----

    #[test]
    fn open_non_git_dir_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let result = Repository::open(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn open_valid_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let repo = Repository::open(tmp.path()).unwrap();
        assert_eq!(repo.path(), tmp.path());
    }

    #[test]
    fn discover_walks_up_to_repo() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let subdir = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&subdir).unwrap();

        let repo = Repository::discover(&subdir).unwrap();
        assert_eq!(repo.path(), tmp.path());
    }

    #[test]
    fn discover_fails_for_non_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let result = Repository::discover(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn current_branch_returns_name() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        gwt_core::process::hidden_command("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        // Create an initial commit so HEAD exists
        gwt_core::process::hidden_command("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(path)
            .output()
            .unwrap();

        let repo = Repository::open(path).unwrap();
        let branch = repo.current_branch().unwrap();
        assert!(branch.is_some());
    }

    #[test]
    fn branches_lists_at_least_one() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        gwt_core::process::hidden_command("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        gwt_core::process::hidden_command("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(path)
            .output()
            .unwrap();

        let repo = Repository::open(path).unwrap();
        let branches = repo.branches().unwrap();
        assert!(!branches.is_empty());
    }

    #[test]
    fn is_bare_false_for_normal_repo() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let repo = Repository::open(tmp.path()).unwrap();
        assert!(!repo.is_bare());
    }

    #[test]
    fn is_worktree_true_for_normal_repo() {
        let tmp = tempfile::tempdir().unwrap();
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let repo = Repository::open(tmp.path()).unwrap();
        assert!(repo.is_worktree());
    }
}
