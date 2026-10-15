//! Failure-only project Git diagnostics. Repairs invalidate the observation.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

const MAX_METADATA_BYTES: u64 = 65_536;
type Metadata = Vec<(PathBuf, Result<MetadataContents, io::ErrorKind>)>;

#[derive(Debug, PartialEq, Eq)]
enum MetadataContents {
    Directory,
    File(Vec<u8>),
}

struct Failure {
    metadata: Metadata,
    extra_files: Vec<PathBuf>,
    diagnosis: String,
}

fn failures() -> &'static Mutex<BTreeMap<PathBuf, Failure>> {
    static CACHE: OnceLock<Mutex<BTreeMap<PathBuf, Failure>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn read_metadata(path: &Path) -> Result<MetadataContents, io::ErrorKind> {
    let kind = fs::metadata(path).map_err(|error| error.kind())?;
    if kind.is_dir() {
        return Ok(MetadataContents::Directory);
    }
    if !kind.is_file() {
        return Err(io::ErrorKind::InvalidInput);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_METADATA_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| error.kind())?;
    // An oversized/unreadable file cannot support a stable failure fence.
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(io::ErrorKind::InvalidData);
    }
    Ok(MetadataContents::File(bytes))
}

fn control_target(path: &Path, prefix: &str) -> Option<PathBuf> {
    let MetadataContents::File(bytes) = read_metadata(path).ok()? else {
        return None;
    };
    let text = std::str::from_utf8(&bytes).ok()?;
    let raw = text.lines().next()?.trim().strip_prefix(prefix)?.trim();
    if raw.is_empty() {
        return None;
    }
    let target = PathBuf::from(raw);
    Some(if target.is_absolute() {
        target
    } else {
        path.parent()?.join(target)
    })
}

fn metadata(repo: &Path) -> Metadata {
    let mut paths = vec![repo.join(".git")];
    for root in repo.ancestors() {
        let marker = root.join(".git");
        if marker.exists() {
            paths.push(marker.clone());
            let admin = if marker.is_dir() {
                Some(marker)
            } else {
                control_target(&marker, "gitdir:")
            };
            if let Some(admin) = admin {
                paths.extend([
                    admin.join("gitdir"),
                    admin.join("commondir"),
                    admin.join("config.worktree"),
                    admin.join("HEAD"),
                ]);
                let common = control_target(&admin.join("commondir"), "").unwrap_or(admin);
                paths.extend([
                    common.join("config"),
                    common.join("HEAD"),
                    common.join("objects"),
                    common.join("refs"),
                ]);
            }
            break;
        }
    }
    if let Some(common) = gwt_core::repo_hash::repository_common_dir(repo) {
        paths.extend([
            common.join("config"),
            common.join("HEAD"),
            common.join("objects"),
            common.join("refs"),
        ]);
    }
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            let contents = read_metadata(&path);
            (path, contents)
        })
        .collect()
}

fn cache_key(repo: &Path) -> PathBuf {
    fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf())
}

/// No Git process is started until the metadata that failed has changed.
pub fn cached_metadata_failure(repo: &Path) -> Option<String> {
    let mut observed = metadata(repo);
    let mut cache = failures()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let key = cache_key(repo);
    if let Some(failure) = cache.get(&key) {
        observed.extend(
            failure
                .extra_files
                .iter()
                .map(|path| (path.clone(), read_metadata(path))),
        );
        observed.sort_by(|a, b| a.0.cmp(&b.0));
        observed.dedup_by(|a, b| a.0 == b.0);
    }
    if cache
        .get(&key)
        .is_some_and(|failure| failure.metadata != observed)
    {
        cache.remove(&key);
    }
    cache.get(&key).map(|failure| failure.diagnosis.clone())
}

/// Record only deterministic Git configuration/metadata failures. Process
/// deadlines, network errors and origin setup remain ordinary retryable errors.
pub fn record_metadata_failure(repo: &Path, command: &str, stderr: &str) -> Option<String> {
    let lower = stderr.to_ascii_lowercase();
    if ![
        "invalid path",
        "bad config",
        "bad git config",
        "not a git repository",
        "invalid gitfile",
        "unable to read config",
        "cannot chdir",
        "invalid line",
    ]
    .iter()
    .any(|text| lower.contains(text))
    {
        return None;
    }
    let mut observed = metadata(repo);
    // Git identifies malformed includes by their exact source path. Watch
    // that bounded file too, rather than permanently fencing its repair.
    let extra_files: Vec<PathBuf> = stderr
        .rsplit_once(" in file ")
        .and_then(|(_, path)| {
            let path = PathBuf::from(path.trim().trim_matches(['\"', '\'']));
            if path.is_absolute() {
                return Some(path);
            }
            let common = gwt_core::repo_hash::repository_common_dir(repo)?;
            let root = if common.file_name().is_some_and(|name| name == ".git") {
                common.parent()?
            } else {
                &common
            };
            fs::canonicalize(root.join(path)).ok()
        })
        .into_iter()
        .collect();
    observed.extend(
        extra_files
            .iter()
            .map(|path| (path.clone(), read_metadata(path))),
    );
    observed.sort_by(|a, b| a.0.cmp(&b.0));
    observed.dedup_by(|a, b| a.0 == b.0);
    let mut diagnosis = format!("Git needs attention\nCommand: {command}\nGit stderr: {}\nRepair the Git metadata, then retry; validation resumes when the metadata changes.", stderr.trim());
    for (path, bytes) in &observed {
        let Ok(MetadataContents::File(bytes)) = bytes else {
            continue;
        };
        let text = String::from_utf8_lossy(bytes);
        let mut core = false;
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                core = trimmed.eq_ignore_ascii_case("[core]");
            }
            if let Some(offset) = line.find("/mnt/") {
                let raw = &line[offset + 5..];
                if let Some((drive, tail)) = raw.split_once('/') {
                    if drive.len() == 1 && drive.as_bytes()[0].is_ascii_alphabetic() {
                        diagnosis.push_str(&format!(
                            "\n{}:{}: WSL path {}; Windows path candidate {}:/{}",
                            path.display(),
                            index + 1,
                            &line[offset..],
                            drive.to_ascii_uppercase(),
                            tail.trim_matches(['\"', '\''])
                        ));
                    }
                }
            }
            let key = trimmed.split(['=', ' ', '\t']).next().unwrap_or_default();
            if core && (key.eq_ignore_ascii_case("worktree") || key.eq_ignore_ascii_case("bare")) {
                diagnosis.push_str(&format!(
                    "\n{}:{}: shared core.{trimmed}",
                    path.display(),
                    index + 1
                ));
            }
        }
        let common = path.parent().unwrap_or(repo);
        if path.file_name().is_some_and(|name| name == "config")
            && common.join("HEAD").is_file()
            && common.join("objects").is_dir()
            && common.join("refs").is_dir()
            && common.file_name().is_none_or(|name| name != ".git")
        {
            diagnosis.push_str(&format!("\nGit administration directory: {}. Shared core.worktree or core.bare=false can redirect Git. HEAD/objects/refs alone do not prove a bare layout. Only if this is gwt's shared bare repository, confirm the layout in .gwt/project.toml, then repair explicitly:\ngit config --file \"{}\" --unset core.worktree\ngit config --file \"{}\" core.bare true\nA valid --separate-git-dir working checkout uses core.bare=false; leave that layout unchanged. No configuration was changed automatically.", common.display(), path.display(), path.display()));
        }
    }
    // Do not permanently fence a failure whose relevant metadata is unreadable.
    if (lower.contains("bad config")
        && extra_files.is_empty()
        && !observed
            .iter()
            .any(|(path, _)| stderr.contains(&path.to_string_lossy().to_string())))
        || observed
            .iter()
            .any(|(_, bytes)| matches!(bytes, Err(kind) if *kind != io::ErrorKind::NotFound))
    {
        return Some(diagnosis);
    }
    let mut cache = failures()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= 64 {
        cache.pop_first();
    }
    cache.insert(
        cache_key(repo),
        Failure {
            metadata: observed,
            extra_files,
            diagnosis: diagnosis.clone(),
        },
    );
    Some(diagnosis)
}

/// PM launch preflight validates Git's view, including shared bare settings.
pub fn validate_project_git(repo: &Path) -> io::Result<PathBuf> {
    if let Some(diagnosis) = cached_metadata_failure(repo) {
        return Err(io::Error::other(diagnosis));
    }
    let command = "git rev-parse --git-common-dir";
    let result = (|| {
        let root = crate::worktree::main_worktree_root(repo).map_err(|error| error.to_string())?;
        let output =
            gwt_core::process::run_git_logged(&["rev-parse", "--git-common-dir"], Some(&root))
                .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(root)
    })();
    result.map_err(|error| {
        io::Error::other(record_metadata_failure(repo, command, &error).unwrap_or(error))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_checkout_directory_and_head_repairs_invalidate_failures() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path();
        let git = repo.join(".git");
        fs::create_dir_all(git.join("objects")).unwrap();
        fs::create_dir_all(git.join("refs")).unwrap();
        fs::write(git.join("config"), "[core]\n bare = false\n").unwrap();
        assert_eq!(read_metadata(&git), Ok(MetadataContents::Directory));
        record_metadata_failure(
            repo,
            "git rev-parse --git-common-dir",
            "fatal: not a git repository",
        )
        .unwrap();
        assert!(cached_metadata_failure(repo).is_some());
        fs::write(git.join("HEAD"), "ref: refs/heads/develop\n").unwrap();
        assert!(cached_metadata_failure(repo).is_none());
        record_metadata_failure(
            repo,
            "git rev-parse --git-common-dir",
            "fatal: bad config line 1 in file .git/config",
        )
        .unwrap();
        assert!(
            cached_metadata_failure(repo).is_some(),
            "Git reports the target config relative to its cwd"
        );
        fs::write(
            git.join("config"),
            "[core]\n bare = false\n filemode = true\n",
        )
        .unwrap();
        assert!(cached_metadata_failure(repo).is_none());
    }

    #[test]
    fn bare_configuration_failure_is_fenced_until_the_metadata_changes() {
        let temp = tempfile::tempdir().unwrap();
        let bare = temp.path().join("shared.git");
        std::fs::create_dir_all(bare.join("objects")).unwrap();
        std::fs::create_dir_all(bare.join("refs")).unwrap();
        std::fs::write(bare.join("HEAD"), "ref: refs/heads/develop\n").unwrap();
        let config = bare.join("config");
        std::fs::write(
            &config,
            "[core]\n bare = false\n worktree = /mnt/e/gwt/work/issue-5255\n",
        )
        .unwrap();
        let before = std::fs::read(&config).unwrap();
        let diagnosis = record_metadata_failure(
            &bare,
            "git rev-parse --git-common-dir",
            "fatal: Invalid path '/mnt': No such file or directory",
        )
        .unwrap();
        for expected in [
            "Git needs attention",
            "config:3",
            "E:/gwt/work/issue-5255",
            "--unset core.worktree",
            "core.bare true",
            "core.bare=false",
        ] {
            assert!(
                diagnosis.contains(expected),
                "missing {expected}: {diagnosis}"
            );
        }
        assert_eq!(
            cached_metadata_failure(&bare).as_deref(),
            Some(diagnosis.as_str())
        );
        assert_eq!(
            std::fs::read(&config).unwrap(),
            before,
            "diagnosis must never repair configuration"
        );
        std::fs::write(&config, "[core]\n bare = true\n").unwrap();
        assert!(cached_metadata_failure(&bare).is_none());
        std::fs::remove_file(bare.join("HEAD")).unwrap();
        record_metadata_failure(
            &bare,
            "git rev-parse --git-common-dir",
            "fatal: not a git repository",
        )
        .unwrap();
        assert!(cached_metadata_failure(&bare).is_some());
        std::fs::write(bare.join("HEAD"), "ref: refs/heads/develop\n").unwrap();
        assert!(
            cached_metadata_failure(&bare).is_none(),
            "restoring HEAD must resume Git inspection"
        );
        let included = temp.path().join("included.config");
        std::fs::write(&included, "invalid config").unwrap();
        record_metadata_failure(
            &bare,
            "git rev-parse --git-common-dir",
            &format!("fatal: bad config line 1 in file {}", included.display()),
        )
        .unwrap();
        assert!(cached_metadata_failure(&bare).is_some());
        std::fs::write(&included, "[core]\n bare = true\n").unwrap();
        assert!(
            cached_metadata_failure(&bare).is_none(),
            "repairing the reported include must resume Git inspection"
        );
        assert!(record_metadata_failure(&bare, "git fetch", "process deadline expired").is_none());
        assert!(cached_metadata_failure(&bare).is_none());
    }
}
