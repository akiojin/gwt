use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};

use crate::cli::gwtd_resolver::default_installed_candidates;
use crate::native_app::{GUI_FRONT_DOOR_BINARY_NAME, INTERNAL_DAEMON_BINARY_NAME};
use fs2::FileExt;
use gwt_agent::AgentId;
use gwt_skills::pm_guidance::{generate_pm_guidance_for_claude, generate_pm_guidance_for_codex};
use gwt_skills::{
    distribute_to_worktree_for_targets_with_policy, generate_codex_hooks_for_mode,
    generate_coordination_guidance_for_claude, generate_coordination_guidance_for_codex,
    generate_hermes_hooks, generate_openclaw_hooks, generate_opencode_hooks,
    generate_settings_local, update_git_exclude, update_git_exclude_for_targets,
    CodexHookDiscoveryMode, ManagedAssetTarget,
};

/// Which `.codex/hooks.json` copies a non-launch (re-)materialization owns.
///
/// #3474: the self-heal writer ran with [`CodexHookDiscoveryMode::WorkspaceHome`],
/// which for a linked worktree resolves to the repo-root copy, while the health
/// auditor only ever read the worktree-local copy. A stale worktree-local file
/// was therefore reported forever and rewritten never. Outside a launch, gwt
/// does not know which Codex version will open the worktree, so it owns BOTH
/// discovery locations — the auditor reads the same set (see
/// [`managed_codex_hook_paths`]). Only the launch path narrows this, from the
/// Codex version it is actually about to run.
pub const MANAGED_CODEX_HOOK_DISCOVERY_MODE: CodexHookDiscoveryMode = CodexHookDiscoveryMode::Both;

fn with_managed_asset_lock<T>(
    worktree: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let identity_root = gwt_git::worktree::main_worktree_root(worktree).unwrap_or_else(|_| {
        dunce::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf())
    });
    let identity = gwt_core::repo_hash::compute_path_hash(&identity_root);
    let lock_dir = gwt_core::paths::gwt_home().join("locks/managed-assets");
    fs::create_dir_all(&lock_dir)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_dir.join(format!("{identity}.lock")))?;
    gwt_core::operation_deadline::lock_exclusive(&lock)?;
    let result = operation();
    let unlock = FileExt::unlock(&lock);
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

/// Every `.codex/hooks.json` gwt owns for `worktree`: the worktree-local copy
/// (read by Codex before 0.131.0-alpha.21) and the workspace-home copy (read by
/// newer Codex). Deduplicated when both resolve to the same file.
pub fn managed_codex_hook_paths(worktree: &Path) -> Vec<PathBuf> {
    gwt_skills::codex_hooks_paths_for_codex_discovery(worktree, MANAGED_CODEX_HOOK_DISCOVERY_MODE)
}

/// Resolve the only Host Codex config whose project trust lifecycle gwt can
/// own without durable per-launch provenance.
///
/// A process-level absolute `CODEX_HOME` is stable across worktrees. When it
/// is absent, Codex uses the OS user home. Relative or profile-only homes are
/// deliberately unsupported because their launch-time path cannot be
/// recovered exactly by every later cleanup route.
pub fn process_stable_codex_config_path_with(
    process_codex_home: Option<&OsStr>,
    os_user_home: Option<&Path>,
) -> Option<PathBuf> {
    let codex_home = match process_codex_home.filter(|value| !value.is_empty()) {
        Some(value) => {
            let configured = PathBuf::from(value);
            if !configured.is_absolute() {
                return None;
            }
            dunce::canonicalize(&configured).unwrap_or(configured)
        }
        None => {
            let home = os_user_home?.to_path_buf();
            if !home.is_absolute() {
                return None;
            }
            home.join(".codex")
        }
    };
    Some(gwt_core::paths::normalize_windows_child_process_path(&codex_home).join("config.toml"))
}

/// Resolve the shared Host Codex config for one managed worktree.
///
/// Even an absolute process-level `CODEX_HOME` is not a shared lifecycle
/// target when it lives inside the worktree that gwt will eventually remove.
pub fn process_stable_codex_config_path_for_worktree_with(
    worktree: &Path,
    process_codex_home: Option<&OsStr>,
    os_user_home: Option<&Path>,
) -> Option<PathBuf> {
    let config_path = process_stable_codex_config_path_with(process_codex_home, os_user_home)?;
    let comparable_config = comparable_lifecycle_path(&config_path);
    let comparable_worktree = comparable_lifecycle_path(worktree);
    (!path_is_same_or_descendant(&comparable_config, &comparable_worktree)).then_some(config_path)
}

fn comparable_lifecycle_path(path: &Path) -> PathBuf {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        if let Ok(mut canonical) = dunce::canonicalize(ancestor) {
            for component in suffix.iter().rev() {
                canonical.push(component);
            }
            return gwt_core::paths::normalize_windows_child_process_path(&canonical);
        }
        let (Some(file_name), Some(parent)) = (ancestor.file_name(), ancestor.parent()) else {
            break;
        };
        suffix.push(file_name.to_os_string());
        ancestor = parent;
    }
    gwt_core::paths::normalize_windows_child_process_path(path)
}

/// Compare two Codex config paths after resolving the deepest existing
/// component and normalizing child-process path syntax.
pub fn codex_config_paths_equivalent(left: &Path, right: &Path) -> bool {
    let left = comparable_lifecycle_path(left);
    let right = comparable_lifecycle_path(right);
    paths_equivalent(&left, &right)
}

#[cfg(not(windows))]
fn paths_equivalent(left: &Path, right: &Path) -> bool {
    left == right
}

#[cfg(windows)]
fn paths_equivalent(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .replace('/', "\\")
        .eq_ignore_ascii_case(&right.to_string_lossy().replace('/', "\\"))
}

#[cfg(not(windows))]
fn path_is_same_or_descendant(candidate: &Path, root: &Path) -> bool {
    candidate.starts_with(root)
}

#[cfg(windows)]
fn path_is_same_or_descendant(candidate: &Path, root: &Path) -> bool {
    let candidate = candidate
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase();
    let root = root
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase();
    candidate == root
        || candidate
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

/// Revoke gwt-owned Codex project trust and remove a managed worktree as one
/// config-lock transaction.
///
/// The callback runs while `config.toml.gwt-lock` is held, so a concurrent
/// registrar cannot republish trust after filesystem removal. When the
/// process uses a relative `CODEX_HOME`, no project trust is automatically
/// written and cleanup proceeds without touching that user-owned config.
pub fn cleanup_worktree_with_codex_project_trust<T>(
    worktree: &Path,
    cleanup: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let Some(config_path) = process_stable_codex_config_path_for_worktree_with(
        worktree,
        std::env::var_os("CODEX_HOME").as_deref(),
        dirs::home_dir().as_deref(),
    ) else {
        return cleanup();
    };
    gwt_skills::revoke_codex_managed_project_trust_with_cleanup(
        worktree,
        &config_path,
        cleanup,
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "Codex project trust cleanup transaction failed for worktree {} (config {}): {error}",
                worktree.display(),
                config_path.display()
            ),
        )
    })
}

/// Whether a present worktree-local merged hook config contains only
/// gwt-generated content. Callers may discard such a file at an explicit
/// lifecycle boundary, but must keep deletions, symlinks/reparse points, and
/// any config containing user-owned keys or hooks.
pub fn managed_hook_config_is_disposable(worktree: &Path, entry: &str) -> bool {
    if entry != ".claude/settings.local.json" && entry != ".codex/hooks.json" {
        return false;
    }
    let path = worktree.join(entry);
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return false;
    };
    if !metadata.file_type().is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }
    !gwt_skills::managed_hook_config_has_user_content(&path)
}

pub fn refresh_managed_gwt_assets_for_worktree(worktree: &Path) -> io::Result<()> {
    with_managed_asset_lock(worktree, || {
        crate::cli::memory::migrate_legacy_memory_file(worktree).ok();
        crate::cli::discussion::migrate_legacy_discussions_file(worktree).ok();
        materialize_managed_gwt_assets_for_targets(
            worktree,
            &ManagedAssetTarget::ALL,
            MANAGED_CODEX_HOOK_DISCOVERY_MODE,
            worktree_is_ephemeral(worktree),
        )?;
        update_git_exclude(worktree).map_err(|error| {
            io::Error::other(format!("failed to update gwt managed excludes: {error}"))
        })?;
        Ok(())
    })
}

/// Refresh the resident PM's own managed assets without mutating the linked
/// main checkout's workspace-home Codex hooks. Launch materialization owns
/// provider-specific workspace-home discovery; safe-boundary refreshes are
/// confined to the canonical PM checkout.
pub fn refresh_managed_gwt_assets_for_pm_worktree(worktree: &Path) -> io::Result<()> {
    with_managed_asset_lock(worktree, || {
        refresh_managed_gwt_assets_for_pm_worktree_locked(worktree)
    })
}

/// Caller holds the managed-assets lock through repoint and regeneration.
pub(crate) fn refresh_managed_gwt_assets_for_pm_worktree_locked(worktree: &Path) -> io::Result<()> {
    if !crate::pm_registry::is_pm_worktree(worktree) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a canonical PM worktree: {}", worktree.display()),
        ));
    }
    let snapshot = PmManagedAssetSnapshot::capture(worktree)?;
    let refresh = (|| {
        materialize_managed_gwt_assets_for_targets(
            worktree,
            &ManagedAssetTarget::ALL,
            CodexHookDiscoveryMode::WorktreeLocal,
            false,
        )?;
        update_git_exclude(worktree).map_err(|error| {
            io::Error::other(format!("failed to update PM managed excludes: {error}"))
        })
    })();
    match refresh {
        Ok(()) => {
            snapshot.discard();
            crate::cli::memory::migrate_legacy_memory_file(worktree).ok();
            crate::cli::discussion::migrate_legacy_discussions_file(worktree).ok();
            Ok(())
        }
        Err(error) => match snapshot.restore() {
            Ok(()) => Err(error),
            Err(restore_error) => Err(io::Error::other(format!(
                "{error}; restoring prior PM managed assets also failed: {restore_error}"
            ))),
        },
    }
}

/// Preserve only untracked gwt-owned collisions before the non-force Git
/// repoint. The same lock also covers the caller's checkout and regeneration.
/// Backups remain outside the checkout after success and after rollback.
pub(crate) fn with_pm_repoint_transaction<T>(
    worktree: &Path,
    target: &str,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    with_managed_asset_lock(worktree, || {
        let tree = pm_repoint_target_tree(worktree, target)?;
        let tracked = pm_repoint_git(worktree, &["ls-files", "-z"])?;
        let tracked = std::str::from_utf8(&tracked)
            .map_err(|_| io::Error::other("user-owned changes: non-UTF8 index path"))?
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect::<BTreeSet<_>>();
        let index_owns = |path: &Path| {
            path.ancestors().any(|entry| tracked.contains(entry))
                || tracked
                    .range(path.to_path_buf()..)
                    .next()
                    .is_some_and(|entry| entry.starts_with(path))
        };
        let mut candidates = Vec::new();
        let mut protected = Vec::new();
        for (path, (mode, _)) in &tree {
            if index_owns(path) {
                continue;
            }
            match pm_repoint_existing_file(worktree, path) {
                Ok(false) => continue,
                Ok(true) if matches!(mode.as_str(), "100644" | "100755") => {}
                _ => {
                    protected.push(path.clone());
                    continue;
                }
            }
            if gwt_skills::is_gwt_managed_skill_or_command_path(path)
                || pm_repoint_work_history_path(path)
            {
                candidates.push(path.clone());
            } else {
                protected.push(path.clone());
            }
        }
        if !protected.is_empty() {
            return Err(io::Error::other(format!(
                "user-owned changes: {} incoming path collision(s): {}",
                protected.len(),
                protected
                    .iter()
                    .take(5)
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if candidates.is_empty() {
            return operation();
        }

        let project_state = worktree
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| io::Error::other("managed artifacts: PM has no project-state root"))?
            .join("project-state");
        let backup_root = project_state
            .join("pm-repoint-backups")
            .join(uuid::Uuid::new_v4().to_string());
        pm_repoint_create_dir(&backup_root)?;
        let mut entries = Vec::new();
        let result = (|| {
            for relative in candidates {
                entries.push(PmRepointBackup::capture(worktree, &backup_root, relative)?);
            }
            let incoming_history = validate_pm_repoint_incoming_history(worktree, &tree, &entries)?;
            let history = entries
                .iter()
                .filter(|entry| pm_repoint_work_history_path(&entry.relative))
                .map(|entry| entry.relative.clone())
                .collect::<Vec<_>>();
            for relative in history {
                validate_pm_repoint_history_name(worktree, &relative)?;
                let paths =
                    gwt_core::workspace_projection::preserve_workspace_work_event_log_as_shards(
                        &worktree.join(&relative),
                        &worktree.join(".gwt/work/events"),
                    )
                    .map_err(|error| io::Error::other(format!("Work history: {error}")))?;
                for path in paths {
                    let relative = path.strip_prefix(worktree).map_err(|_| {
                        io::Error::other("Work history: preserved shard escaped checkout")
                    })?;
                    let Some((mode, _)) = tree.get(relative) else {
                        continue;
                    };
                    if !matches!(mode.as_str(), "100644" | "100755")
                        || incoming_history.get(relative) != Some(&fs::read(&path)?)
                    {
                        return Err(io::Error::other(format!(
                            "Work history: incoming shard differs from preserved event: {}",
                            relative.display()
                        )));
                    }
                    if !index_owns(relative)
                        && !entries.iter().any(|entry| entry.relative == relative)
                    {
                        entries.push(PmRepointBackup::capture(
                            worktree,
                            &backup_root,
                            relative.to_path_buf(),
                        )?);
                    }
                }
            }
            for entry in &mut entries {
                let path = worktree.join(&entry.relative);
                if !pm_repoint_existing_file(worktree, &entry.relative)?
                    || fs::read(&path)? != entry.bytes
                {
                    return Err(io::Error::other(format!(
                        "managed artifacts: source changed before repoint: {}",
                        entry.relative.display()
                    )));
                }
                fs::remove_file(&path)?;
                entry.displaced = true;
                pm_repoint_sync_dir(path.parent().expect("collision has parent"))?;
            }
            operation()
        })();
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                let mut failures = Vec::new();
                for entry in entries.iter().filter(|entry| entry.displaced) {
                    if let Err(restore) = entry.restore(worktree) {
                        failures.push(format!("{}: {restore}", entry.relative.display()));
                    }
                }
                Err(io::Error::other(format!(
                    "{error}; managed artifacts / Work history backup retained at {}{}",
                    backup_root.display(),
                    if failures.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "; {} restore failure(s): {}",
                            failures.len(),
                            failures.into_iter().take(5).collect::<Vec<_>>().join(", ")
                        )
                    }
                )))
            }
        }
    })
}

fn pm_repoint_git(worktree: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let output = gwt_core::process::run_git_logged(args, Some(worktree))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "managed artifacts: git {} failed: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(400)
                .collect::<String>()
                .trim()
        )));
    }
    Ok(output.stdout)
}

fn pm_repoint_target_tree(
    worktree: &Path,
    target: &str,
) -> io::Result<BTreeMap<PathBuf, (String, String)>> {
    let output = pm_repoint_git(worktree, &["ls-tree", "-r", "-z", "--full-tree", target])?;
    let text = std::str::from_utf8(&output)
        .map_err(|_| io::Error::other("user-owned changes: non-UTF8 incoming tree path"))?;
    text.split('\0')
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (metadata, path) = entry
                .split_once('\t')
                .ok_or_else(|| io::Error::other("managed artifacts: invalid Git tree entry"))?;
            let fields = metadata.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3
                || !Path::new(path)
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
            {
                return Err(io::Error::other("managed artifacts: invalid Git tree path"));
            }
            Ok((
                PathBuf::from(path),
                (fields[0].to_string(), fields[2].to_string()),
            ))
        })
        .collect()
}

/// Inspect every managed parent without following symlinks/reparse points.
fn pm_repoint_existing_file(worktree: &Path, relative: &Path) -> io::Result<bool> {
    let mut path = worktree.to_path_buf();
    for part in relative.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err(io::Error::other(
                "user-owned changes: noncanonical collision path",
            ));
        }
        path.push(part);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        reject_pm_managed_asset_indirection(&path, &metadata)?;
        let valid = if path == worktree.join(relative) {
            metadata.is_file()
        } else {
            metadata.is_dir()
        };
        if !valid {
            return Err(io::Error::other(format!(
                "user-owned changes: unsupported collision node: {}",
                path.display()
            )));
        }
    }
    Ok(true)
}

fn pm_repoint_work_history_path(relative: &Path) -> bool {
    if relative == Path::new(".gwt/work/events.jsonl") {
        return true;
    }
    let Ok(tail) = relative.strip_prefix(".gwt/work/events") else {
        return false;
    };
    let Some(name) = tail
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".jsonl"))
    else {
        return false;
    };
    if name.len() != 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return false;
    }
    tail == Path::new(&format!("{name}.jsonl"))
        || tail == Path::new(&format!("{}/{name}.jsonl", &name[..2]))
}

fn validate_pm_repoint_history_name(worktree: &Path, relative: &Path) -> io::Result<()> {
    if relative == Path::new(".gwt/work/events.jsonl") {
        return Ok(());
    }
    let bytes = fs::read(worktree.join(relative))?;
    let mut lines = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace));
    let value: serde_json::Value = serde_json::from_slice(lines.next().unwrap_or_default())
        .map_err(|error| io::Error::other(format!("Work history: invalid shard: {error}")))?;
    let id = value["id"]
        .as_str()
        .ok_or_else(|| io::Error::other("Work history: shard lacks event id"))?;
    let canonical = gwt_core::paths::gwt_work_event_shard_path(Path::new(".gwt/work/events"), id);
    if lines.next().is_some() || canonical.file_name() != relative.file_name() {
        return Err(io::Error::other(format!(
            "Work history: shard path does not match event id: {}",
            relative.display()
        )));
    }
    Ok(())
}

fn pm_repoint_event_lines(bytes: &[u8]) -> io::Result<BTreeMap<String, &[u8]>> {
    let mut events = BTreeMap::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
    {
        let value: serde_json::Value = serde_json::from_slice(line).map_err(|error| {
            io::Error::other(format!("Work history: invalid incoming event: {error}"))
        })?;
        let id = value["id"]
            .as_str()
            .ok_or_else(|| io::Error::other("Work history: event lacks id"))?;
        if events
            .insert(id.to_string(), line)
            .is_some_and(|previous| previous != line)
        {
            return Err(io::Error::other(
                "Work history: divergent duplicate event id",
            ));
        }
    }
    Ok(events)
}

fn validate_pm_repoint_incoming_history(
    worktree: &Path,
    tree: &BTreeMap<PathBuf, (String, String)>,
    entries: &[PmRepointBackup],
) -> io::Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut preserved = BTreeMap::new();
    for entry in entries
        .iter()
        .filter(|entry| pm_repoint_work_history_path(&entry.relative))
    {
        for (id, bytes) in pm_repoint_event_lines(&entry.bytes)? {
            if preserved
                .insert(id, bytes)
                .is_some_and(|previous| previous != bytes)
            {
                return Err(io::Error::other("Work history: divergent source event id"));
            }
        }
    }
    let mut incoming_blobs = BTreeMap::new();
    if preserved.is_empty() {
        return Ok(incoming_blobs);
    }
    let incoming = tree
        .iter()
        .filter(|(path, _)| pm_repoint_work_history_path(path))
        .collect::<Vec<_>>();
    // Bound stdin below a pipe buffer: the existing batch helper writes its
    // requests before draining stdout, which may contain a large legacy log.
    for chunk in incoming.chunks(32) {
        if chunk
            .iter()
            .any(|(_, (mode, _))| !matches!(mode.as_str(), "100644" | "100755"))
        {
            return Err(io::Error::other(
                "Work history: incoming history is not a regular file",
            ));
        }
        let oids = chunk
            .iter()
            .map(|(_, (_, oid))| oid.clone())
            .collect::<Vec<_>>();
        let blobs = gwt_git::blob::read_blob_bytes_batch(worktree, &oids).map_err(|error| {
            io::Error::other(format!(
                "Work history: {}",
                error.to_string().chars().take(400).collect::<String>()
            ))
        })?;
        for ((path, _), blob) in chunk.iter().zip(blobs) {
            for (id, bytes) in pm_repoint_event_lines(&blob)? {
                if preserved
                    .get(&id)
                    .is_some_and(|original| *original != bytes)
                {
                    return Err(io::Error::other(format!(
                        "Work history: incoming event differs from preserved history: {}",
                        path.display()
                    )));
                }
            }
            incoming_blobs.insert((*path).clone(), blob);
        }
    }
    Ok(incoming_blobs)
}

struct PmRepointBackup {
    relative: PathBuf,
    backup: PathBuf,
    bytes: Vec<u8>,
    permissions: fs::Permissions,
    displaced: bool,
}

impl PmRepointBackup {
    fn capture(worktree: &Path, backup_root: &Path, relative: PathBuf) -> io::Result<Self> {
        if !pm_repoint_existing_file(worktree, &relative)? {
            return Err(io::Error::other(
                "managed artifacts: collision disappeared before backup",
            ));
        }
        let source = worktree.join(&relative);
        let bytes = fs::read(&source)?;
        let permissions = fs::metadata(&source)?.permissions();
        let backup = backup_root.join(&relative);
        pm_repoint_create_dir(backup.parent().expect("backup parent"))?;
        pm_repoint_write_new(&backup, &bytes, &permissions)?;
        if fs::read(&backup)? != bytes {
            return Err(io::Error::other(
                "managed artifacts: backup verification failed",
            ));
        }
        Ok(Self {
            relative,
            backup,
            bytes,
            permissions,
            displaced: false,
        })
    }

    fn restore(&self, worktree: &Path) -> io::Result<()> {
        if pm_repoint_existing_file(worktree, &self.relative)? {
            if fs::read(worktree.join(&self.relative))? == self.bytes {
                return Ok(());
            }
            return Err(io::Error::other(
                "user-owned changes: refusing to overwrite current file",
            ));
        }
        let path = worktree.join(&self.relative);
        pm_repoint_create_dir(path.parent().expect("restore parent"))?;
        let bytes = fs::read(&self.backup)?;
        if bytes != self.bytes {
            return Err(io::Error::other(
                "backup verification failed during restore",
            ));
        }
        pm_repoint_write_new(&path, &bytes, &self.permissions)
    }
}

fn pm_repoint_write_new(
    path: &Path,
    bytes: &[u8],
    permissions: &fs::Permissions,
) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.set_permissions(permissions.clone())?;
    file.sync_all()?;
    pm_repoint_sync_dir(path.parent().expect("written file parent"))
}

fn pm_repoint_create_dir(path: &Path) -> io::Result<()> {
    if !pm_managed_asset_node_exists(path)? {
        if let Some(parent) = path.parent() {
            pm_repoint_create_dir(parent)?;
        }
    }
    ensure_real_pm_managed_asset_directory(path)?;
    pm_repoint_sync_dir(path)?;
    if let Some(parent) = path.parent() {
        pm_repoint_sync_dir(parent)?;
    }
    Ok(())
}

fn pm_repoint_sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

const PM_MANAGED_ASSET_TRANSACTION_ROOTS: &[&str] = &[
    ".claude",
    ".codex",
    ".gwt/opencode",
    ".gwt/openclaw",
    ".gwt/hermes",
];

struct PmManagedAssetSnapshot {
    entries: Vec<PmManagedAssetSnapshotEntry>,
    gwt_parent: PathBuf,
    gwt_parent_existed: bool,
    backup_root: PathBuf,
}

struct PmManagedAssetSnapshotEntry {
    target: PathBuf,
    backup: PathBuf,
    existed: bool,
}

impl PmManagedAssetSnapshot {
    fn capture(worktree: &Path) -> io::Result<Self> {
        let project_state = worktree
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| io::Error::other("canonical PM worktree has no project-state root"))?
            .join("project-state");
        ensure_real_pm_managed_asset_directory(&project_state)?;
        let gwt_parent = worktree.join(".gwt");
        let gwt_parent_existed = pm_managed_asset_node_exists(&gwt_parent)?;
        if gwt_parent_existed {
            let metadata = fs::symlink_metadata(&gwt_parent)?;
            reject_pm_managed_asset_indirection(&gwt_parent, &metadata)?;
            if !metadata.file_type().is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "PM managed provider-home parent is not a directory: {}",
                        gwt_parent.display()
                    ),
                ));
            }
        }
        let mut targets = PM_MANAGED_ASSET_TRANSACTION_ROOTS
            .iter()
            .map(|relative| worktree.join(relative))
            .collect::<Vec<_>>();
        targets.push(resolve_git_exclude_path(worktree)?);
        let exclude_backup = unique_pm_managed_asset_sibling(
            targets
                .last()
                .expect("Git exclude is always appended to the transaction targets"),
            "backup",
        )?;
        let target_presence = targets
            .iter()
            .map(|target| pm_managed_asset_node_exists(target))
            .collect::<io::Result<Vec<_>>>()?;
        let backup_parent = project_state.join("pm-managed-assets-backups");
        ensure_real_pm_managed_asset_directory(&backup_parent)?;
        let backup_root =
            backup_parent.join(format!("{}-{}", std::process::id(), uuid::Uuid::new_v4()));
        fs::create_dir(&backup_root)?;
        let mut entries: Vec<PmManagedAssetSnapshotEntry> = Vec::with_capacity(targets.len());
        for (index, (target, existed)) in targets.into_iter().zip(target_presence).enumerate() {
            let backup = if index < PM_MANAGED_ASSET_TRANSACTION_ROOTS.len() {
                backup_root.join(index.to_string())
            } else {
                exclude_backup.clone()
            };
            if existed {
                if let Err(error) = copy_pm_managed_asset_node(&target, &backup) {
                    let mut cleanup_failures = Vec::new();
                    if let Err(cleanup_error) = remove_pm_managed_asset_node(&backup) {
                        cleanup_failures.push(format!("{}: {cleanup_error}", backup.display()));
                    }
                    for captured in &entries {
                        if let Err(cleanup_error) = remove_pm_managed_asset_node(&captured.backup) {
                            cleanup_failures
                                .push(format!("{}: {cleanup_error}", captured.backup.display()));
                        }
                    }
                    if cleanup_failures.is_empty() {
                        if let Err(cleanup_error) = fs::remove_dir(&backup_root) {
                            cleanup_failures
                                .push(format!("{}: {cleanup_error}", backup_root.display()));
                        }
                    }
                    if !cleanup_failures.is_empty() {
                        return Err(io::Error::other(format!(
                            "{error}; incomplete PM managed asset capture cleanup retained recovery data under {}: {}",
                            backup_root.display(), cleanup_failures.join("; ")
                        )));
                    }
                    return Err(error);
                }
            }
            entries.push(PmManagedAssetSnapshotEntry {
                target,
                backup,
                existed,
            });
        }
        Ok(Self {
            entries,
            gwt_parent,
            gwt_parent_existed,
            backup_root,
        })
    }

    fn restore(self) -> io::Result<()> {
        let mut failures = Vec::new();
        for entry in &self.entries {
            let quarantine = match unique_pm_managed_asset_sibling(&entry.target, "failed") {
                Ok(path) => path,
                Err(error) => {
                    failures.push(format!(
                        "prepare rollback for {}: {error}; backup retained at {}",
                        entry.target.display(),
                        entry.backup.display()
                    ));
                    continue;
                }
            };
            let target_present = match pm_managed_asset_node_exists(&entry.target) {
                Ok(present) => present,
                Err(error) => {
                    failures.push(format!(
                        "inspect changed {}: {error}; backup retained at {}",
                        entry.target.display(),
                        entry.backup.display()
                    ));
                    continue;
                }
            };
            if target_present {
                if let Err(error) = fs::rename(&entry.target, &quarantine) {
                    failures.push(format!(
                        "quarantine changed {}: {error}; backup retained at {}",
                        entry.target.display(),
                        entry.backup.display()
                    ));
                    continue;
                }
            }
            if entry.existed {
                if let Err(error) = fs::rename(&entry.backup, &entry.target) {
                    let put_back = if target_present {
                        fs::rename(&quarantine, &entry.target)
                            .map_err(|restore_error| restore_error.to_string())
                    } else {
                        Ok(())
                    };
                    failures.push(format!(
                        "restore {}: {error}; changed-tree recovery={put_back:?}; backup retained at {}",
                        entry.target.display(),
                        entry.backup.display()
                    ));
                    continue;
                }
            }
            if target_present {
                if let Err(error) = remove_pm_managed_asset_node(&quarantine) {
                    failures.push(format!(
                        "remove failed PM managed asset quarantine {}: {error}",
                        quarantine.display()
                    ));
                }
            }
        }
        if !self.gwt_parent_existed {
            match fs::remove_dir(&self.gwt_parent) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => failures.push(format!(
                    "remove newly-created empty PM provider-home parent {}: {error}",
                    self.gwt_parent.display()
                )),
            }
        }
        if failures.is_empty() {
            if let Err(error) = fs::remove_dir(&self.backup_root) {
                tracing::warn!(
                    path = %self.backup_root.display(),
                    %error,
                    "restored PM managed assets but could not discard the empty external recovery directory"
                );
            }
            Ok(())
        } else {
            failures.push(format!(
                "external PM managed asset recovery directory retained at {}",
                self.backup_root.display()
            ));
            Err(io::Error::other(failures.join("; ")))
        }
    }

    fn discard(self) {
        for entry in &self.entries {
            if let Err(error) = remove_pm_managed_asset_node(&entry.backup) {
                tracing::warn!(
                    path = %entry.backup.display(),
                    %error,
                    "failed to discard PM managed asset transaction backup"
                );
            }
        }
        if let Err(error) = remove_pm_managed_asset_node(&self.backup_root) {
            tracing::warn!(
                path = %self.backup_root.display(),
                %error,
                "PM managed assets committed with an external recovery residue"
            );
        }
    }
}

fn ensure_real_pm_managed_asset_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            reject_pm_managed_asset_indirection(path, &metadata)?;
            if metadata.file_type().is_dir() {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "PM managed asset transaction path is not a directory: {}",
                        path.display()
                    ),
                ))
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(path),
        Err(error) => Err(error),
    }
}

fn pm_managed_asset_node_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn unique_pm_managed_asset_sibling(target: &Path, role: &str) -> io::Result<PathBuf> {
    let parent = target.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("PM managed asset path has no parent: {}", target.display()),
        )
    })?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("managed-asset");
    Ok(parent.join(format!(
        ".{name}.gwt-pm-{role}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    )))
}

fn resolve_git_exclude_path(worktree: &Path) -> io::Result<PathBuf> {
    let output = gwt_core::process::run_git_logged(
        &["rev-parse", "--git-path", "info/exclude"],
        Some(worktree),
    )?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "resolve PM Git exclude path: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if path.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "resolve PM Git exclude path: Git returned an empty path",
        ));
    }
    Ok(if path.is_absolute() {
        path
    } else {
        worktree.join(path)
    })
}

fn remove_pm_managed_asset_node(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    reject_pm_managed_asset_indirection(path, &metadata)?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn copy_pm_managed_asset_node(source: &Path, destination: &Path) -> io::Result<()> {
    copy_pm_managed_asset_node_at(source, destination, source)
}

fn copy_pm_managed_asset_node_at(
    source: &Path,
    destination: &Path,
    transaction_root: &Path,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        if !pm_managed_asset_symlink_is_allowed(transaction_root, source) {
            reject_pm_managed_asset_indirection(source, &metadata)?;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        return copy_pm_managed_asset_symlink(source, destination, &metadata);
    }
    reject_pm_managed_asset_indirection(source, &metadata)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if metadata.file_type().is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_pm_managed_asset_node_at(
                &entry.path(),
                &destination.join(entry.file_name()),
                transaction_root,
            )?;
        }
        fs::set_permissions(destination, metadata.permissions())?;
        return Ok(());
    }
    if metadata.file_type().is_file() {
        fs::copy(source, destination)?;
        fs::set_permissions(destination, metadata.permissions())?;
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "unsupported node in PM managed asset transaction: {}",
            source.display()
        ),
    ))
}

fn pm_managed_asset_symlink_is_allowed(transaction_root: &Path, source: &Path) -> bool {
    transaction_root.file_name().and_then(|name| name.to_str()) == Some("hermes")
        && transaction_root
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some(".gwt")
        && source.parent() == Some(transaction_root)
        && matches!(
            source.file_name().and_then(|name| name.to_str()),
            Some(".env" | "auth.json")
        )
}

#[cfg(unix)]
fn copy_pm_managed_asset_symlink(
    source: &Path,
    destination: &Path,
    _metadata: &fs::Metadata,
) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(source)?, destination)
}

#[cfg(windows)]
fn copy_pm_managed_asset_symlink(
    source: &Path,
    destination: &Path,
    metadata: &fs::Metadata,
) -> io::Result<()> {
    use std::os::windows::fs::{symlink_dir, symlink_file, FileTypeExt};

    let target = fs::read_link(source)?;
    if metadata.file_type().is_symlink_dir() {
        symlink_dir(target, destination)
    } else if metadata.file_type().is_symlink_file() {
        symlink_file(target, destination)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "unsupported symlink in PM managed asset transaction: {}",
                source.display()
            ),
        ))
    }
}

#[cfg(not(any(unix, windows)))]
fn copy_pm_managed_asset_symlink(
    source: &Path,
    _destination: &Path,
    _metadata: &fs::Metadata,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "symlink backup is unsupported on this platform: {}",
            source.display()
        ),
    ))
}

#[cfg(windows)]
fn reject_pm_managed_asset_indirection(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    if metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "indirect node is not allowed in PM managed asset transaction: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn reject_pm_managed_asset_indirection(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "symlink is not allowed in PM managed asset transaction: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// What one managed-asset materialization committed to disk that a caller
/// still has to agree with afterwards.
///
/// #3967: `hook_bin` is the fallback binary the regenerated hook commands
/// embed. It is resolved once, here, and pinned into the environment only for
/// the duration of the generation call — so Codex trust pre-registration, which
/// must match those command strings byte for byte, reads it from this value
/// instead of re-deriving a second answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManagedAssetMaterialization {
    pub hook_bin: Option<String>,
}

pub fn refresh_managed_gwt_assets_for_agent(worktree: &Path, agent_id: &AgentId) -> io::Result<()> {
    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
        worktree,
        agent_id,
        MANAGED_CODEX_HOOK_DISCOVERY_MODE,
        worktree_is_ephemeral(worktree),
    )
    .map(|_| ())
}

pub fn refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
    worktree: &Path,
    agent_id: &AgentId,
    codex_hook_discovery_mode: CodexHookDiscoveryMode,
    is_ephemeral: bool,
) -> io::Result<ManagedAssetMaterialization> {
    with_managed_asset_lock(worktree, || {
        let targets = refresh_targets_for_agent(worktree, agent_id);
        let hook_bin = materialize_managed_gwt_assets_for_targets(
            worktree,
            &targets,
            codex_hook_discovery_mode,
            is_ephemeral,
        )?;
        let exclude_targets = detect_existing_managed_asset_targets(worktree);
        update_git_exclude_for_targets(worktree, &exclude_targets).map_err(|error| {
            io::Error::other(format!("failed to update gwt managed excludes: {error}"))
        })?;
        Ok(ManagedAssetMaterialization { hook_bin })
    })
}

pub fn refresh_existing_managed_gwt_assets_for_worktree(worktree: &Path) -> io::Result<()> {
    with_managed_asset_lock(worktree, || {
        let targets = detect_existing_managed_asset_targets(worktree);
        materialize_managed_gwt_assets_for_targets(
            worktree,
            &targets,
            MANAGED_CODEX_HOOK_DISCOVERY_MODE,
            worktree_is_ephemeral(worktree),
        )?;
        update_git_exclude_for_targets(worktree, &targets).map_err(|error| {
            io::Error::other(format!("failed to update gwt managed excludes: {error}"))
        })?;
        Ok(())
    })
}

/// Whether a non-launch (re-)materialization targets an ephemeral worktree.
/// SPEC #3245 FR-007 replaced the lane-file resolution with the structural
/// worktree-form predicate (`.intake` / `.intake-<n>` naming): the decision is
/// deterministic per worktree path, so an ambient env value from another
/// session can never redirect asset policy (#3377), and disposable ephemeral
/// worktrees keep the embedded-bundle override (#3374). (The launch path does
/// NOT use this: it passes `config.is_ephemeral` directly.)
fn worktree_is_ephemeral(worktree: &Path) -> bool {
    crate::worktree_form::is_ephemeral_worktree_path(worktree)
}

/// Materialize managed assets, returning the fallback binary the regenerated
/// hook commands were pinned to (`None` when no hook config was generated).
fn materialize_managed_gwt_assets_for_targets(
    worktree: &Path,
    targets: &[ManagedAssetTarget],
    codex_hook_discovery_mode: CodexHookDiscoveryMode,
    is_ephemeral: bool,
) -> io::Result<Option<String>> {
    // Fail fast with a clear, attributed error when the worktree was not
    // properly created (e.g. branch/worktree materialization failed). Without
    // this guard, distribution would silently `create_dir_all` a phantom tree
    // and the failure would surface much later as a misleading
    // "failed to generate Claude coordination skill: No such file or directory"
    // — attributing a worktree-setup failure to the skill writer.
    if !worktree.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "gwt managed assets: worktree is not a ready directory \
                 (branch/worktree creation likely failed): {}",
                worktree.display()
            ),
        ));
    }
    // #3374: an ephemeral worktree refreshes tracked gwt-* assets from the
    // embedded bundle — its tracked copies are a stale base-ref snapshot, not
    // user content. Persistent worktrees keep the preserve-tracked default.
    let policy = if is_ephemeral {
        gwt_skills::TrackedAssetWritePolicy::OverrideGwtManaged
    } else {
        gwt_skills::TrackedAssetWritePolicy::PreserveTracked
    };
    distribute_to_worktree_for_targets_with_policy(worktree, targets, policy).map_err(|error| {
        io::Error::other(format!("failed to distribute gwt managed assets: {error}"))
    })?;
    if targets.is_empty() {
        return Ok(None);
    }
    // SPEC-3431 T-052: gwt-pm is generated, not bundled, so the prune above
    // deletes it like any other unknown `gwt-*` skill. Regenerating here — the
    // one funnel every launch, resume, and refresh passes through — is what
    // makes the `$gwt-pm` bootstrap prompt resolvable at all. The predicate is
    // structural (canonical PM worktree path), so no other worktree can be
    // handed the PM contract by an ambient value.
    let is_pm = crate::pm_registry::is_pm_worktree(worktree);
    let hook_bin =
        regenerate_managed_hook_configs_for_targets(worktree, targets, codex_hook_discovery_mode)?;
    if targets.contains(&ManagedAssetTarget::ClaudeCode) {
        generate_coordination_guidance_for_claude(worktree).map_err(|error| {
            io::Error::other(format!(
                "failed to generate Claude coordination skill: {error}"
            ))
        })?;
        if is_pm {
            generate_pm_guidance_for_claude(worktree).map_err(|error| {
                io::Error::other(format!("failed to generate Claude PM skill: {error}"))
            })?;
        }
    }
    if targets.contains(&ManagedAssetTarget::Codex) {
        generate_coordination_guidance_for_codex(worktree).map_err(|error| {
            io::Error::other(format!(
                "failed to generate Codex coordination skill: {error}"
            ))
        })?;
        if is_pm {
            generate_pm_guidance_for_codex(worktree).map_err(|error| {
                io::Error::other(format!("failed to generate Codex PM skill: {error}"))
            })?;
        }
    }
    Ok(hook_bin)
}

pub fn regenerate_existing_managed_hook_configs(worktree: &Path) -> io::Result<()> {
    with_managed_asset_lock(worktree, || {
        let targets = detect_existing_managed_asset_targets(worktree);
        regenerate_managed_hook_configs_for_targets(
            worktree,
            &targets,
            MANAGED_CODEX_HOOK_DISCOVERY_MODE,
        )
        .map(|_| ())
    })
}

/// Regenerate the managed hook configs, returning the fallback binary they were
/// generated with (`None` when there was nothing to generate).
fn regenerate_managed_hook_configs_for_targets(
    worktree: &Path,
    targets: &[ManagedAssetTarget],
    codex_hook_discovery_mode: CodexHookDiscoveryMode,
) -> io::Result<Option<String>> {
    if targets.is_empty() {
        return Ok(None);
    }
    let (_hook_bin_guard, hook_bin) = install_hook_bin_override()?;
    if targets.contains(&ManagedAssetTarget::ClaudeCode) {
        generate_settings_local(worktree).map_err(|error| {
            io::Error::other(format!(
                "failed to regenerate Claude hook settings: {error}"
            ))
        })?;
    }
    if targets.contains(&ManagedAssetTarget::Codex) {
        generate_codex_hooks_for_mode(worktree, codex_hook_discovery_mode).map_err(|error| {
            io::Error::other(format!("failed to regenerate Codex hook settings: {error}"))
        })?;
    }
    if targets.contains(&ManagedAssetTarget::OpenCode) {
        generate_opencode_hooks(worktree).map_err(|error| {
            io::Error::other(format!(
                "failed to regenerate OpenCode hook settings: {error}"
            ))
        })?;
    }
    if targets.contains(&ManagedAssetTarget::OpenClaw) {
        generate_openclaw_hooks(worktree).map_err(|error| {
            io::Error::other(format!(
                "failed to regenerate OpenClaw hook settings: {error}"
            ))
        })?;
    }
    if targets.contains(&ManagedAssetTarget::Hermes) {
        generate_hermes_hooks(worktree).map_err(|error| {
            io::Error::other(format!(
                "failed to regenerate Hermes hook settings: {error}"
            ))
        })?;
    }
    Ok(Some(hook_bin))
}

fn managed_targets_for_agent(agent_id: &AgentId) -> Option<ManagedAssetTarget> {
    match agent_id {
        AgentId::ClaudeCode => Some(ManagedAssetTarget::ClaudeCode),
        AgentId::Codex => Some(ManagedAssetTarget::Codex),
        AgentId::OpenCode => Some(ManagedAssetTarget::OpenCode),
        AgentId::OpenClaw => Some(ManagedAssetTarget::OpenClaw),
        AgentId::Hermes => Some(ManagedAssetTarget::Hermes),
        AgentId::GrokBuild => Some(ManagedAssetTarget::ClaudeCode),
        AgentId::Antigravity | AgentId::Gemini | AgentId::Copilot | AgentId::Custom(_) => None,
    }
}

/// Targets a launch refresh must write: the launched provider plus every
/// managed provider surface the worktree already carries (#3233). Writing only
/// the launched provider left an existing mirror (e.g. `.codex/skills`) frozen
/// at whatever the previous build materialized, so a bundle asset added since
/// then appeared on one side only and broke `.claude` / `.codex` parity.
fn refresh_targets_for_agent(worktree: &Path, agent_id: &AgentId) -> Vec<ManagedAssetTarget> {
    // An agent with no managed surface of its own (Gemini, Copilot, …) still
    // launches inside a worktree whose existing `.claude` / `.codex` surfaces
    // must not be left frozen, so start from the optional primary instead of
    // returning early.
    let mut targets: Vec<ManagedAssetTarget> =
        managed_targets_for_agent(agent_id).into_iter().collect();
    for existing in detect_existing_managed_asset_targets(worktree) {
        push_existing_target(&mut targets, true, existing);
    }
    targets
}

fn detect_existing_managed_asset_targets(worktree: &Path) -> Vec<ManagedAssetTarget> {
    let mut targets = Vec::new();
    push_existing_target(
        &mut targets,
        worktree.join(".claude").exists()
            || worktree.join(".claude/skills").exists()
            || worktree.join(".claude/commands").exists()
            || worktree.join(".claude/settings.local.json").exists(),
        ManagedAssetTarget::ClaudeCode,
    );
    push_existing_target(
        &mut targets,
        worktree.join(".codex").exists()
            || worktree.join(".codex/skills").exists()
            || worktree.join(".codex/hooks.json").exists(),
        ManagedAssetTarget::Codex,
    );
    push_existing_target(
        &mut targets,
        worktree.join(".gwt/opencode").exists(),
        ManagedAssetTarget::OpenCode,
    );
    push_existing_target(
        &mut targets,
        worktree.join(".gwt/openclaw").exists(),
        ManagedAssetTarget::OpenClaw,
    );
    push_existing_target(
        &mut targets,
        worktree.join(".gwt/hermes").exists(),
        ManagedAssetTarget::Hermes,
    );
    targets
}

fn push_existing_target(
    targets: &mut Vec<ManagedAssetTarget>,
    exists: bool,
    target: ManagedAssetTarget,
) {
    if exists && !targets.contains(&target) {
        targets.push(target);
    }
}

/// Where the managed hook binary came from, which decides whether generation
/// still has to publish it to the environment.
enum HookBinPin {
    /// Already answered for this thread or process; the generator will read it
    /// without help.
    Ambient(String),
    /// Resolved here, so generation has to publish it for the duration of the
    /// materialization.
    Resolved(String),
}

/// The fallback binary a managed hook command embeds when a launch did not
/// inject `GWT_BIN_PATH`.
///
/// #3967: this is the one answer generation and Codex trust pre-registration
/// must share. It is published to the environment only while materialization
/// runs, so anything that needs it afterwards asks here rather than deriving a
/// second answer of its own — gwt-skills' library fallback resolves a gwt
/// started from a checkout build to `target/debug/gwtd`, which
/// `sanitize_hook_bin_for_config_path` then reduces to the bare `gwtd`, while
/// this resolver skips build outputs and pins the installed absolute path.
pub fn managed_hook_bin() -> io::Result<String> {
    Ok(match managed_hook_bin_pin()? {
        HookBinPin::Ambient(hook_bin) | HookBinPin::Resolved(hook_bin) => hook_bin,
    })
}

fn managed_hook_bin_pin() -> io::Result<HookBinPin> {
    // #4057: a thread-local test pin already answers the generator, so leave
    // the process environment alone — mutating it here would leak this
    // thread's binary into every other materialization in the process.
    if let Some(hook_bin) = gwt_skills::settings_local::hook_bin_override() {
        return Ok(HookBinPin::Ambient(hook_bin));
    }
    if let Some(hook_bin) = std::env::var_os("GWT_HOOK_BIN")
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string_lossy().into_owned())
    {
        return Ok(HookBinPin::Ambient(hook_bin));
    }
    Ok(HookBinPin::Resolved(
        resolve_public_gwt_bin_path()?
            .to_string_lossy()
            .into_owned(),
    ))
}

/// Pin the fallback binary generated hook commands embed, and report it.
fn install_hook_bin_override() -> io::Result<(EnvVarGuard, String)> {
    match managed_hook_bin_pin()? {
        HookBinPin::Ambient(hook_bin) => Ok((EnvVarGuard::noop("GWT_HOOK_BIN"), hook_bin)),
        HookBinPin::Resolved(hook_bin) => {
            Ok((EnvVarGuard::set("GWT_HOOK_BIN", &hook_bin), hook_bin))
        }
    }
}

pub fn resolve_public_gwt_bin_path() -> io::Result<PathBuf> {
    let current_exe = std::env::current_exe()
        .map_err(|error| io::Error::other(format!("current_exe: {error}")))?;
    Ok(resolve_public_gwt_bin_with_lookup(
        &current_exe,
        |command| which::which(command).ok(),
    ))
}

pub fn resolve_public_gwt_bin_with_lookup(
    current_exe: &Path,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> PathBuf {
    resolve_public_gwt_bin_with_candidates(
        current_exe,
        default_installed_candidates(None),
        lookup,
        |candidate| candidate.is_file(),
    )
}

fn resolve_public_gwt_bin_with_candidates(
    current_exe: &Path,
    installed_candidates: impl IntoIterator<Item = PathBuf>,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
    is_file: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(candidate) = installed_candidates
        .into_iter()
        .find(|candidate| is_stable_hook_binary(candidate) && is_file(candidate))
    {
        return candidate;
    }

    if is_named_gwtd_binary(current_exe) && is_stable_hook_binary(current_exe) {
        return current_exe.to_path_buf();
    }

    if is_named_gwt_binary(current_exe) && is_stable_hook_binary(current_exe) {
        if let Some(candidate) = sibling_daemon_binary(current_exe)
            .filter(|candidate| is_stable_hook_binary(candidate) && is_file(candidate))
        {
            return candidate;
        }
    }

    if let Some(candidate) = lookup(INTERNAL_DAEMON_BINARY_NAME)
        .filter(|candidate| !same_path(candidate, current_exe) && is_stable_hook_binary(candidate))
    {
        return candidate;
    }

    PathBuf::from(INTERNAL_DAEMON_BINARY_NAME)
}

fn strip_windows_exe_suffix(value: &str) -> &str {
    value
        .rsplit_once('.')
        .filter(|(_, ext)| ext.eq_ignore_ascii_case("exe"))
        .map(|(stem, _)| stem)
        .unwrap_or(value)
}

fn is_named_gwt_binary(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .next_back()
        .map(|value| strip_windows_exe_suffix(&value).to_string())
        .is_some_and(|value| value.eq_ignore_ascii_case(GUI_FRONT_DOOR_BINARY_NAME))
}

fn is_named_gwtd_binary(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .next_back()
        .map(|value| strip_windows_exe_suffix(&value).to_string())
        .is_some_and(|value| value.eq_ignore_ascii_case(INTERNAL_DAEMON_BINARY_NAME))
}

fn is_bunx_temp_executable(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .any(|segment| segment.starts_with("bunx-"))
}

/// Whether `path` is a gwt build output owned by one checkout
/// (`<root>/target/[<triple>/]{debug,release}/gwt[d]`).
///
/// #3567: the hook generator asks the same question when it decides whether a
/// resolved binary may be written into another worktree's config, so both sides
/// share one implementation and cannot drift apart.
pub(crate) fn is_worktree_local_build_binary(path: &Path) -> bool {
    gwt_skills::build_output_owner_root(path).is_some()
}

fn is_stable_hook_binary(path: &Path) -> bool {
    !is_bunx_temp_executable(path) && !is_worktree_local_build_binary(path)
}

fn sibling_daemon_binary(path: &Path) -> Option<PathBuf> {
    if !is_named_gwt_binary(path) {
        return None;
    }
    let sibling_name = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("exe") => {
            format!("{INTERNAL_DAEMON_BINARY_NAME}.exe")
        }
        _ => INTERNAL_DAEMON_BINARY_NAME.to_string(),
    };
    Some(path.with_file_name(sibling_name))
}

fn normalized_path_segments(path: &Path) -> Vec<String> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = dunce::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = dunce::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
    restore: bool,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self {
            key,
            previous,
            restore: true,
        }
    }

    fn noop(key: &'static str) -> Self {
        Self {
            key,
            previous: None,
            restore: false,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if !self.restore {
            return;
        }
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
    };

    use super::{
        is_bunx_temp_executable, is_named_gwt_binary, is_named_gwtd_binary,
        is_worktree_local_build_binary, normalized_path_segments,
        resolve_public_gwt_bin_with_candidates, resolve_public_gwt_bin_with_lookup, same_path,
        EnvVarGuard,
    };

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn repoint_git(root: &Path, args: &[&str]) -> String {
        let output = gwt_core::process::run_git_logged(args, Some(root)).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn repoint_fixture(incoming: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf, String) {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("pm/worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        repoint_git(&worktree, &["init", "--quiet"]);
        repoint_git(&worktree, &["config", "user.name", "Test"]);
        repoint_git(&worktree, &["config", "user.email", "test@example.com"]);
        repoint_git(
            &worktree,
            &["commit", "--quiet", "--allow-empty", "-m", "base"],
        );
        let base = repoint_git(&worktree, &["rev-parse", "HEAD"]);
        for (path, bytes) in incoming {
            let path = worktree.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        repoint_git(&worktree, &["add", "."]);
        repoint_git(&worktree, &["commit", "--quiet", "-m", "incoming"]);
        let target = repoint_git(&worktree, &["rev-parse", "HEAD"]);
        repoint_git(&worktree, &["checkout", "--quiet", "--detach", &base]);
        (temp, worktree, target)
    }

    fn seed_repoint_collision(worktree: &Path, path: &str, bytes: &str) {
        let path = worktree.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn pm_repoint_transaction_preserves_ignored_managed_bytes_outside_checkout() {
        let asset = ".claude/skills/gwt-execute/SKILL.md";
        let (temp, worktree, target) = repoint_fixture(&[(asset, "incoming")]);
        seed_repoint_collision(&worktree, asset, "old generated bytes");
        seed_repoint_collision(&worktree, "user.txt", "user work");
        std::fs::write(worktree.join(".git/info/exclude"), ".claude/\n").unwrap();
        super::with_pm_repoint_transaction(&worktree, &target, || {
            assert!(!worktree.join(asset).exists());
            repoint_git(&worktree, &["checkout", "--quiet", "--detach", &target]);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(worktree.join(asset)).unwrap(),
            "incoming"
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join("user.txt")).unwrap(),
            "user work"
        );
        let backups = temp.path().join("project-state/pm-repoint-backups");
        let backup = std::fs::read_dir(backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            std::fs::read_to_string(backup.join(asset)).unwrap(),
            "old generated bytes"
        );
    }

    #[test]
    fn pm_repoint_transaction_restores_on_callback_failure_without_overwriting_new_content() {
        let asset = ".codex/skills/gwt-execute/SKILL.md";
        let (_temp, worktree, target) = repoint_fixture(&[(asset, "incoming")]);
        seed_repoint_collision(&worktree, asset, "original");
        let error = super::with_pm_repoint_transaction::<()>(&worktree, &target, || {
            Err(std::io::Error::other("repoint rejected"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("repoint rejected"));
        assert_eq!(
            std::fs::read_to_string(worktree.join(asset)).unwrap(),
            "original"
        );

        let error = super::with_pm_repoint_transaction::<()>(&worktree, &target, || {
            std::fs::write(worktree.join(asset), "concurrent user edit")?;
            Err(std::io::Error::other("repoint rejected"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("backup"), "{error}");
        assert_eq!(
            std::fs::read_to_string(worktree.join(asset)).unwrap(),
            "concurrent user edit"
        );
    }

    #[test]
    fn pm_repoint_transaction_rejects_user_collision_before_displacing_managed_files() {
        let asset = ".claude/commands/gwt-execute.md";
        let user = ".claude/skills/user/SKILL.md";
        let (_temp, worktree, target) = repoint_fixture(&[(asset, "incoming"), (user, "incoming")]);
        seed_repoint_collision(&worktree, asset, "generated");
        seed_repoint_collision(&worktree, user, "user owned");
        let error = super::with_pm_repoint_transaction::<()>(&worktree, &target, || {
            panic!("must reject before checkout")
        })
        .unwrap_err();
        assert!(error.to_string().contains("user-owned changes"), "{error}");
        assert_eq!(
            std::fs::read_to_string(worktree.join(asset)).unwrap(),
            "generated"
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join(user)).unwrap(),
            "user owned"
        );
    }

    #[test]
    fn pm_repoint_transaction_preserves_unknown_work_history_and_rejects_divergent_incoming_event()
    {
        let legacy = ".gwt/work/events.jsonl";
        let original = " {\"id\":\"event-future\",\"work_item_id\":\"work-future\",\"kind\":\"future-kind\",\"updated_at\":\"2026-09-10T00:00:00Z\",\"extra\":true}\n";
        let (_temp, worktree, target) = repoint_fixture(&[(legacy, "")]);
        seed_repoint_collision(&worktree, legacy, original);
        super::with_pm_repoint_transaction(&worktree, &target, || {
            repoint_git(&worktree, &["checkout", "--quiet", "--detach", &target]);
            Ok(())
        })
        .unwrap();
        let shard = gwt_core::paths::gwt_work_event_shard_path(
            &worktree.join(".gwt/work/events"),
            "event-future",
        );
        assert_eq!(std::fs::read_to_string(shard).unwrap(), original);

        let relative_shard = gwt_core::paths::gwt_work_event_shard_path(
            Path::new(".gwt/work/events"),
            "event-future",
        );
        let divergent = original.replace("true", "false");
        let (_temp, worktree, target) =
            repoint_fixture(&[(legacy, ""), (relative_shard.to_str().unwrap(), &divergent)]);
        seed_repoint_collision(&worktree, legacy, original);
        let error = super::with_pm_repoint_transaction::<()>(&worktree, &target, || {
            panic!("must reject divergent incoming event")
        })
        .unwrap_err();
        assert!(error.to_string().contains("Work history"), "{error}");
        assert_eq!(
            std::fs::read_to_string(worktree.join(legacy)).unwrap(),
            original
        );

        let (_temp, worktree, target) = repoint_fixture(&[(legacy, &divergent)]);
        seed_repoint_collision(&worktree, legacy, original);
        let mut called = false;
        let error = super::with_pm_repoint_transaction(&worktree, &target, || {
            called = true;
            Ok(())
        })
        .expect_err("must reject divergent incoming legacy event before preservation");
        assert!(!called);
        assert!(error.to_string().contains("Work history"), "{error}");
        assert_eq!(
            std::fs::read_to_string(worktree.join(legacy)).unwrap(),
            original
        );
        let canonical = gwt_core::paths::gwt_work_event_shard_path(
            &worktree.join(".gwt/work/events"),
            "event-future",
        );
        assert!(
            !canonical.exists(),
            "divergence must be rejected before publication"
        );

        let flat_shard = Path::new(".gwt/work/events").join(relative_shard.file_name().unwrap());
        let (_temp, worktree, target) =
            repoint_fixture(&[(legacy, ""), (flat_shard.to_str().unwrap(), &divergent)]);
        seed_repoint_collision(&worktree, legacy, original);
        let error = super::with_pm_repoint_transaction::<()>(&worktree, &target, || {
            panic!("must reject divergent incoming flat shard")
        })
        .unwrap_err();
        assert!(error.to_string().contains("Work history"), "{error}");
        assert_eq!(
            std::fs::read_to_string(worktree.join(legacy)).unwrap(),
            original
        );
    }

    #[test]
    fn pm_repoint_transaction_holds_materializer_lock_through_callback() {
        let asset = ".claude/commands/gwt-execute.md";
        let (_temp, worktree, target) = repoint_fixture(&[(asset, "incoming")]);
        seed_repoint_collision(&worktree, asset, "original");
        let (entered, receiver) = std::sync::mpsc::channel();
        let mut worker = None;
        super::with_pm_repoint_transaction(&worktree, &target, || {
            let other = worktree.clone();
            worker = Some(std::thread::spawn(move || {
                super::with_managed_asset_lock(&other, || {
                    entered.send(()).unwrap();
                    Ok(())
                })
                .unwrap();
            }));
            assert!(matches!(
                receiver.recv_timeout(std::time::Duration::from_millis(100)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ));
            assert!(!worktree.join(asset).exists());
            Ok(())
        })
        .unwrap();
        receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        worker.unwrap().join().unwrap();
    }

    #[test]
    fn pm_repoint_transaction_rejects_work_shard_with_mismatched_event_id() {
        let shard = gwt_core::paths::gwt_work_event_shard_path(
            Path::new(".gwt/work/events"),
            "expected-id",
        );
        let relative = shard.to_str().unwrap();
        let (_temp, worktree, target) = repoint_fixture(&[(relative, "")]);
        let original = "{\"id\":\"different-id\",\"work_item_id\":\"work-future\",\"kind\":\"future-kind\",\"updated_at\":\"2026-09-10T00:00:00Z\"}\n";
        seed_repoint_collision(&worktree, relative, original);
        let error = super::with_pm_repoint_transaction::<()>(&worktree, &target, || {
            panic!("must reject mismatched shard identity")
        })
        .unwrap_err();
        assert!(error.to_string().contains("Work history"), "{error}");
        assert_eq!(
            std::fs::read_to_string(worktree.join(relative)).unwrap(),
            original
        );
    }

    #[cfg(unix)]
    #[test]
    fn pm_repoint_transaction_rejects_symlinked_managed_parent_and_incoming_asset() {
        let asset = ".codex/skills/gwt-execute/SKILL.md";
        let (temp, worktree, target) = repoint_fixture(&[(asset, "incoming")]);
        let outside = temp.path().join("outside");
        seed_repoint_collision(
            &outside,
            "skills/gwt-execute/SKILL.md",
            "outside user bytes",
        );
        std::os::unix::fs::symlink(&outside, worktree.join(".codex")).unwrap();
        let error = super::with_pm_repoint_transaction::<()>(&worktree, &target, || {
            panic!("must reject symlink")
        })
        .unwrap_err();
        assert!(error.to_string().contains("user-owned changes"), "{error}");
        assert_eq!(
            std::fs::read_to_string(outside.join("skills/gwt-execute/SKILL.md")).unwrap(),
            "outside user bytes"
        );

        let (_temp, worktree, target) = repoint_fixture(&[(asset, "incoming")]);
        let base = repoint_git(&worktree, &["rev-parse", "HEAD"]);
        repoint_git(&worktree, &["checkout", "--quiet", "--detach", &target]);
        std::fs::remove_file(worktree.join(asset)).unwrap();
        std::os::unix::fs::symlink("outside-file", worktree.join(asset)).unwrap();
        repoint_git(&worktree, &["add", asset]);
        repoint_git(&worktree, &["commit", "--quiet", "-m", "incoming symlink"]);
        let target = repoint_git(&worktree, &["rev-parse", "HEAD"]);
        repoint_git(&worktree, &["checkout", "--quiet", "--detach", &base]);
        seed_repoint_collision(&worktree, asset, "original");
        let error = super::with_pm_repoint_transaction::<()>(&worktree, &target, || {
            panic!("must reject incoming symlink")
        })
        .unwrap_err();
        assert!(error.to_string().contains("user-owned changes"), "{error}");
        assert_eq!(
            std::fs::read_to_string(worktree.join(asset)).unwrap(),
            "original"
        );
    }

    #[test]
    fn process_stable_codex_config_path_uses_os_default_and_absolute_process_home() {
        let dir = tempfile::tempdir().expect("tempdir");
        let os_user_home = dir.path().join("home");
        let explicit_codex_home = dir.path().join("shared-codex");
        std::fs::create_dir_all(&explicit_codex_home).expect("create explicit Codex home");

        assert_eq!(
            super::process_stable_codex_config_path_with(None, Some(&os_user_home)),
            Some(os_user_home.join(".codex/config.toml"))
        );
        assert_eq!(
            super::process_stable_codex_config_path_with(
                Some(explicit_codex_home.as_os_str()),
                Some(&os_user_home),
            ),
            Some(
                dunce::canonicalize(&explicit_codex_home)
                    .expect("canonical explicit Codex home")
                    .join("config.toml")
            )
        );
    }

    #[test]
    fn process_stable_codex_config_path_rejects_relative_process_home() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert_eq!(
            super::process_stable_codex_config_path_with(
                Some(std::ffi::OsStr::new("relative/codex-home")),
                Some(dir.path()),
            ),
            None,
            "a worktree-relative process CODEX_HOME cannot have one shared lifecycle"
        );
    }

    #[test]
    fn cleanup_ignores_a_process_codex_home_inside_the_removed_worktree() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("tempdir");
        let worktree = dir.path().join("worktree");
        let nested_codex_home = worktree.join(".codex");
        std::fs::create_dir_all(&nested_codex_home).expect("create nested Codex home");
        std::fs::write(nested_codex_home.join("config.toml"), "projects = [\n")
            .expect("write malformed nested config");
        let _codex_home =
            gwt_core::test_support::ScopedEnvVar::set("CODEX_HOME", &nested_codex_home);

        super::cleanup_worktree_with_codex_project_trust(&worktree, || {
            std::fs::remove_dir_all(&worktree)
        })
        .expect("nested process CODEX_HOME is outside the shared trust lifecycle");

        assert!(!worktree.exists());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_detects_an_uncreated_nested_codex_home_through_a_symlink_alias() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("tempdir");
        let real_parent = dir.path().join("real");
        let worktree = real_parent.join("worktree");
        std::fs::create_dir_all(&worktree).expect("create real worktree");
        let alias_parent = dir.path().join("alias");
        std::os::unix::fs::symlink(&real_parent, &alias_parent).expect("create parent alias");
        let aliased_codex_home = alias_parent.join("worktree/.codex");
        let _codex_home =
            gwt_core::test_support::ScopedEnvVar::set("CODEX_HOME", &aliased_codex_home);

        super::cleanup_worktree_with_codex_project_trust(&worktree, || {
            if worktree.join(".codex").exists() {
                return Err(std::io::Error::other(
                    "policy created a config lock inside the deletion target",
                ));
            }
            std::fs::remove_dir_all(&worktree)
        })
        .expect("an aliased uncreated nested Codex home is not shared lifecycle state");

        assert!(!worktree.exists());
    }

    #[test]
    fn materialize_into_missing_worktree_fails_with_clear_attribution() {
        // #fix: when the launch's worktree was never created (branch/worktree
        // materialization failed), managed-asset materialization must fail fast
        // with a clear, attributed error — NOT the misleading downstream
        // "failed to generate Claude coordination skill: No such file or
        // directory" that points at the skill writer instead of the worktree.
        let missing = std::env::temp_dir()
            .join(format!("gwt-missing-worktree-{}", std::process::id()))
            .join("issue-3206");
        let err = super::refresh_managed_gwt_assets_for_worktree(&missing)
            .expect_err("a missing worktree must error");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        let msg = err.to_string();
        assert!(
            msg.contains("worktree is not a ready directory"),
            "error must attribute to the worktree, got: {msg}"
        );
        assert!(
            msg.contains("issue-3206"),
            "error must name the failing worktree path, got: {msg}"
        );
    }

    // SPEC #3245 FR-007: the (re-)materialization asset policy is decided by
    // the structural worktree form (`.intake*` naming), never by ambient env.
    #[test]
    fn worktree_is_ephemeral_is_path_based() {
        let persistent = tempfile::tempdir().expect("worktree");
        assert!(!super::worktree_is_ephemeral(persistent.path()));
        let root = tempfile::tempdir().expect("root");
        let ephemeral = root.path().join(".intake");
        std::fs::create_dir_all(&ephemeral).expect("mk ephemeral");
        assert!(super::worktree_is_ephemeral(&ephemeral));
        let suffixed = root.path().join(".intake-2");
        std::fs::create_dir_all(&suffixed).expect("mk suffixed");
        assert!(super::worktree_is_ephemeral(&suffixed));
    }

    #[test]
    fn bunx_temp_current_exe_prefers_stable_path_gwtd() {
        let current_exe = Path::new(
            r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let stable = PathBuf::from(r"C:\Users\Example\.bun\bin\gwtd.exe");

        let resolved = resolve_public_gwt_bin_with_candidates(
            current_exe,
            Vec::new(),
            |command| {
                assert_eq!(command, "gwtd");
                Some(stable.clone())
            },
            |_| false,
        );

        assert_eq!(resolved, stable);
    }

    #[test]
    fn stable_gwtd_current_exe_is_kept_without_path_lookup() {
        let current_exe = Path::new(r"C:\Users\Example\.bun\bin\gwtd.exe");

        let resolved = resolve_public_gwt_bin_with_candidates(
            current_exe,
            Vec::new(),
            |_command| panic!("stable gwtd binary should not hit PATH lookup"),
            |candidate| candidate == current_exe,
        );

        assert_eq!(resolved, current_exe);
    }

    #[test]
    fn worktree_local_gwtd_current_exe_is_rejected_in_favor_of_stable_path() {
        let current_exe = Path::new("/repo/work/issue-3398/target/debug/gwtd");
        let stable = PathBuf::from("/usr/local/bin/gwtd");

        let resolved = resolve_public_gwt_bin_with_candidates(
            current_exe,
            Vec::new(),
            |command| {
                assert_eq!(command, "gwtd");
                Some(stable.clone())
            },
            |_| false,
        );

        assert_eq!(resolved, stable);
    }

    #[test]
    fn windows_cross_target_gwtd_is_rejected_in_favor_of_stable_path() {
        let current_exe =
            Path::new(r"C:\repo\work\issue-3398\target\x86_64-pc-windows-msvc\release\gwtd.exe");
        let stable = PathBuf::from(r"C:\Program Files\GWT\gwtd.exe");

        let resolved = resolve_public_gwt_bin_with_candidates(
            current_exe,
            Vec::new(),
            |_command| Some(stable.clone()),
            |_| false,
        );

        assert_eq!(resolved, stable);
    }

    #[test]
    fn installed_stable_candidate_wins_over_path() {
        let current_exe = Path::new("/repo/target/debug/gwtd");
        let installed = PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd");
        let path = PathBuf::from("/usr/local/bin/gwtd");

        let resolved = resolve_public_gwt_bin_with_candidates(
            current_exe,
            vec![installed.clone()],
            |_command| Some(path),
            |candidate| candidate == installed,
        );

        assert_eq!(resolved, installed);
    }

    #[test]
    fn installed_app_candidate_wins_over_stable_current_executable() {
        let current_exe = Path::new("/usr/local/bin/gwtd");
        let installed = PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd");

        let resolved = resolve_public_gwt_bin_with_candidates(
            current_exe,
            vec![installed.clone()],
            |_command| None,
            |candidate| candidate == installed || candidate == current_exe,
        );

        assert_eq!(resolved, installed);
    }

    #[test]
    fn bunx_temp_candidates_are_never_persisted() {
        let current_exe = Path::new(
            r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let path_candidate = PathBuf::from(
            r"C:\Users\Example\AppData\Local\Temp\bunx-2222222222-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwtd.exe",
        );

        let resolved = resolve_public_gwt_bin_with_lookup(current_exe, |_command| {
            Some(path_candidate.clone())
        });

        assert!(
            !is_bunx_temp_executable(&resolved),
            "temporary bunx paths must not be persisted: {}",
            resolved.display()
        );
        assert!(!is_worktree_local_build_binary(&resolved));
    }

    #[test]
    fn gui_front_door_current_exe_prefers_daemon_sibling_when_path_lookup_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_name = if cfg!(windows) { "gwt.exe" } else { "gwt" };
        let daemon_name = if cfg!(windows) { "gwtd.exe" } else { "gwtd" };
        let current_exe = temp.path().join("app").join(executable_name);
        let sibling_daemon = current_exe.with_file_name(daemon_name);
        std::fs::create_dir_all(current_exe.parent().expect("current exe parent"))
            .expect("create current exe parent");
        std::fs::write(&current_exe, b"gwt").expect("write current exe fixture");
        std::fs::write(&sibling_daemon, b"gwtd").expect("write sibling daemon fixture");

        let resolved = resolve_public_gwt_bin_with_candidates(
            &current_exe,
            Vec::new(),
            |_command| None,
            |candidate| candidate == sibling_daemon,
        );

        assert_eq!(resolved, sibling_daemon);
    }

    #[test]
    fn stable_gui_front_door_falls_back_to_path_when_daemon_sibling_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_name = if cfg!(windows) { "gwt.exe" } else { "gwt" };
        let daemon_name = if cfg!(windows) { "gwtd.exe" } else { "gwtd" };
        let current_exe = temp.path().join("app").join(executable_name);
        let path_daemon = temp.path().join("path-bin").join(daemon_name);
        std::fs::create_dir_all(current_exe.parent().expect("current exe parent"))
            .expect("create current exe parent");
        std::fs::create_dir_all(path_daemon.parent().expect("PATH daemon parent"))
            .expect("create PATH daemon parent");
        std::fs::write(&current_exe, b"gwt").expect("write current exe fixture");
        std::fs::write(&path_daemon, b"gwtd").expect("write PATH daemon fixture");

        let resolved = resolve_public_gwt_bin_with_candidates(
            &current_exe,
            Vec::new(),
            |command| {
                assert_eq!(command, "gwtd");
                Some(path_daemon.clone())
            },
            |candidate| candidate == path_daemon,
        );

        assert_eq!(resolved, path_daemon);
    }

    #[test]
    fn stable_gui_front_door_prefers_matching_daemon_sibling_over_foreign_path_install() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_name = if cfg!(windows) { "gwt.exe" } else { "gwt" };
        let daemon_name = if cfg!(windows) { "gwtd.exe" } else { "gwtd" };
        let current_exe = temp.path().join("app").join(executable_name);
        let sibling_daemon = current_exe.with_file_name(daemon_name);
        let foreign_install = temp.path().join("foreign").join(daemon_name);
        std::fs::create_dir_all(current_exe.parent().expect("current exe parent"))
            .expect("create current exe parent");
        std::fs::create_dir_all(foreign_install.parent().expect("foreign daemon parent"))
            .expect("create foreign daemon parent");
        std::fs::write(&current_exe, b"gwt").expect("write current exe fixture");
        std::fs::write(&sibling_daemon, b"gwtd").expect("write sibling daemon fixture");
        std::fs::write(&foreign_install, b"foreign gwtd").expect("write foreign daemon fixture");

        let resolved = resolve_public_gwt_bin_with_candidates(
            &current_exe,
            Vec::new(),
            |command| {
                assert_eq!(command, "gwtd");
                Some(foreign_install.clone())
            },
            |candidate| candidate == sibling_daemon || candidate == foreign_install,
        );

        assert_eq!(resolved, sibling_daemon);
    }

    #[test]
    fn path_helpers_identify_named_binaries_and_temp_layouts() {
        let stable = Path::new(r"C:\Users\Example\.bun\bin\gwt.exe");
        let stable_upper = Path::new(r"C:\Users\Example\.bun\bin\gwt.EXE");
        let daemon_upper = Path::new(r"C:\Program Files\GWT\GWTD.EXE");
        let bunx = Path::new(
            r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let other = Path::new(r"C:\Users\Example\.bun\bin\other.exe");

        assert!(is_named_gwt_binary(stable));
        assert!(is_named_gwt_binary(stable_upper));
        assert!(is_named_gwtd_binary(daemon_upper));
        assert!(!is_named_gwt_binary(other));
        assert!(is_bunx_temp_executable(bunx));
        assert!(!is_bunx_temp_executable(stable));
        assert_eq!(
            normalized_path_segments(Path::new(r"C:\Users\Example\.bun\bin\gwt.exe"))
                .last()
                .map(String::as_str),
            Some("gwt.exe")
        );
        assert!(!is_worktree_local_build_binary(stable));
        assert!(!is_worktree_local_build_binary(bunx));
        assert!(!is_worktree_local_build_binary(other));
        assert!(is_worktree_local_build_binary(Path::new(
            r"C:\repo\target\debug\gwtd.exe"
        )));
    }

    #[test]
    fn same_path_and_env_var_guard_preserve_previous_values() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        std::fs::create_dir_all(&nested).expect("create nested");

        assert!(same_path(&nested, &dir.path().join("nested")));

        std::env::set_var("GWT_MANAGED_ASSETS_TEST", "before");
        {
            let _scoped = EnvVarGuard::set("GWT_MANAGED_ASSETS_TEST", "during");
            assert_eq!(
                std::env::var("GWT_MANAGED_ASSETS_TEST").as_deref(),
                Ok("during")
            );
        }
        assert_eq!(
            std::env::var("GWT_MANAGED_ASSETS_TEST").as_deref(),
            Ok("before")
        );

        {
            let _noop = EnvVarGuard::noop("GWT_MANAGED_ASSETS_TEST");
            assert_eq!(
                std::env::var("GWT_MANAGED_ASSETS_TEST").as_deref(),
                Ok("before")
            );
        }
        std::env::remove_var("GWT_MANAGED_ASSETS_TEST");
    }
}
