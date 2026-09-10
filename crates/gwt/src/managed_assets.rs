use std::{
    fs, io,
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
    if !crate::pm_registry::is_pm_worktree(worktree) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a canonical PM worktree: {}", worktree.display()),
        ));
    }
    with_managed_asset_lock(worktree, || {
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
    })
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

pub fn refresh_managed_gwt_assets_for_agent(worktree: &Path, agent_id: &AgentId) -> io::Result<()> {
    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
        worktree,
        agent_id,
        MANAGED_CODEX_HOOK_DISCOVERY_MODE,
        worktree_is_ephemeral(worktree),
    )
}

pub fn refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
    worktree: &Path,
    agent_id: &AgentId,
    codex_hook_discovery_mode: CodexHookDiscoveryMode,
    is_ephemeral: bool,
) -> io::Result<()> {
    with_managed_asset_lock(worktree, || {
        let targets = managed_targets_for_agent(agent_id)
            .into_iter()
            .collect::<Vec<_>>();
        materialize_managed_gwt_assets_for_targets(
            worktree,
            &targets,
            codex_hook_discovery_mode,
            is_ephemeral,
        )?;
        let exclude_targets = detect_existing_managed_asset_targets(worktree);
        update_git_exclude_for_targets(worktree, &exclude_targets).map_err(|error| {
            io::Error::other(format!("failed to update gwt managed excludes: {error}"))
        })?;
        Ok(())
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

fn materialize_managed_gwt_assets_for_targets(
    worktree: &Path,
    targets: &[ManagedAssetTarget],
    codex_hook_discovery_mode: CodexHookDiscoveryMode,
    is_ephemeral: bool,
) -> io::Result<()> {
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
        return Ok(());
    }
    // SPEC-3431 T-052: gwt-pm is generated, not bundled, so the prune above
    // deletes it like any other unknown `gwt-*` skill. Regenerating here — the
    // one funnel every launch, resume, and refresh passes through — is what
    // makes the `$gwt-pm` bootstrap prompt resolvable at all. The predicate is
    // structural (canonical PM worktree path), so no other worktree can be
    // handed the PM contract by an ambient value.
    let is_pm = crate::pm_registry::is_pm_worktree(worktree);
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
    Ok(())
}

pub fn regenerate_existing_managed_hook_configs(worktree: &Path) -> io::Result<()> {
    with_managed_asset_lock(worktree, || {
        let targets = detect_existing_managed_asset_targets(worktree);
        regenerate_managed_hook_configs_for_targets(
            worktree,
            &targets,
            MANAGED_CODEX_HOOK_DISCOVERY_MODE,
        )
    })
}

fn regenerate_managed_hook_configs_for_targets(
    worktree: &Path,
    targets: &[ManagedAssetTarget],
    codex_hook_discovery_mode: CodexHookDiscoveryMode,
) -> io::Result<()> {
    if targets.is_empty() {
        return Ok(());
    }
    let _hook_bin_guard = install_hook_bin_override()?;
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
    Ok(())
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

fn install_hook_bin_override() -> io::Result<EnvVarGuard> {
    if std::env::var_os("GWT_HOOK_BIN").is_some_and(|value| !value.is_empty()) {
        return Ok(EnvVarGuard::noop("GWT_HOOK_BIN"));
    }
    let hook_bin = resolve_public_gwt_bin_path()?;
    Ok(EnvVarGuard::set("GWT_HOOK_BIN", hook_bin))
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

pub(crate) fn is_worktree_local_build_binary(path: &Path) -> bool {
    let segments = normalized_path_segments(path)
        .into_iter()
        .map(|segment| segment.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let Some(file_name) = segments.last() else {
        return false;
    };
    let binary_name = strip_windows_exe_suffix(file_name);
    if binary_name != GUI_FRONT_DOOR_BINARY_NAME && binary_name != INTERNAL_DAEMON_BINARY_NAME {
        return false;
    }

    segments.iter().enumerate().any(|(index, segment)| {
        segment == "target"
            && segments[index + 1..segments.len().saturating_sub(1)]
                .iter()
                .any(|segment| matches!(segment.as_str(), "debug" | "release"))
    })
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
