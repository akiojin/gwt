//! Git repository discovery and inspection

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use gwt_core::{GwtError, Result};

/// The type of repository detected at a given path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoType {
    /// A normal (non-bare) git repository. Contains the resolved repo root path.
    Normal(PathBuf),
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
    // Check if path itself has .git (normal repo or worktree)
    if path.join(".git").exists() {
        return RepoType::Normal(path.to_path_buf());
    }
    // Check if path itself is a bare repo (has HEAD + objects + refs)
    if path.join("HEAD").exists() && path.join("objects").exists() && path.join("refs").exists() {
        let develop = find_develop_worktree(path.parent().unwrap_or(path));
        return RepoType::Bare {
            develop_worktree: develop,
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
                return RepoType::Normal(child);
            }
        }
        if found_bare {
            let develop = find_develop_worktree(path);
            return RepoType::Bare {
                develop_worktree: develop,
            };
        }
    }
    // Walk parent directories
    let mut current = path.to_path_buf();
    while let Some(parent) = current.parent() {
        if parent == current {
            break;
        }
        if parent.join(".git").exists() {
            return RepoType::Normal(parent.to_path_buf());
        }
        if parent.join("HEAD").exists()
            && parent.join("objects").exists()
            && parent.join("refs").exists()
        {
            let develop = find_develop_worktree(parent.parent().unwrap_or(parent));
            return RepoType::Bare {
                develop_worktree: develop,
            };
        }
        current = parent.to_path_buf();
    }
    RepoType::NonRepo
}

/// Find a develop worktree in the given directory.
/// Looks for a child directory named "develop" that has a .git worktree marker.
fn find_develop_worktree(parent: &Path) -> Option<PathBuf> {
    let develop = parent.join("develop");
    if develop.is_dir() && develop.join(".git").exists() {
        return Some(develop);
    }
    None
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
    let output = Command::new("git")
        .args(["clone", "--depth=1", "-b", "develop", url, target])
        .output()
        .map_err(|e| GwtError::Git(format!("git clone: {e}")))?;

    if output.status.success() {
        return Ok(target_dir.to_path_buf());
    }

    // Fallback: clone without -b develop (uses default branch)
    let output = Command::new("git")
        .args(["clone", "--depth=1", url, target])
        .output()
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
/// If a pre-commit hook already exists, the gwt block is appended (preserving
/// existing content). If the gwt block is already present, it is rewritten in
/// place so legacy develop protection is removed.
pub fn install_develop_protection(repo_path: &Path) -> Result<()> {
    let hooks_dir = repo_path.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).map_err(GwtError::Io)?;

    let hook_path = hooks_dir.join("pre-commit");
    let existing = if hook_path.exists() {
        fs::read_to_string(&hook_path).map_err(GwtError::Io)?
    } else {
        String::new()
    };

    let new_content = upsert_managed_hook_block(&existing);

    fs::write(&hook_path, new_content).map_err(GwtError::Io)?;

    // Set executable permission
    let perms = fs::Permissions::from_mode(0o755);
    fs::set_permissions(&hook_path, perms).map_err(GwtError::Io)?;

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
        let output = std::process::Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(&self.path)
            .output()
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
        let output = std::process::Command::new("git")
            .args(["branch", "-a", "--format=%(refname:short)"])
            .current_dir(&self.path)
            .output()
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
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--is-bare-repository"])
            .current_dir(&self.path)
            .output();

        match output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim() == "true",
            _ => false,
        }
    }

    /// Check if the current directory is inside a worktree.
    pub fn is_worktree(&self) -> bool {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&self.path)
            .output();

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
        Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(matches!(detect_repo_type(tmp.path()), RepoType::Normal(_)));
    }

    #[test]
    fn detect_repo_type_returns_bare_for_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        Command::new("git")
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
        Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();
        let subdir = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&subdir).unwrap();
        assert!(matches!(detect_repo_type(&subdir), RepoType::Normal(_)));
    }

    #[test]
    fn detect_repo_type_finds_bare_in_child_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bare_dir = tmp.path().join("repo.git");
        Command::new("git")
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
    fn detect_repo_type_finds_normal_in_child_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_dir = tmp.path().join("my-project");
        std::fs::create_dir_all(&repo_dir).unwrap();
        Command::new("git")
            .args(["init", repo_dir.to_str().unwrap()])
            .output()
            .unwrap();
        // Parent directory should detect Normal via child scan
        assert!(matches!(detect_repo_type(tmp.path()), RepoType::Normal(_)));
    }

    // ---- clone_repo tests ----

    #[test]
    fn clone_repo_fails_with_invalid_url() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("clone-target");
        let result = clone_repo("https://invalid.example.com/no-such-repo.git", &target);
        assert!(result.is_err());
    }

    // ---- install_develop_protection tests ----

    #[test]
    fn install_develop_protection_creates_hook() {
        let tmp = tempfile::tempdir().unwrap();
        Command::new("git")
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

        // Check executable permission
        let perms = std::fs::metadata(&hook_path).unwrap().permissions();
        assert!(perms.mode() & 0o111 != 0);
    }

    #[test]
    fn install_develop_protection_preserves_existing_hook() {
        let tmp = tempfile::tempdir().unwrap();
        Command::new("git")
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
        Command::new("git")
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
        Command::new("git")
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
        Command::new("git")
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
        Command::new("git")
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
        Command::new("git")
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
        Command::new("git")
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
        std::process::Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let repo = Repository::open(tmp.path()).unwrap();
        assert_eq!(repo.path(), tmp.path());
    }

    #[test]
    fn discover_walks_up_to_repo() {
        let tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
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
        std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        // Create an initial commit so HEAD exists
        std::process::Command::new("git")
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
        std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        std::process::Command::new("git")
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
        std::process::Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let repo = Repository::open(tmp.path()).unwrap();
        assert!(!repo.is_bare());
    }

    #[test]
    fn is_worktree_true_for_normal_repo() {
        let tmp = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let repo = Repository::open(tmp.path()).unwrap();
        assert!(repo.is_worktree());
    }
}
