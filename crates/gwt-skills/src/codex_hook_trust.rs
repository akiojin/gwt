//! Codex hook trust-state registration for gwt-managed project hooks.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use fs2::FileExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::settings_local::{
    codex_event_hook_commands, codex_event_hook_commands_with_bin,
    codex_hooks_paths_for_codex_discovery, codex_self_improvement_stop_hook_commands,
    write_text_atomically, CodexHookDiscoveryMode,
};

const CODEX_DEFAULT_COMMAND_TIMEOUT_SECONDS: u64 = 600;
/// Distinctive substrings that mark a command as a dispatch into gwt's own hook
/// transports. Recognising the transport says only "gwt owns this hook", never
/// "this hook is safe" — trust still requires an exact match against a command
/// gwt emits.
const GWT_HOOK_TRANSPORT_MARKERS: &[&str] = &[" hook event ", " hook gwt-self-improvement-stop"];
/// How long a mutation waits for another writer to finish before giving up.
/// Sized for a burst of concurrent launches against a large shared config, not
/// for a wedged holder: past it, failing with the lock path beats hanging the
/// launch forever.
const CODEX_CONFIG_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const CODEX_CONFIG_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(25);
const MANAGED_EVENTS: &[(&str, &str)] = &[
    ("SessionStart", "session_start"),
    ("UserPromptSubmit", "user_prompt_submit"),
    ("PreToolUse", "pre_tool_use"),
    ("PostToolUse", "post_tool_use"),
    ("Stop", "stop"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexHookTrustEntry {
    pub key: String,
    pub trusted_hash: String,
}

/// Issue #4071 AC-2: what the scan compared one hooks file against. A
/// `Hooks need review` refusal quotes it so the reader can tell a command /
/// trusted_hash mismatch from a registration that never ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexHookTrustExpectation {
    /// The hooks file in the same form the trust keys use.
    pub hooks_path: PathBuf,
    /// The fallback binary the generator was allowed to write into this file:
    /// [`crate::CANONICAL_HOOK_BIN`] for a git-tracked config, the absolute
    /// install path otherwise (#3567).
    pub expected_gwt_bin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexHookTrustReport {
    pub config_path: PathBuf,
    pub trusted_entries: Vec<CodexHookTrustEntry>,
    /// Issue #3967 AC-4: gwt hooks the pre-registration could not vouch for,
    /// as `<hooks.json>:<event>:<group>:<handler>`. Each one is a Codex
    /// `Hooks need review` prompt waiting to happen, so the caller must fail
    /// loudly instead of launching an unattended agent into it. User hooks are
    /// never listed here — reviewing those is the user's own business.
    pub untrusted_gwt_hooks: Vec<String>,
    /// Issue #4071 AC-2: one entry per hooks file the scan read.
    pub expectations: Vec<CodexHookTrustExpectation>,
    /// Issue #3967: the command string behind each entry of
    /// [`Self::untrusted_gwt_hooks`], in the same order. The expected fallback
    /// binary alone did not say what the file actually held, so diagnosing a
    /// recurrence meant reading the machine's `.codex/hooks.json` by hand.
    pub untrusted_gwt_hook_commands: Vec<String>,
    /// Issue #4071 AC-2: whether `config_path` was written. False when no gwt
    /// hook could be vouched for — gwt 9.91.0 then left the config untouched
    /// and the failure read as if registration had never run.
    pub wrote_trust_state: bool,
}

impl CodexHookTrustReport {
    /// Issue #4071 AC-2: the launch-blocking reason for the gwt hooks Codex
    /// would still ask a human about, or `None` when every gwt hook is
    /// trusted. It states whether any trust state was written and which
    /// fallback binary each hooks file was compared against, so a skipped
    /// registration and a command / path-form mismatch read differently in
    /// the failure record.
    pub fn hooks_need_review_reason(&self) -> Option<String> {
        if self.untrusted_gwt_hooks.is_empty() {
            return None;
        }
        let mut reason = format!(
            "Codex hook trust is incomplete: Codex would stop this launch on `Hooks need review` for {}. ",
            self.untrusted_gwt_hooks.join(", ")
        );
        if self.wrote_trust_state {
            reason.push_str(&format!(
                "gwt wrote {} trusted entries to {}; the listed hooks were skipped because their command does not match what gwt generates (trusted_hash mismatch, not a missing registration). ",
                self.trusted_entries.len(),
                self.config_path.display()
            ));
        } else {
            reason.push_str(&format!(
                "gwt wrote no trust entry to {}: none of the gwt hooks matched what gwt generates, so registration was skipped. ",
                self.config_path.display()
            ));
        }
        let expectations = self
            .expectations
            .iter()
            .map(|expectation| {
                format!(
                    "{} => `{}`",
                    expectation.hooks_path.display(),
                    expectation.expected_gwt_bin
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        reason.push_str(&format!(
            "Expected fallback binary per hooks file: {expectations}."
        ));
        // Issue #3967: quote what the file actually holds. Without it a
        // recurrence only says which hooks Codex would stop on, and telling a
        // stale command apart from a mismatched binary needs the machine.
        if let Some(command) = self.untrusted_gwt_hook_commands.first() {
            reason.push_str(&format!(" First untrusted command: `{command}`."));
        }
        Some(reason)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexProjectTrustReport {
    pub config_path: PathBuf,
    pub project_path: PathBuf,
}

pub fn collect_codex_managed_hook_trust_entries(
    worktree: &Path,
) -> io::Result<Vec<CodexHookTrustEntry>> {
    collect_codex_managed_hook_trust_entries_for_mode(
        worktree,
        CodexHookDiscoveryMode::WorkspaceHome,
    )
}

pub fn collect_codex_managed_hook_trust_entries_for_mode(
    worktree: &Path,
    mode: CodexHookDiscoveryMode,
) -> io::Result<Vec<CodexHookTrustEntry>> {
    collect_codex_managed_hook_trust_entries_for_mode_with_expected_bin(worktree, mode, None)
}

#[cfg(test)]
fn collect_codex_managed_hook_trust_entries_with_expected_bin(
    worktree: &Path,
    expected_gwt_bin: Option<&str>,
) -> io::Result<Vec<CodexHookTrustEntry>> {
    collect_codex_managed_hook_trust_entries_for_mode_with_expected_bin(
        worktree,
        CodexHookDiscoveryMode::WorkspaceHome,
        expected_gwt_bin,
    )
}

fn collect_codex_managed_hook_trust_entries_for_mode_with_expected_bin(
    worktree: &Path,
    mode: CodexHookDiscoveryMode,
    expected_gwt_bin: Option<&str>,
) -> io::Result<Vec<CodexHookTrustEntry>> {
    Ok(scan_codex_hook_trust_for_mode(worktree, mode, expected_gwt_bin)?.trusted)
}

/// Everything one scan of the discovered `.codex/hooks.json` files learned:
/// the hooks gwt can vouch for, and the gwt hooks it could not.
#[derive(Debug, Default)]
struct CodexHookTrustScan {
    trusted: Vec<CodexHookTrustEntry>,
    untrusted_gwt_hooks: Vec<String>,
    untrusted_gwt_hook_commands: Vec<String>,
    expectations: Vec<CodexHookTrustExpectation>,
}

fn scan_codex_hook_trust_for_mode(
    worktree: &Path,
    mode: CodexHookDiscoveryMode,
    expected_gwt_bin: Option<&str>,
) -> io::Result<CodexHookTrustScan> {
    let mut scan = CodexHookTrustScan::default();
    for hooks_path in codex_hooks_paths_for_codex_discovery(worktree, mode) {
        let path_scan = scan_codex_hook_trust_from_path(&hooks_path, expected_gwt_bin)?;
        scan.trusted.extend(path_scan.trusted);
        scan.untrusted_gwt_hooks
            .extend(path_scan.untrusted_gwt_hooks);
        scan.untrusted_gwt_hook_commands
            .extend(path_scan.untrusted_gwt_hook_commands);
        scan.expectations.extend(path_scan.expectations);
    }
    Ok(scan)
}

/// Walk every hook group and every handler in the file. Codex keys its trust
/// state by `<path>:<event>:<group index>:<handler index>`, so a gwt hook that
/// does not sit at `0:0` — the repo-owned `gwt-self-improvement-stop` Stop hook
/// is appended after the managed group — is a hook Codex still asks a human
/// about (Issue #3967 AC-1).
fn scan_codex_hook_trust_from_path(
    hooks_path: &Path,
    expected_gwt_bin: Option<&str>,
) -> io::Result<CodexHookTrustScan> {
    if !hooks_path.exists() {
        return Ok(CodexHookTrustScan::default());
    }

    // #3567: what the generator was allowed to write into THIS file is not
    // always the binary the caller resolved — a git-tracked config keeps the
    // canonical portable fallback, and a foreign checkout's build output is
    // never pinned here. Trust has to expect the same value, or gwt vouches for
    // nothing and Codex stops the launch on `Hooks need review`.
    let sanitized_expected_gwt_bin = expected_gwt_bin.map_or_else(
        || crate::settings_local::managed_hook_bin_for_config_path(hooks_path),
        |bin| crate::settings_local::sanitize_hook_bin_for_config_path(hooks_path, bin),
    );
    let expected_gwt_bin = Some(sanitized_expected_gwt_bin.as_str());

    // Issue #4071: Codex derives this key from the hooks path it discovered,
    // normalized but never canonicalized — on Windows that is the plain
    // `E:\...` form. `std::fs::canonicalize` yields the `\\?\` verbatim
    // form there, and an entry under that key is inert: Codex still stops on
    // `Hooks need review`. `dunce` strips the prefix and is a no-op elsewhere.
    let key_source = dunce::canonicalize(hooks_path)?;
    let content = fs::read_to_string(hooks_path)?;
    let root: Value = serde_json::from_str(&content).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Codex hooks JSON parse failed: {err}"),
        )
    })?;

    let Some(hooks_by_event) = root.get("hooks").and_then(Value::as_object) else {
        return Ok(CodexHookTrustScan::default());
    };

    let mut scan = CodexHookTrustScan {
        expectations: vec![CodexHookTrustExpectation {
            hooks_path: key_source.clone(),
            expected_gwt_bin: sanitized_expected_gwt_bin.clone(),
        }],
        ..CodexHookTrustScan::default()
    };
    for (event_json_name, event_snake_name) in MANAGED_EVENTS {
        let Some(groups) = hooks_by_event
            .get(*event_json_name)
            .and_then(Value::as_array)
        else {
            continue;
        };
        for (group_index, group) in groups.iter().enumerate() {
            let Some(group) = group.as_object() else {
                continue;
            };
            let matcher = group
                .get("matcher")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(hooks) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for (handler_index, hook) in hooks.iter().enumerate() {
                let Some(hook) = hook.as_object() else {
                    continue;
                };
                let Some(command) = hook.get("command").and_then(Value::as_str) else {
                    continue;
                };
                if hook.get("type").and_then(Value::as_str) != Some("command") {
                    continue;
                }
                let key = hook_key(&key_source, event_snake_name, group_index, handler_index);
                if matcher == "*"
                    && is_trusted_gwt_hook_command(
                        command,
                        event_json_name,
                        event_snake_name,
                        expected_gwt_bin,
                    )
                {
                    scan.trusted.push(CodexHookTrustEntry {
                        key,
                        trusted_hash: command_hook_trusted_hash(event_snake_name, matcher, command),
                    });
                } else if is_gwt_hook_transport_command(command) {
                    scan.untrusted_gwt_hooks.push(key);
                    scan.untrusted_gwt_hook_commands.push(command.to_string());
                }
            }
        }
    }

    Ok(scan)
}

pub fn register_codex_managed_hook_trust(
    worktree: &Path,
    config_path: &Path,
) -> io::Result<CodexHookTrustReport> {
    register_codex_managed_hook_trust_for_mode(
        worktree,
        config_path,
        CodexHookDiscoveryMode::WorkspaceHome,
    )
}

pub fn register_codex_managed_hook_trust_for_mode(
    worktree: &Path,
    config_path: &Path,
    mode: CodexHookDiscoveryMode,
) -> io::Result<CodexHookTrustReport> {
    register_codex_managed_hook_trust_for_mode_with_expected_bin(worktree, config_path, mode, None)
}

/// Register trust for the hooks materialization just generated, told exactly
/// which fallback binary it wrote.
///
/// Issue #3967 (recurrence in v9.93.1): the binary a generated hook command
/// falls back to is resolved once, by materialization, and pinned only for the
/// duration of that materialization. Re-deriving it here instead answers with
/// this library's own `current_exe` fallback, and for a gwt started from a
/// checkout build (`target/debug/gwt`) that is the build output — where
/// materialization had written the installed absolute path. Every managed hook
/// then fails the exact-command match, lands in `untrusted_gwt_hooks`, and
/// Codex stops the launch on `Hooks need review`. Callers that generated the
/// hooks must pass the value they generated them with; `None` keeps the
/// library fallback for callers that did not.
pub fn register_codex_managed_hook_trust_for_mode_with_expected_bin(
    worktree: &Path,
    config_path: &Path,
    mode: CodexHookDiscoveryMode,
    expected_gwt_bin: Option<&str>,
) -> io::Result<CodexHookTrustReport> {
    let CodexHookTrustScan {
        trusted: trusted_entries,
        untrusted_gwt_hooks,
        untrusted_gwt_hook_commands,
        expectations,
    } = scan_codex_hook_trust_for_mode(worktree, mode, expected_gwt_bin)?;
    if trusted_entries.is_empty() {
        return Ok(CodexHookTrustReport {
            config_path: config_path.to_path_buf(),
            trusted_entries,
            untrusted_gwt_hooks,
            untrusted_gwt_hook_commands,
            expectations,
            wrote_trust_state: false,
        });
    }

    // Issue #4071: read, mutate and publish as one critical section. A
    // concurrent launch that reads between our read and our write would
    // otherwise write back a copy without our entries.
    with_codex_config_lock(config_path, || {
        let mut root = read_codex_config(config_path)?;
        let root_table = root.as_table_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Codex config root must be a TOML table",
            )
        })?;
        let hooks_table = ensure_child_table(root_table, "hooks")?;
        let state_table = ensure_child_table(hooks_table, "state")?;

        for entry in &trusted_entries {
            let hook_state = ensure_child_table(state_table, &entry.key)?;
            enable_hook_unless_explicitly_disabled(hook_state);
            hook_state.insert(
                "trusted_hash".to_string(),
                toml::Value::String(entry.trusted_hash.clone()),
            );
        }

        let rendered = toml::to_string_pretty(&root).map_err(|err| {
            io::Error::other(format!("Codex config TOML serialize failed: {err}"))
        })?;
        write_text_atomically(config_path, &rendered)
    })?;

    Ok(CodexHookTrustReport {
        config_path: config_path.to_path_buf(),
        trusted_entries,
        untrusted_gwt_hooks,
        untrusted_gwt_hook_commands,
        expectations,
        wrote_trust_state: true,
    })
}

pub fn register_codex_managed_project_trust(
    worktree: &Path,
    config_path: &Path,
) -> io::Result<CodexProjectTrustReport> {
    register_codex_managed_project_trust_with_writer(worktree, config_path, write_text_atomically)
}

/// Remove only the `trusted` value gwt owns for one managed worktree.
///
/// This is deliberately narrower than registration: explicit `untrusted` and
/// future string values belong to the user and remain byte-for-byte untouched.
/// Existing worktrees are canonicalized. A missing absolute path is accepted
/// for prune recovery so stale trust can still be removed after an external
/// filesystem deletion.
pub fn revoke_codex_managed_project_trust(
    worktree: &Path,
    config_path: &Path,
) -> io::Result<CodexProjectTrustReport> {
    revoke_codex_managed_project_trust_with_writer(worktree, config_path, write_text_atomically)
}

/// Revoke project trust and complete the caller's filesystem cleanup while
/// holding the same config-scoped lock used by project and hook registration.
///
/// Keeping the lock through `cleanup` prevents a registrar that observed the
/// path before deletion from publishing `trusted` after the path disappears.
/// A cleanup error is returned after the trust write, leaving an existing
/// worktree untrusted so a later managed launch can safely register it again.
pub fn revoke_codex_managed_project_trust_with_cleanup<T>(
    worktree: &Path,
    config_path: &Path,
    cleanup: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    with_codex_config_lock(config_path, || {
        let project_path = codex_project_path_for_revocation(worktree)?;
        revoke_codex_managed_project_trust_under_lock(
            &project_path,
            config_path,
            write_text_atomically,
        )?;
        cleanup()
    })
}

fn revoke_codex_managed_project_trust_with_writer(
    worktree: &Path,
    config_path: &Path,
    write_config: impl FnOnce(&Path, &str) -> io::Result<()>,
) -> io::Result<CodexProjectTrustReport> {
    with_codex_config_lock(config_path, || {
        let project_path = codex_project_path_for_revocation(worktree)?;
        revoke_codex_managed_project_trust_under_lock(&project_path, config_path, write_config)?;
        Ok(CodexProjectTrustReport {
            config_path: config_path.to_path_buf(),
            project_path,
        })
    })
}

fn codex_project_path_for_revocation(worktree: &Path) -> io::Result<PathBuf> {
    match fs::canonicalize(worktree) {
        Ok(canonical) => Ok(gwt_core::paths::normalize_windows_child_process_path(
            &canonical,
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound && worktree.is_absolute() => Ok(
            gwt_core::paths::normalize_windows_child_process_path(worktree),
        ),
        Err(error) => Err(error),
    }
}

fn revoke_codex_managed_project_trust_under_lock(
    project_path: &Path,
    config_path: &Path,
    write_config: impl FnOnce(&Path, &str) -> io::Result<()>,
) -> io::Result<()> {
    let mut root = read_codex_config(config_path)?;
    let root_table = root.as_table_mut().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Codex config root must be a TOML table",
        )
    })?;
    if revoke_codex_project_trust_level(root_table, project_path)? == 0 {
        return Ok(());
    }

    let rendered = toml::to_string_pretty(&root)
        .map_err(|err| io::Error::other(format!("Codex config TOML serialize failed: {err}")))?;
    write_config(config_path, &rendered)
}

fn register_codex_managed_project_trust_with_writer(
    worktree: &Path,
    config_path: &Path,
    write_config: impl FnOnce(&Path, &str) -> io::Result<()>,
) -> io::Result<CodexProjectTrustReport> {
    let project_path = update_codex_config_with_writer(
        config_path,
        |root_table| {
            let canonical_worktree = fs::canonicalize(worktree)?;
            let project_path =
                gwt_core::paths::normalize_windows_child_process_path(&canonical_worktree);
            ensure_codex_project_trust_level(root_table, &project_path)?;
            Ok(project_path)
        },
        write_config,
    )?;

    Ok(CodexProjectTrustReport {
        config_path: config_path.to_path_buf(),
        project_path,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingCodexProjectTrust {
    Missing,
    Trusted,
}

fn ensure_codex_project_trust_level(
    root_table: &mut toml::Table,
    project_path: &Path,
) -> io::Result<()> {
    let project_key = project_path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "gwt-managed Codex worktree path is not valid UTF-8: {}",
                project_path.display()
            ),
        )
    })?;

    let projects = match root_table.get("projects") {
        Some(toml::Value::Table(projects)) => Some(projects),
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Codex config key `projects` must be a TOML table",
            ));
        }
        None => None,
    };
    let mut equivalent_count = 0usize;
    let mut exact_key_exists = false;
    let mut observed_trust: Option<ExistingCodexProjectTrust> = None;

    if let Some(projects) = projects {
        for (key, value) in projects {
            if !codex_project_keys_equivalent(key, project_key) {
                continue;
            }
            equivalent_count += 1;
            exact_key_exists |= key == project_key;

            let project = value.as_table().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Codex project alias `{key}` must be a TOML table"),
                )
            })?;
            let trust = match project.get("trust_level") {
                Some(toml::Value::String(level)) if level == "untrusted" => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!(
                            "gwt-managed Codex worktree is explicitly untrusted through alias `{key}`"
                        ),
                    ));
                }
                Some(toml::Value::String(level)) if level == "trusted" => {
                    ExistingCodexProjectTrust::Trusted
                }
                Some(toml::Value::String(level)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Codex project trust level for alias `{key}` is unsupported: {level}"
                        ),
                    ));
                }
                Some(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Codex project trust level for alias `{key}` must be a string"),
                    ));
                }
                None => ExistingCodexProjectTrust::Missing,
            };
            if observed_trust.is_some_and(|observed| observed != trust) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Codex project trust has conflicting aliases for {}",
                        project_path.display()
                    ),
                ));
            }
            observed_trust = Some(trust);
        }
    }

    if observed_trust == Some(ExistingCodexProjectTrust::Missing)
        && (!exact_key_exists || equivalent_count > 1)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Codex project trust has conflicting aliases for {}",
                project_path.display()
            ),
        ));
    }

    let projects = ensure_child_table(root_table, "projects")?;
    let project = ensure_child_table(projects, project_key)?;
    project.insert(
        "trust_level".to_string(),
        toml::Value::String("trusted".to_string()),
    );
    Ok(())
}

fn revoke_codex_project_trust_level(
    root_table: &mut toml::Table,
    project_path: &Path,
) -> io::Result<usize> {
    let project_key = project_path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "gwt-managed Codex worktree path is not valid UTF-8: {}",
                project_path.display()
            ),
        )
    })?;
    let Some(projects) = root_table.get_mut("projects") else {
        return Ok(0);
    };
    let projects = projects.as_table_mut().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Codex config key `projects` must be a TOML table",
        )
    })?;

    // Validate every equivalent alias before mutating any of them. A malformed
    // alias therefore fails closed without producing a partly edited document.
    let mut trusted_keys = Vec::new();
    for (key, value) in projects.iter() {
        if !codex_project_keys_equivalent(key, project_key) {
            continue;
        }
        let project = value.as_table().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Codex project alias `{key}` must be a TOML table"),
            )
        })?;
        match project.get("trust_level") {
            Some(toml::Value::String(level)) if level == "trusted" => {
                trusted_keys.push(key.clone());
            }
            Some(toml::Value::String(_)) | None => {}
            Some(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Codex project trust level for alias `{key}` must be a string"),
                ));
            }
        }
    }

    let removed = trusted_keys.len();
    for key in trusted_keys {
        let remove_project = {
            let project = projects
                .get_mut(&key)
                .and_then(toml::Value::as_table_mut)
                .expect("validated Codex project table remains present");
            project.remove("trust_level");
            project.is_empty()
        };
        if remove_project {
            projects.remove(&key);
        }
    }
    Ok(removed)
}

fn codex_project_keys_equivalent(candidate: &str, project_key: &str) -> bool {
    let normalized_candidate =
        gwt_core::paths::normalize_windows_child_process_path_text(candidate);
    if normalized_candidate == project_key {
        return true;
    }

    match (
        normalized_windows_project_identity(candidate),
        normalized_windows_project_identity(project_key),
    ) {
        (Some(candidate), Some(project)) => candidate == project,
        _ => false,
    }
}

fn normalized_windows_project_identity(value: &str) -> Option<String> {
    const POWERSHELL_FILE_SYSTEM_PROVIDER_PREFIX: &str = r"Microsoft.PowerShell.Core\FileSystem::";

    let value = strip_ascii_prefix_ignore_case(value, POWERSHELL_FILE_SYSTEM_PROVIDER_PREFIX)
        .unwrap_or(value);
    let mut normalized = value.replace('\\', "/");
    if let Some(rest) = strip_ascii_prefix_ignore_case(&normalized, "//?/UNC/") {
        normalized = format!("//{rest}");
    } else if let Some(rest) = strip_ascii_prefix_ignore_case(&normalized, "//?/") {
        normalized = rest.to_string();
    }

    let (prefix, remainder, protected_components) =
        if let Some(unc_path) = normalized.strip_prefix("//") {
            let components = unc_path
                .split('/')
                .filter(|component| !component.is_empty())
                .collect::<Vec<_>>();
            if components.len() < 2 {
                return None;
            }
            (
                format!(
                    "//{}/{}",
                    components[0].to_lowercase(),
                    components[1].to_lowercase()
                ),
                components[2..].to_vec(),
                0,
            )
        } else {
            let bytes = normalized.as_bytes();
            if bytes.len() < 3
                || !bytes[0].is_ascii_alphabetic()
                || bytes[1] != b':'
                || bytes[2] != b'/'
            {
                return None;
            }
            (
                normalized[..2].to_ascii_lowercase(),
                normalized[3..]
                    .split('/')
                    .filter(|component| !component.is_empty())
                    .collect::<Vec<_>>(),
                0,
            )
        };

    let mut components = Vec::new();
    for component in remainder {
        match component {
            "." => {}
            ".." if components.len() > protected_components => {
                components.pop();
            }
            ".." => return None,
            component => components.push(component.to_lowercase()),
        }
    }

    if components.is_empty() {
        Some(format!("{prefix}/"))
    } else {
        Some(format!("{prefix}/{}", components.join("/")))
    }
}

fn strip_ascii_prefix_ignore_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = value.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

fn update_codex_config_with_writer<T>(
    config_path: &Path,
    update: impl FnOnce(&mut toml::Table) -> io::Result<T>,
    write_config: impl FnOnce(&Path, &str) -> io::Result<()>,
) -> io::Result<T> {
    with_codex_config_lock(config_path, || {
        let mut root = read_codex_config(config_path)?;
        let root_table = root.as_table_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Codex config root must be a TOML table",
            )
        })?;
        let result = update(root_table)?;

        let rendered = toml::to_string_pretty(&root).map_err(|err| {
            io::Error::other(format!("Codex config TOML serialize failed: {err}"))
        })?;
        write_config(config_path, &rendered)?;
        Ok(result)
    })
}

#[cfg(test)]
fn command_hook_trusted_hash_for_test(
    event_name_snake: &str,
    matcher: &str,
    command: &str,
) -> String {
    command_hook_trusted_hash(event_name_snake, matcher, command)
}

/// Run one read-modify-write of the Codex config under a cross-process lock.
///
/// Issue #4071: `$CODEX_HOME/config.toml` is a single file shared by every
/// Codex agent on the machine — every gwt launch across every project, plus
/// manual `codex` sessions gwt does not manage. Registration reads the whole
/// file, mutates it, and writes it back, so two concurrent launches that both
/// read the pre-write bytes each serialize their own copy and the later writer
/// silently drops the earlier one's entries. That is how a burst of launches
/// left freshly registered worktrees untrusted (Codex then stops on `Hooks
/// need review`) and how 6,444 hand-added `enabled = true` rows disappeared on
/// the next launch.
///
/// The lock lives on a sibling `<config>.gwt-lock` file rather than on the
/// config itself, because the write publishes through `rename` and would
/// otherwise replace the very inode the lock is held on. It is advisory, so it
/// only serializes gwt against gwt; the atomic rename keeps every other reader
/// from ever seeing a half-written file.
pub(crate) fn with_codex_config_lock<T>(
    config_path: &Path,
    body: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = codex_config_lock_path(config_path);
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;

    let deadline = Instant::now() + CODEX_CONFIG_LOCK_TIMEOUT;
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(error) if Instant::now() >= deadline => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "timed out after {}s waiting for the Codex config lock {}: {error}",
                        CODEX_CONFIG_LOCK_TIMEOUT.as_secs(),
                        lock_path.display()
                    ),
                ));
            }
            Err(_) => std::thread::sleep(CODEX_CONFIG_LOCK_POLL_INTERVAL),
        }
    }

    let result = body();
    let _ = FileExt::unlock(&lock);
    result
}

fn codex_config_lock_path(config_path: &Path) -> PathBuf {
    let mut name = config_path
        .file_name()
        .map_or_else(|| OsString::from("config.toml"), OsString::from);
    name.push(".gwt-lock");
    config_path.with_file_name(name)
}

pub(crate) fn read_codex_config(path: &Path) -> io::Result<toml::Value> {
    if !path.exists() {
        return Ok(toml::Value::Table(toml::Table::new()));
    }

    let content = fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(toml::Value::Table(toml::Table::new()));
    }

    toml::from_str::<toml::Value>(&content).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Codex config TOML parse failed: {err}"),
        )
    })
}

pub(crate) fn ensure_child_table<'a>(
    table: &'a mut toml::Table,
    key: &str,
) -> io::Result<&'a mut toml::Table> {
    if !table.contains_key(key) {
        table.insert(key.to_string(), toml::Value::Table(toml::Table::new()));
    }

    table
        .get_mut(key)
        .and_then(toml::Value::as_table_mut)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Codex config key `{key}` must be a TOML table"),
            )
        })
}

fn enable_hook_unless_explicitly_disabled(hook_state: &mut toml::Table) {
    if hook_state.get("enabled").and_then(toml::Value::as_bool) == Some(false) {
        return;
    }
    hook_state.insert("enabled".to_string(), toml::Value::Boolean(true));
}

fn hook_key(
    key_source: &Path,
    event_name: &str,
    group_index: usize,
    handler_index: usize,
) -> String {
    format!(
        "{}:{event_name}:{group_index}:{handler_index}",
        key_source.display()
    )
}

fn command_hook_trusted_hash(event_name_snake: &str, matcher: &str, command: &str) -> String {
    let mut identity = json!({
        "event_name": event_name_snake,
        "hooks": [
            {
                "async": false,
                "command": command,
                "timeout": CODEX_DEFAULT_COMMAND_TIMEOUT_SECONDS,
                "type": "command"
            }
        ]
    });
    if codex_trust_identity_uses_matcher(event_name_snake) {
        identity
            .as_object_mut()
            .expect("Codex hook trust identity must be an object")
            .insert("matcher".to_string(), Value::String(matcher.to_string()));
    }
    sort_json_objects(&mut identity);
    let bytes = serde_json::to_vec(&identity).expect("serialize Codex hook trust identity");
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

fn codex_trust_identity_uses_matcher(event_name_snake: &str) -> bool {
    matches!(
        event_name_snake,
        "pre_tool_use"
            | "permission_request"
            | "post_tool_use"
            | "pre_compact"
            | "post_compact"
            | "session_start"
    )
}

fn sort_json_objects(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                sort_json_objects(item);
            }
        }
        Value::Object(map) => {
            let mut sorted = std::mem::take(map).into_iter().collect::<Vec<_>>();
            sorted.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (key, mut child) in sorted {
                sort_json_objects(&mut child);
                map.insert(key, child);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// A hook gwt may pre-trust on the user's behalf: either a generated managed
/// event command, or the repo-owned `gwt-self-improvement-stop` Stop hook that
/// this repository used to ship as tracked content and that older worktrees
/// still carry. Both are matched against the exact strings gwt itself emits —
/// never by shape — so a tampered command or a swapped binary path is still
/// left for Codex's `/hooks` review.
fn is_trusted_gwt_hook_command(
    command: &str,
    event_json_name: &str,
    event_snake_name: &str,
    expected_gwt_bin: Option<&str>,
) -> bool {
    is_generated_gwt_event_command(command, event_json_name, expected_gwt_bin)
        || (event_snake_name == "stop"
            && codex_self_improvement_stop_hook_commands()
                .iter()
                .any(|expected| expected == command))
}

/// Does this command *dispatch* one of gwt's own hook transports? Used only to
/// decide whether an untrusted hook is gwt's problem (Issue #3967 AC-4) or the
/// user's own hook, which gwt must not vouch for either way.
///
/// The transport marker has to sit where an argument list begins — directly
/// after a token that names the gwt binary. A bare substring test would let a
/// user hook that merely quotes the token (`echo ' hook event '`) abort every
/// Codex launch in the worktree, and this answer gates a launch failure.
/// Tampered invocations (`'/tmp/attacker/gwtd' hook event Stop`) still count:
/// they are gwt hooks that gwt refuses to vouch for, which is exactly what the
/// caller must surface.
fn is_gwt_hook_transport_command(command: &str) -> bool {
    GWT_HOOK_TRANSPORT_MARKERS.iter().any(|marker| {
        command.match_indices(marker).any(|(index, _)| {
            command[..index]
                .split_whitespace()
                .next_back()
                .is_some_and(references_gwt_hook_binary)
        })
    })
}

fn references_gwt_hook_binary(token: &str) -> bool {
    let token = token.trim_matches(|character| matches!(character, '\'' | '"' | '&'));
    token.ends_with("gwtd")
        || token.ends_with("gwtd.exe")
        || token.ends_with("$gwt_bin")
        || token.ends_with("$gwtBin")
}

fn is_generated_gwt_event_command(
    command: &str,
    event_json_name: &str,
    expected_gwt_bin: Option<&str>,
) -> bool {
    expected_generated_gwt_event_commands(event_json_name, expected_gwt_bin)
        .iter()
        .any(|expected| expected == command)
}

fn expected_generated_gwt_event_commands(
    event_json_name: &str,
    expected_gwt_bin: Option<&str>,
) -> Vec<String> {
    expected_gwt_bin.map_or_else(
        || codex_event_hook_commands(event_json_name),
        |bin| codex_event_hook_commands_with_bin(bin, event_json_name),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        sync::mpsc,
        thread,
        time::Duration,
    };

    use fs2::FileExt;
    use serde_json::{json, Value};

    use super::*;
    use crate::{
        generate_codex_hooks, generate_codex_hooks_for_mode,
        settings_local::codex_event_hook_commands_with_bin, CodexHookDiscoveryMode,
    };

    #[test]
    fn command_hook_hash_matches_codex_for_known_post_tool_use_fixture() {
        let command = "'/Applications/GWT.app/Contents/MacOS/gwtd' hook event PostToolUse";

        let trusted_hash = command_hook_trusted_hash_for_test("post_tool_use", "*", command);

        assert_eq!(
            trusted_hash,
            "sha256:9c3ce103f03f0b27a28bc4a30883f7e98a80b5df566b4572fcbb2955ebf5ba62"
        );
    }

    #[test]
    fn command_hook_hash_omits_codex_ignored_matchers_for_prompt_and_stop() {
        let user_prompt_command =
            "gwt_bin=\"${GWT_BIN_PATH:-/Applications/GWT.app/Contents/MacOS/gwtd}\"; \"$gwt_bin\" hook event UserPromptSubmit";
        let stop_command =
            "gwt_bin=\"${GWT_BIN_PATH:-/Applications/GWT.app/Contents/MacOS/gwtd}\"; \"$gwt_bin\" hook event Stop";

        assert_eq!(
            command_hook_trusted_hash_for_test("user_prompt_submit", "*", user_prompt_command),
            "sha256:1a86ba6796c5b5bf1601fd1af1d6094846287ec85e9f1ad4d39335c6b306e2fa"
        );
        assert_eq!(
            command_hook_trusted_hash_for_test("stop", "*", stop_command),
            "sha256:984e12cd30ef54cf4c63af8aabce1849705e5de09c70d039367ba68de9760389"
        );
    }

    #[test]
    fn generated_codex_hooks_produce_five_trust_entries() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dunce::canonicalize(dir.path().join(".codex/hooks.json")).unwrap();

        let entries = collect_codex_managed_hook_trust_entries(dir.path()).unwrap();

        assert_eq!(
            entries.len(),
            5,
            "expected one trust entry per managed event"
        );
        for event_name in [
            "session_start",
            "user_prompt_submit",
            "pre_tool_use",
            "post_tool_use",
            "stop",
        ] {
            let expected_key = format!("{}:{event_name}:0:0", hooks_path.display());
            let entry = entries
                .iter()
                .find(|entry| entry.key == expected_key)
                .unwrap_or_else(|| panic!("missing trust key {expected_key}; got {entries:?}"));
            assert!(
                entry.trusted_hash.starts_with("sha256:"),
                "trusted hash must use Codex sha256 prefix"
            );
        }
    }

    #[test]
    fn managed_project_trust_registers_only_the_exact_canonical_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        let sibling = dir.path().join("sibling-worktree");
        fs::create_dir_all(&worktree).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        let canonical_worktree = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&worktree).unwrap(),
        );
        let canonical_sibling = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&sibling).unwrap(),
        );
        let canonical_parent = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(dir.path()).unwrap(),
        );
        let config_path = dir.path().join("codex/config.toml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        let mut root = toml::Table::new();
        root.insert(
            "model".to_string(),
            toml::Value::String("gpt-5.6-sol".to_string()),
        );
        let mut projects = toml::Table::new();
        let mut managed_config = toml::Table::new();
        managed_config.insert(
            "sandbox_mode".to_string(),
            toml::Value::String("workspace-write".to_string()),
        );
        projects.insert(
            canonical_worktree.to_string_lossy().into_owned(),
            toml::Value::Table(managed_config),
        );
        let mut sibling_config = toml::Table::new();
        sibling_config.insert(
            "trust_level".to_string(),
            toml::Value::String("untrusted".to_string()),
        );
        projects.insert(
            canonical_sibling.to_string_lossy().into_owned(),
            toml::Value::Table(sibling_config),
        );
        root.insert("projects".to_string(), toml::Value::Table(projects));
        fs::write(
            &config_path,
            toml::to_string_pretty(&toml::Value::Table(root)).unwrap(),
        )
        .unwrap();

        let report = register_codex_managed_project_trust(&worktree, &config_path).unwrap();

        assert_eq!(report.project_path, canonical_worktree);
        assert_eq!(report.config_path, config_path);
        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&report.config_path).unwrap()).unwrap();
        assert_eq!(config["model"].as_str(), Some("gpt-5.6-sol"));
        assert_eq!(
            config["projects"][report.project_path.to_string_lossy().as_ref()]["trust_level"]
                .as_str(),
            Some("trusted")
        );
        assert_eq!(
            config["projects"][report.project_path.to_string_lossy().as_ref()]["sandbox_mode"]
                .as_str(),
            Some("workspace-write"),
            "unknown settings on the exact project must survive trust registration"
        );
        assert_eq!(
            config["projects"][canonical_sibling.to_string_lossy().as_ref()]["trust_level"]
                .as_str(),
            Some("untrusted"),
            "a sibling directory must never be trusted as a side effect"
        );
        assert!(
            config["projects"]
                .as_table()
                .is_some_and(|projects| projects
                    .get(canonical_parent.to_string_lossy().as_ref())
                    .is_none()),
            "a parent directory must never be trusted as a side effect"
        );
    }

    #[test]
    fn managed_project_trust_refuses_device_prefix_alias_explicit_untrusted() {
        let project_path = Path::new(r"C:\repo\work\issue-3729");
        let alias = r"\\?\C:\repo\work\issue-3729";
        let mut alias_config = toml::Table::new();
        alias_config.insert(
            "trust_level".to_string(),
            toml::Value::String("untrusted".to_string()),
        );
        let mut projects = toml::Table::new();
        projects.insert(alias.to_string(), toml::Value::Table(alias_config));
        let mut root = toml::Table::new();
        root.insert("projects".to_string(), toml::Value::Table(projects));
        let original = root.clone();

        let error = ensure_codex_project_trust_level(&mut root, project_path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("explicitly untrusted"));
        assert_eq!(root, original);
    }

    #[test]
    fn managed_project_trust_refuses_case_and_separator_alias_explicit_untrusted() {
        let project_path = Path::new(r"C:\Repo\Work\issue-3729");
        let aliases = [
            r"c:/repo/work/ISSUE-3729",
            r"Microsoft.PowerShell.Core\FileSystem:://?/c:/repo/work/issue-3729",
        ];

        for alias in aliases {
            let mut alias_config = toml::Table::new();
            alias_config.insert(
                "trust_level".to_string(),
                toml::Value::String("untrusted".to_string()),
            );
            let mut projects = toml::Table::new();
            projects.insert(alias.to_string(), toml::Value::Table(alias_config));
            let mut root = toml::Table::new();
            root.insert("projects".to_string(), toml::Value::Table(projects));
            let original = root.clone();

            let error = ensure_codex_project_trust_level(&mut root, project_path).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{alias}");
            assert!(error.to_string().contains("explicitly untrusted"));
            assert_eq!(root, original);
        }
    }

    #[test]
    fn managed_project_trust_refuses_conflicting_device_prefix_aliases() {
        let project_path = Path::new(r"C:\repo\work\issue-3729");
        let alias = r"\\?\C:\repo\work\issue-3729";
        let mut exact_config = toml::Table::new();
        exact_config.insert(
            "sandbox_mode".to_string(),
            toml::Value::String("workspace-write".to_string()),
        );
        let mut alias_config = toml::Table::new();
        alias_config.insert(
            "trust_level".to_string(),
            toml::Value::String("trusted".to_string()),
        );
        let mut projects = toml::Table::new();
        projects.insert(
            project_path.to_string_lossy().into_owned(),
            toml::Value::Table(exact_config),
        );
        projects.insert(alias.to_string(), toml::Value::Table(alias_config));
        let mut root = toml::Table::new();
        root.insert("projects".to_string(), toml::Value::Table(projects));
        let original = root.clone();

        let error = ensure_codex_project_trust_level(&mut root, project_path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("conflicting aliases"));
        assert_eq!(root, original);
    }

    #[test]
    fn managed_project_trust_refuses_invalid_device_prefix_alias_values() {
        let project_path = Path::new(r"C:\repo\work\issue-3729");
        let alias = r"\\?\C:\repo\work\issue-3729";

        for alias_value in [
            {
                let mut table = toml::Table::new();
                table.insert(
                    "trust_level".to_string(),
                    toml::Value::String("ask".to_string()),
                );
                toml::Value::Table(table)
            },
            {
                let mut table = toml::Table::new();
                table.insert("trust_level".to_string(), toml::Value::Boolean(true));
                toml::Value::Table(table)
            },
            toml::Value::String("scalar project entry".to_string()),
        ] {
            let mut projects = toml::Table::new();
            projects.insert(alias.to_string(), alias_value);
            let mut root = toml::Table::new();
            root.insert("projects".to_string(), toml::Value::Table(projects));
            let original = root.clone();

            let error = ensure_codex_project_trust_level(&mut root, project_path).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(root, original);
        }
    }

    #[test]
    fn managed_project_trust_preserves_trusted_aliases_and_mutates_only_exact_target() {
        let project_path = Path::new("E:/gwt/work/issue-3729");
        let aliases = [
            "//?/E:/gwt/work/issue-3729",
            "Microsoft.PowerShell.Core\\FileSystem:://?/E:/gwt/work/issue-3729",
        ];
        let mut projects = toml::Table::new();
        for alias in aliases {
            let mut alias_config = toml::Table::new();
            alias_config.insert(
                "trust_level".to_string(),
                toml::Value::String("trusted".to_string()),
            );
            alias_config.insert("owner".to_string(), toml::Value::String(alias.to_string()));
            projects.insert(alias.to_string(), toml::Value::Table(alias_config));
        }
        let original_aliases = projects.clone();
        let mut root = toml::Table::new();
        root.insert("projects".to_string(), toml::Value::Table(projects));

        ensure_codex_project_trust_level(&mut root, project_path).unwrap();

        let projects = root["projects"].as_table().unwrap();
        assert_eq!(projects.len(), aliases.len() + 1);
        assert_eq!(
            projects[project_path.to_string_lossy().as_ref()]["trust_level"].as_str(),
            Some("trusted")
        );
        for alias in aliases {
            assert_eq!(projects.get(alias), original_aliases.get(alias));
        }
    }

    #[test]
    fn managed_project_trust_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let config_path = dir.path().join("codex/config.toml");

        let first = register_codex_managed_project_trust(&worktree, &config_path).unwrap();
        let first_config = fs::read_to_string(&config_path).unwrap();
        let second = register_codex_managed_project_trust(&worktree, &config_path).unwrap();
        let second_config = fs::read_to_string(&config_path).unwrap();

        assert_eq!(second, first);
        assert_eq!(second_config, first_config);
    }

    #[test]
    fn managed_project_trust_write_failure_preserves_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let config_path = dir.path().join("codex/config.toml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let original = "model = \"gpt-5.6-sol\"\n";
        fs::write(&config_path, original).unwrap();

        let error = register_codex_managed_project_trust_with_writer(
            &worktree,
            &config_path,
            |_path, _rendered| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected platform-stable config write failure",
                ))
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&config_path).unwrap(), original);
    }

    #[test]
    fn managed_project_trust_lock_setup_failure_preserves_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let blocked_parent = dir.path().join("codex-home-is-a-file");
        let original = "do not replace this file\n";
        fs::write(&blocked_parent, original).unwrap();
        let config_path = blocked_parent.join("config.toml");

        register_codex_managed_project_trust(&worktree, &config_path)
            .expect_err("lock parent creation must fail closed");

        assert_eq!(fs::read_to_string(&blocked_parent).unwrap(), original);
        assert!(!config_path.exists());
    }

    #[test]
    fn managed_project_trust_refuses_unknown_or_non_string_trust_levels() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let canonical_worktree = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&worktree).unwrap(),
        );
        let config_path = dir.path().join("codex/config.toml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        for unsupported in [
            toml::Value::String("ask".to_string()),
            toml::Value::Boolean(true),
        ] {
            let mut project = toml::Table::new();
            project.insert("trust_level".to_string(), unsupported);
            let mut projects = toml::Table::new();
            projects.insert(
                canonical_worktree.to_string_lossy().into_owned(),
                toml::Value::Table(project),
            );
            let mut root = toml::Table::new();
            root.insert("projects".to_string(), toml::Value::Table(projects));
            let original = toml::to_string_pretty(&toml::Value::Table(root)).unwrap();
            fs::write(&config_path, &original).unwrap();

            let error = register_codex_managed_project_trust(&worktree, &config_path).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(fs::read_to_string(&config_path).unwrap(), original);
        }
    }

    #[test]
    fn managed_project_trust_refuses_malformed_or_scalar_projects_without_rewriting() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let config_path = dir.path().join("codex/config.toml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        for original in ["model = [\n", "projects = \"legacy scalar\"\n"] {
            fs::write(&config_path, original).unwrap();

            let error = register_codex_managed_project_trust(&worktree, &config_path).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(fs::read_to_string(&config_path).unwrap(), original);
        }
    }

    #[test]
    fn managed_project_trust_refuses_to_override_explicit_untrusted() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let canonical_worktree = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&worktree).unwrap(),
        );
        let config_path = dir.path().join("codex/config.toml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        let mut project = toml::Table::new();
        project.insert(
            "trust_level".to_string(),
            toml::Value::String("untrusted".to_string()),
        );
        let mut projects = toml::Table::new();
        projects.insert(
            canonical_worktree.to_string_lossy().into_owned(),
            toml::Value::Table(project),
        );
        let mut root = toml::Table::new();
        root.insert("projects".to_string(), toml::Value::Table(projects));
        let original = toml::to_string_pretty(&toml::Value::Table(root)).unwrap();
        fs::write(&config_path, &original).unwrap();

        let error = register_codex_managed_project_trust(&worktree, &config_path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("explicitly untrusted"));
        assert_eq!(fs::read_to_string(config_path).unwrap(), original);
    }

    #[test]
    fn project_and_hook_trust_share_one_config_rmw_lock() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        generate_codex_hooks(&worktree).unwrap();
        let config_path = dir.path().join("codex/config.toml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let lock_path = codex_config_lock_path(&config_path);
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .unwrap();
        FileExt::lock_exclusive(&lock).unwrap();

        let (project_tx, project_rx) = mpsc::channel();
        let project_worktree = worktree.clone();
        let project_config = config_path.clone();
        let project_thread = thread::spawn(move || {
            project_tx
                .send(register_codex_managed_project_trust(
                    &project_worktree,
                    &project_config,
                ))
                .unwrap();
        });
        let (hook_tx, hook_rx) = mpsc::channel();
        let hook_worktree = worktree.clone();
        let hook_config = config_path.clone();
        let hook_thread = thread::spawn(move || {
            hook_tx
                .send(register_codex_managed_hook_trust(
                    &hook_worktree,
                    &hook_config,
                ))
                .unwrap();
        });

        assert!(matches!(
            project_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(matches!(
            hook_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(lock);

        project_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("project registration should resume after unlock")
            .unwrap();
        hook_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("hook registration should resume after unlock")
            .unwrap();
        project_thread.join().unwrap();
        hook_thread.join().unwrap();

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();
        let canonical_worktree = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(worktree).unwrap(),
        );
        assert_eq!(
            config["projects"][canonical_worktree.to_string_lossy().as_ref()]["trust_level"]
                .as_str(),
            Some("trusted")
        );
        assert!(
            config["hooks"]["state"]
                .as_table()
                .is_some_and(|state| !state.is_empty()),
            "hook update must survive the concurrent project RMW"
        );
    }

    #[test]
    fn concurrent_project_trust_registrations_preserve_both_exact_entries() {
        let dir = tempfile::tempdir().unwrap();
        let first_worktree = dir.path().join("first-managed-worktree");
        let second_worktree = dir.path().join("second-managed-worktree");
        fs::create_dir_all(&first_worktree).unwrap();
        fs::create_dir_all(&second_worktree).unwrap();
        let config_path = dir.path().join("codex/config.toml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let lock_path = codex_config_lock_path(&config_path);
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .unwrap();
        FileExt::lock_exclusive(&lock).unwrap();

        let (tx, rx) = mpsc::channel();
        let mut threads = Vec::new();
        for worktree in [&first_worktree, &second_worktree] {
            let tx = tx.clone();
            let worktree = worktree.clone();
            let config_path = config_path.clone();
            threads.push(thread::spawn(move || {
                tx.send(register_codex_managed_project_trust(
                    &worktree,
                    &config_path,
                ))
                .unwrap();
            }));
        }
        drop(tx);

        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(lock);

        for _ in 0..2 {
            rx.recv_timeout(Duration::from_secs(2))
                .expect("project registration should resume after unlock")
                .unwrap();
        }
        for thread in threads {
            thread.join().unwrap();
        }

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        for worktree in [first_worktree, second_worktree] {
            let canonical = gwt_core::paths::normalize_windows_child_process_path(
                &fs::canonicalize(worktree).unwrap(),
            );
            assert_eq!(
                config["projects"][canonical.to_string_lossy().as_ref()]["trust_level"].as_str(),
                Some("trusted")
            );
        }
    }

    // SPEC #1921 T528 / FR-244..FR-246: pre-delete revocation owns only the
    // exact managed project's `trusted` value and shares the config RMW lock.
    #[test]
    fn managed_project_trust_revocation_removes_only_trusted_and_preserves_unrelated_config() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let project_path = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&worktree).unwrap(),
        );
        let project_key = project_path.to_string_lossy().into_owned();
        let config_path = dir.path().join("config.toml");

        let mut target = toml::Table::new();
        target.insert(
            "trust_level".to_string(),
            toml::Value::String("trusted".to_string()),
        );
        target.insert(
            "sandbox_mode".to_string(),
            toml::Value::String("workspace-write".to_string()),
        );
        let mut metadata = toml::Table::new();
        metadata.insert(
            "owner".to_string(),
            toml::Value::String("user-value".to_string()),
        );
        target.insert("metadata".to_string(), toml::Value::Table(metadata));

        let other_project_key = "/repo/work/other";
        let mut other_project = toml::Table::new();
        other_project.insert(
            "trust_level".to_string(),
            toml::Value::String("trusted".to_string()),
        );
        let mut projects = toml::Table::new();
        projects.insert(project_key.clone(), toml::Value::Table(target));
        projects.insert(
            other_project_key.to_string(),
            toml::Value::Table(other_project.clone()),
        );

        let hook_key = "/repo/.codex/hooks.json:stop:0:0";
        let mut hook_entry = toml::Table::new();
        hook_entry.insert("enabled".to_string(), toml::Value::Boolean(true));
        hook_entry.insert(
            "trusted_hash".to_string(),
            toml::Value::String("sha256:user-hook-state".to_string()),
        );
        let mut hook_state = toml::Table::new();
        hook_state.insert(hook_key.to_string(), toml::Value::Table(hook_entry));
        let mut hooks = toml::Table::new();
        hooks.insert("state".to_string(), toml::Value::Table(hook_state));

        let mut root = toml::Table::new();
        root.insert(
            "model".to_string(),
            toml::Value::String("gpt-6-astra".to_string()),
        );
        root.insert("projects".to_string(), toml::Value::Table(projects));
        root.insert("hooks".to_string(), toml::Value::Table(hooks));
        fs::write(
            &config_path,
            toml::to_string_pretty(&toml::Value::Table(root)).unwrap(),
        )
        .unwrap();

        revoke_codex_managed_project_trust(&worktree, &config_path).unwrap();

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        let target = config["projects"][&project_key].as_table().unwrap();
        assert!(!target.contains_key("trust_level"));
        assert_eq!(
            target.get("sandbox_mode").and_then(toml::Value::as_str),
            Some("workspace-write")
        );
        assert_eq!(target["metadata"]["owner"].as_str(), Some("user-value"));
        assert_eq!(
            config["projects"][other_project_key].as_table().unwrap(),
            &other_project
        );
        assert_eq!(config["model"].as_str(), Some("gpt-6-astra"));
        assert_eq!(
            config["hooks"]["state"][hook_key]["trusted_hash"].as_str(),
            Some("sha256:user-hook-state")
        );
    }

    #[test]
    fn managed_project_trust_revocation_removes_the_target_table_only_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let project_path = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&worktree).unwrap(),
        );
        let project_key = project_path.to_string_lossy().into_owned();
        let config_path = dir.path().join("config.toml");

        let mut target = toml::Table::new();
        target.insert(
            "trust_level".to_string(),
            toml::Value::String("trusted".to_string()),
        );
        let mut other = toml::Table::new();
        other.insert(
            "trust_level".to_string(),
            toml::Value::String("untrusted".to_string()),
        );
        let mut projects = toml::Table::new();
        projects.insert(project_key.clone(), toml::Value::Table(target));
        projects.insert("/repo/work/other".to_string(), toml::Value::Table(other));
        let mut root = toml::Table::new();
        root.insert("projects".to_string(), toml::Value::Table(projects));
        fs::write(
            &config_path,
            toml::to_string_pretty(&toml::Value::Table(root)).unwrap(),
        )
        .unwrap();

        revoke_codex_managed_project_trust(&worktree, &config_path).unwrap();

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        let projects = config["projects"].as_table().unwrap();
        assert!(!projects.contains_key(&project_key));
        assert_eq!(
            projects["/repo/work/other"]["trust_level"].as_str(),
            Some("untrusted")
        );
    }

    #[test]
    fn managed_project_trust_revocation_preserves_user_owned_levels_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let project_path = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&worktree).unwrap(),
        );
        let project_key = project_path.to_string_lossy().into_owned();
        let config_path = dir.path().join("config.toml");

        for level in ["untrusted", "ask"] {
            let mut target = toml::Table::new();
            target.insert(
                "trust_level".to_string(),
                toml::Value::String(level.to_string()),
            );
            let mut projects = toml::Table::new();
            projects.insert(project_key.clone(), toml::Value::Table(target));
            let mut root = toml::Table::new();
            root.insert("projects".to_string(), toml::Value::Table(projects));
            let original = toml::to_string_pretty(&toml::Value::Table(root)).unwrap();
            fs::write(&config_path, &original).unwrap();

            revoke_codex_managed_project_trust_with_writer(
                &worktree,
                &config_path,
                |_path, _rendered| panic!("{level} is user-owned and must not trigger a write"),
            )
            .unwrap();

            assert_eq!(fs::read_to_string(&config_path).unwrap(), original);
        }
    }

    #[test]
    fn managed_project_trust_revocation_is_idempotent_for_missing_state() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let project_path = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&worktree).unwrap(),
        );
        let project_key = project_path.to_string_lossy().into_owned();

        let missing_config = dir.path().join("missing/config.toml");
        for _ in 0..2 {
            revoke_codex_managed_project_trust_with_writer(
                &worktree,
                &missing_config,
                |_path, _rendered| panic!("an absent config is an idempotent no-op"),
            )
            .unwrap();
        }
        assert!(!missing_config.exists());

        let no_projects_config = dir.path().join("no-projects.toml");
        let no_projects_original = "model = \"gpt-6-astra\"\n";
        fs::write(&no_projects_config, no_projects_original).unwrap();
        for _ in 0..2 {
            revoke_codex_managed_project_trust_with_writer(
                &worktree,
                &no_projects_config,
                |_path, _rendered| panic!("a missing projects table must not trigger a write"),
            )
            .unwrap();
        }
        assert_eq!(
            fs::read_to_string(&no_projects_config).unwrap(),
            no_projects_original
        );

        let missing_entry_config = dir.path().join("missing-entry.toml");
        let mut other = toml::Table::new();
        other.insert(
            "trust_level".to_string(),
            toml::Value::String("trusted".to_string()),
        );
        let mut projects = toml::Table::new();
        projects.insert("/repo/work/other".to_string(), toml::Value::Table(other));
        assert!(!projects.contains_key(&project_key));
        let mut root = toml::Table::new();
        root.insert("projects".to_string(), toml::Value::Table(projects));
        let missing_entry_original = toml::to_string_pretty(&toml::Value::Table(root)).unwrap();
        fs::write(&missing_entry_config, &missing_entry_original).unwrap();
        for _ in 0..2 {
            revoke_codex_managed_project_trust_with_writer(
                &worktree,
                &missing_entry_config,
                |_path, _rendered| panic!("an absent project entry must not trigger a write"),
            )
            .unwrap();
        }
        assert_eq!(
            fs::read_to_string(&missing_entry_config).unwrap(),
            missing_entry_original
        );
    }

    #[test]
    fn managed_project_trust_revocation_rejects_malformed_and_wrongly_typed_config() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let project_path = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&worktree).unwrap(),
        );
        let project_key = project_path.to_string_lossy().into_owned();
        let config_path = dir.path().join("config.toml");

        let mut scalar_project_entries = toml::Table::new();
        scalar_project_entries.insert(
            project_key.clone(),
            toml::Value::String("scalar project entry".to_string()),
        );
        let mut scalar_project_root = toml::Table::new();
        scalar_project_root.insert(
            "projects".to_string(),
            toml::Value::Table(scalar_project_entries),
        );

        let mut typed_trust = toml::Table::new();
        typed_trust.insert("trust_level".to_string(), toml::Value::Boolean(true));
        let mut typed_trust_projects = toml::Table::new();
        typed_trust_projects.insert(project_key, toml::Value::Table(typed_trust));
        let mut typed_trust_root = toml::Table::new();
        typed_trust_root.insert(
            "projects".to_string(),
            toml::Value::Table(typed_trust_projects),
        );

        let cases = [
            "model = [\n".to_string(),
            "projects = \"legacy scalar\"\n".to_string(),
            toml::to_string_pretty(&toml::Value::Table(scalar_project_root)).unwrap(),
            toml::to_string_pretty(&toml::Value::Table(typed_trust_root)).unwrap(),
        ];
        for original in cases {
            fs::write(&config_path, &original).unwrap();

            let error = revoke_codex_managed_project_trust(&worktree, &config_path).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{original:?}");
            assert_eq!(fs::read_to_string(&config_path).unwrap(), original);
        }
    }

    #[test]
    fn managed_project_trust_revocation_matches_windows_aliases_and_preserves_user_choices() {
        let project_path = Path::new(r"E:\GWT\Work\issue-3729");
        let empty_trusted_alias = r"e:/gwt/work/ISSUE-3729";
        let retained_trusted_alias = r"\\?\E:\GWT\Work\issue-3729";
        let untrusted_alias = r"Microsoft.PowerShell.Core\FileSystem::E:\gwt\work\ISSUE-3729";
        let unsupported_alias = r"Microsoft.PowerShell.Core\FileSystem:://?/e:/gwt/work/issue-3729";

        let mut empty_trusted = toml::Table::new();
        empty_trusted.insert(
            "trust_level".to_string(),
            toml::Value::String("trusted".to_string()),
        );
        let mut retained_trusted = empty_trusted.clone();
        retained_trusted.insert(
            "owner".to_string(),
            toml::Value::String("keep-me".to_string()),
        );
        let mut untrusted = toml::Table::new();
        untrusted.insert(
            "trust_level".to_string(),
            toml::Value::String("untrusted".to_string()),
        );
        let mut unsupported = toml::Table::new();
        unsupported.insert(
            "trust_level".to_string(),
            toml::Value::String("ask".to_string()),
        );

        let mut projects = toml::Table::new();
        projects.insert(
            empty_trusted_alias.to_string(),
            toml::Value::Table(empty_trusted),
        );
        projects.insert(
            retained_trusted_alias.to_string(),
            toml::Value::Table(retained_trusted),
        );
        projects.insert(
            untrusted_alias.to_string(),
            toml::Value::Table(untrusted.clone()),
        );
        projects.insert(
            unsupported_alias.to_string(),
            toml::Value::Table(unsupported.clone()),
        );
        let mut root = toml::Table::new();
        root.insert("projects".to_string(), toml::Value::Table(projects));

        revoke_codex_project_trust_level(&mut root, project_path).unwrap();

        let projects = root["projects"].as_table().unwrap();
        assert!(!projects.contains_key(empty_trusted_alias));
        assert_eq!(
            projects[retained_trusted_alias]["owner"].as_str(),
            Some("keep-me")
        );
        assert!(projects[retained_trusted_alias]
            .as_table()
            .is_some_and(|project| !project.contains_key("trust_level")));
        assert_eq!(
            projects.get(untrusted_alias),
            Some(&toml::Value::Table(untrusted))
        );
        assert_eq!(
            projects.get(unsupported_alias),
            Some(&toml::Value::Table(unsupported))
        );
    }

    #[test]
    fn managed_project_trust_revocation_write_failure_preserves_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let project_path = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&worktree).unwrap(),
        );
        let config_path = dir.path().join("config.toml");

        let mut target = toml::Table::new();
        target.insert(
            "trust_level".to_string(),
            toml::Value::String("trusted".to_string()),
        );
        let mut projects = toml::Table::new();
        projects.insert(
            project_path.to_string_lossy().into_owned(),
            toml::Value::Table(target),
        );
        let mut root = toml::Table::new();
        root.insert("projects".to_string(), toml::Value::Table(projects));
        let original = toml::to_string_pretty(&toml::Value::Table(root)).unwrap();
        fs::write(&config_path, &original).unwrap();

        let error = revoke_codex_managed_project_trust_with_writer(
            &worktree,
            &config_path,
            |_path, _rendered| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected config write failure",
                ))
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&config_path).unwrap(), original);
    }

    #[test]
    fn managed_project_trust_revocation_lock_setup_failure_preserves_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let blocked_parent = dir.path().join("codex-home-is-a-file");
        let original = "do not replace this file\n";
        fs::write(&blocked_parent, original).unwrap();
        let config_path = blocked_parent.join("config.toml");

        revoke_codex_managed_project_trust(&worktree, &config_path)
            .expect_err("lock parent creation must fail closed");

        assert_eq!(fs::read_to_string(&blocked_parent).unwrap(), original);
        assert!(!config_path.exists());
    }

    #[test]
    fn managed_project_trust_revocation_waits_for_the_shared_config_lock() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("managed-worktree");
        fs::create_dir_all(&worktree).unwrap();
        let project_path = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&worktree).unwrap(),
        );
        let config_path = dir.path().join("config.toml");

        let mut target = toml::Table::new();
        target.insert(
            "trust_level".to_string(),
            toml::Value::String("trusted".to_string()),
        );
        let mut projects = toml::Table::new();
        projects.insert(
            project_path.to_string_lossy().into_owned(),
            toml::Value::Table(target),
        );
        let mut root = toml::Table::new();
        root.insert("projects".to_string(), toml::Value::Table(projects));
        fs::write(
            &config_path,
            toml::to_string_pretty(&toml::Value::Table(root)).unwrap(),
        )
        .unwrap();

        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(codex_config_lock_path(&config_path))
            .unwrap();
        FileExt::lock_exclusive(&lock).unwrap();

        let (tx, rx) = mpsc::channel();
        let revoke_worktree = worktree.clone();
        let revoke_config = config_path.clone();
        let revoke_thread = thread::spawn(move || {
            tx.send(revoke_codex_managed_project_trust(
                &revoke_worktree,
                &revoke_config,
            ))
            .unwrap();
        });

        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(lock);
        rx.recv_timeout(Duration::from_secs(2))
            .expect("revocation should resume after shared lock release")
            .unwrap();
        revoke_thread.join().unwrap();

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert!(!config["projects"]
            .as_table()
            .unwrap()
            .contains_key(project_path.to_string_lossy().as_ref()));
    }

    #[test]
    fn concurrent_project_revocation_registration_and_hook_registration_preserve_all_updates() {
        let dir = tempfile::tempdir().unwrap();
        let revoked_worktree = dir.path().join("revoked-worktree");
        let registered_worktree = dir.path().join("registered-worktree");
        let hook_worktree = dir.path().join("hook-worktree");
        fs::create_dir_all(&revoked_worktree).unwrap();
        fs::create_dir_all(&registered_worktree).unwrap();
        fs::create_dir_all(&hook_worktree).unwrap();
        generate_codex_hooks_for_mode(&hook_worktree, CodexHookDiscoveryMode::WorktreeLocal)
            .unwrap();

        let revoked_project = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&revoked_worktree).unwrap(),
        );
        let registered_project = gwt_core::paths::normalize_windows_child_process_path(
            &fs::canonicalize(&registered_worktree).unwrap(),
        );
        let config_path = dir.path().join("config.toml");
        let mut revoked = toml::Table::new();
        revoked.insert(
            "trust_level".to_string(),
            toml::Value::String("trusted".to_string()),
        );
        let mut projects = toml::Table::new();
        projects.insert(
            revoked_project.to_string_lossy().into_owned(),
            toml::Value::Table(revoked),
        );
        let mut root = toml::Table::new();
        root.insert(
            "model".to_string(),
            toml::Value::String("gpt-6-astra".to_string()),
        );
        root.insert("projects".to_string(), toml::Value::Table(projects));
        fs::write(
            &config_path,
            toml::to_string_pretty(&toml::Value::Table(root)).unwrap(),
        )
        .unwrap();

        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(codex_config_lock_path(&config_path))
            .unwrap();
        FileExt::lock_exclusive(&lock).unwrap();

        let (tx, rx) = mpsc::channel();
        let revoke_tx = tx.clone();
        let revoke_path = revoked_worktree.clone();
        let revoke_config = config_path.clone();
        let revoke_thread = thread::spawn(move || {
            revoke_tx
                .send(
                    revoke_codex_managed_project_trust(&revoke_path, &revoke_config)
                        .map(|_report| ()),
                )
                .unwrap();
        });
        let register_tx = tx.clone();
        let register_path = registered_worktree.clone();
        let register_config = config_path.clone();
        let register_thread = thread::spawn(move || {
            register_tx
                .send(
                    register_codex_managed_project_trust(&register_path, &register_config)
                        .map(|_report| ()),
                )
                .unwrap();
        });
        let hook_tx = tx.clone();
        let hook_path = hook_worktree.clone();
        let hook_config = config_path.clone();
        let hook_thread = thread::spawn(move || {
            hook_tx
                .send(
                    register_codex_managed_hook_trust_for_mode(
                        &hook_path,
                        &hook_config,
                        CodexHookDiscoveryMode::WorktreeLocal,
                    )
                    .map(|_report| ()),
                )
                .unwrap();
        });
        drop(tx);

        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(lock);
        for _ in 0..3 {
            rx.recv_timeout(Duration::from_secs(3))
                .expect("all config operations should resume after shared lock release")
                .unwrap();
        }
        revoke_thread.join().unwrap();
        register_thread.join().unwrap();
        hook_thread.join().unwrap();

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        let projects = config["projects"].as_table().unwrap();
        assert!(!projects.contains_key(revoked_project.to_string_lossy().as_ref()));
        assert_eq!(
            projects[registered_project.to_string_lossy().as_ref()]["trust_level"].as_str(),
            Some("trusted")
        );
        assert!(config["hooks"]["state"]
            .as_table()
            .is_some_and(|state| !state.is_empty()));
        assert_eq!(config["model"].as_str(), Some("gpt-6-astra"));
    }

    #[test]
    fn managed_project_trust_same_path_registration_cannot_outlive_locked_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("worktree");
        fs::create_dir_all(&worktree).unwrap();
        let config_path = dir.path().join("config.toml");
        let project_path = register_codex_managed_project_trust(&worktree, &config_path)
            .expect("seed project trust")
            .project_path;

        let (removed_tx, removed_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let cleanup_worktree = worktree.clone();
        let cleanup_config = config_path.clone();
        let cleanup = thread::spawn(move || {
            revoke_codex_managed_project_trust_with_cleanup(
                &cleanup_worktree,
                &cleanup_config,
                || {
                    fs::remove_dir_all(&cleanup_worktree)?;
                    removed_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                },
            )
        });
        removed_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("cleanup must remove the worktree while holding the config lock");

        let (registered_tx, registered_rx) = mpsc::channel();
        let register_worktree = worktree.clone();
        let register_config = config_path.clone();
        let registration = thread::spawn(move || {
            registered_tx
                .send(register_codex_managed_project_trust(
                    &register_worktree,
                    &register_config,
                ))
                .unwrap();
        });

        assert!(matches!(
            registered_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release_tx.send(()).unwrap();
        cleanup.join().unwrap().expect("cleanup transaction");
        let registration_error = registered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("registration must finish after cleanup releases the lock")
            .expect_err("deleted worktree must not be trusted again");
        registration.join().unwrap();
        assert_eq!(registration_error.kind(), io::ErrorKind::NotFound);

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert!(
            config
                .get("projects")
                .and_then(toml::Value::as_table)
                .is_none_or(|projects| {
                    !projects.contains_key(project_path.to_string_lossy().as_ref())
                }),
            "the deleted path must remain untrusted after the waiting registrar exits"
        );
    }

    #[test]
    fn linked_worktree_trust_entries_use_root_checkout_hook_path() {
        let dir = tempfile::tempdir().unwrap();
        let root_checkout = dir.path().join("project");
        let common_git_dir = root_checkout.join("project.git");
        let worktree = root_checkout.join("work/20260524-0545");
        fs::create_dir_all(common_git_dir.join("worktrees/20260524-0545")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!(
                "gitdir: {}\n",
                common_git_dir.join("worktrees/20260524-0545").display()
            ),
        )
        .unwrap();
        generate_codex_hooks(&worktree).unwrap();
        let root_hooks_path = dunce::canonicalize(root_checkout.join(".codex/hooks.json")).unwrap();
        let worktree_hooks_prefix = worktree.join(".codex/hooks.json").display().to_string();

        let entries = collect_codex_managed_hook_trust_entries(&worktree).unwrap();

        assert_eq!(entries.len(), 5);
        assert!(
            entries
                .iter()
                .all(|entry| entry.key.starts_with(&root_hooks_path.display().to_string())),
            "Codex 0.133 linked-worktree trust keys must use root checkout hook path {root_hooks_path:?}; got {entries:?}"
        );
        assert!(
            entries.iter().all(|entry| {
                !entry
                    .key
                    .starts_with(&worktree_hooks_prefix)
            }),
            "worktree-local hook keys are ignored by Codex linked-worktree discovery; got {entries:?}"
        );
    }

    #[test]
    fn linked_worktree_trust_entries_can_use_worktree_hook_path_for_older_codex() {
        let dir = tempfile::tempdir().unwrap();
        let root_checkout = dir.path().join("project");
        let common_git_dir = root_checkout.join("project.git");
        let worktree = root_checkout.join("work/20260524-0545");
        fs::create_dir_all(common_git_dir.join("worktrees/20260524-0545")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!(
                "gitdir: {}\n",
                common_git_dir.join("worktrees/20260524-0545").display()
            ),
        )
        .unwrap();
        generate_codex_hooks_for_mode(&worktree, CodexHookDiscoveryMode::WorktreeLocal).unwrap();
        let worktree_hooks_path = dunce::canonicalize(worktree.join(".codex/hooks.json")).unwrap();

        let entries = collect_codex_managed_hook_trust_entries_for_mode(
            &worktree,
            CodexHookDiscoveryMode::WorktreeLocal,
        )
        .unwrap();

        assert_eq!(entries.len(), 5);
        assert!(
            entries
                .iter()
                .all(|entry| entry.key.starts_with(&worktree_hooks_path.display().to_string())),
            "Codex < 0.131.0-alpha.21 trust keys must use worktree hook path {worktree_hooks_path:?}; got {entries:?}"
        );
    }

    #[test]
    fn linked_worktree_trust_entries_can_register_both_paths_for_unknown_codex() {
        let dir = tempfile::tempdir().unwrap();
        let root_checkout = dir.path().join("project");
        let common_git_dir = root_checkout.join("project.git");
        let worktree = root_checkout.join("work/20260524-0545");
        fs::create_dir_all(common_git_dir.join("worktrees/20260524-0545")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!(
                "gitdir: {}\n",
                common_git_dir.join("worktrees/20260524-0545").display()
            ),
        )
        .unwrap();
        generate_codex_hooks_for_mode(&worktree, CodexHookDiscoveryMode::Both).unwrap();
        let root_hooks_path = dunce::canonicalize(root_checkout.join(".codex/hooks.json")).unwrap();
        let worktree_hooks_path = dunce::canonicalize(worktree.join(".codex/hooks.json")).unwrap();

        let entries = collect_codex_managed_hook_trust_entries_for_mode(
            &worktree,
            CodexHookDiscoveryMode::Both,
        )
        .unwrap();

        assert_eq!(entries.len(), 10);
        assert!(entries.iter().any(|entry| entry
            .key
            .starts_with(&root_hooks_path.display().to_string())));
        assert!(entries.iter().any(|entry| entry
            .key
            .starts_with(&worktree_hooks_path.display().to_string())));
    }

    /// #3567: a git-tracked `.codex/hooks.json` carries the canonical portable
    /// fallback, not the running binary's absolute path. Trust registration must
    /// expect the same thing, or every launch into a repo that commits its hook
    /// config stops on `Hooks need review`.
    #[test]
    fn tracked_hooks_are_trusted_against_the_canonical_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let codex_dir = repo.join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let mut hooks = serde_json::Map::new();
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
        ] {
            hooks.insert(
                event.to_string(),
                json!([
                    {
                        "matcher": "*",
                        "hooks": [
                            {
                                "command": codex_event_hook_commands_with_bin(
                                    crate::CANONICAL_HOOK_BIN,
                                    event,
                                )
                                .into_iter()
                                .next()
                                .unwrap(),
                                "type": "command"
                            }
                        ]
                    }
                ]),
            );
        }
        fs::write(
            codex_dir.join("hooks.json"),
            serde_json::to_string_pretty(&json!({ "hooks": hooks })).unwrap(),
        )
        .unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test"],
            vec!["add", "-A"],
            vec!["commit", "-qm", "hooks"],
        ] {
            assert!(gwt_core::process::hidden_command("git")
                .arg("-C")
                .arg(repo)
                .args(&args)
                .status()
                .unwrap()
                .success());
        }

        let entries = collect_codex_managed_hook_trust_entries_with_expected_bin(
            repo,
            Some("/Applications/GWT.app/Contents/MacOS/gwtd"),
        )
        .unwrap();

        assert_eq!(
            entries.len(),
            5,
            "a tracked hook config must be trusted against the canonical fallback it carries"
        );
    }

    /// #3567: when the resolved binary is a foreign checkout's build output, the
    /// generator writes the canonical fallback instead of pinning it. Trust must
    /// apply the same reduction to the caller's expected binary — this is the
    /// shape a developer running `target/debug/gwtd` against any other worktree
    /// hits on every launch.
    #[test]
    fn foreign_build_output_hooks_are_trusted_against_the_canonical_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let foreign_build_output = "/repo/work/issue-1/target/debug/gwtd";
        let mut hooks = serde_json::Map::new();
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
        ] {
            hooks.insert(
                event.to_string(),
                json!([
                    {
                        "matcher": "*",
                        "hooks": [
                            {
                                "command": codex_event_hook_commands_with_bin(
                                    crate::CANONICAL_HOOK_BIN,
                                    event,
                                )
                                .into_iter()
                                .next()
                                .unwrap(),
                                "type": "command"
                            }
                        ]
                    }
                ]),
            );
        }
        fs::write(
            codex_dir.join("hooks.json"),
            serde_json::to_string_pretty(&json!({ "hooks": hooks })).unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries_with_expected_bin(
            dir.path(),
            Some(foreign_build_output),
        )
        .unwrap();

        assert_eq!(
            entries.len(),
            5,
            "trust must reduce a foreign build output the same way generation did"
        );
    }

    #[test]
    fn portable_generated_hooks_are_trusted_for_explicit_expected_fallback_path() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let expected_fallback = "/host/gwt/bin/gwtd";
        let mut hooks = serde_json::Map::new();
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
        ] {
            hooks.insert(
                event.to_string(),
                json!([
                    {
                        "matcher": "*",
                        "hooks": [
                            {
                                "command": codex_event_hook_commands_with_bin(expected_fallback, event)
                                    .into_iter()
                                    .next()
                                    .unwrap(),
                                "type": "command"
                            }
                        ]
                    }
                ]),
            );
        }
        fs::write(
            codex_dir.join("hooks.json"),
            serde_json::to_string_pretty(&json!({ "hooks": hooks })).unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries_with_expected_bin(
            dir.path(),
            Some(expected_fallback),
        )
        .unwrap();

        assert_eq!(
            entries.len(),
            5,
            "container-local registration must accept the exact host-generated fallback path"
        );
    }

    #[test]
    fn powershell_generated_hook_with_expected_fallback_is_trusted_on_posix_registration() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let expected_fallback = "C:/Program Files/GWT/gwtd.exe";
        let powershell_stop_command = codex_event_hook_commands_with_bin(expected_fallback, "Stop")
            .into_iter()
            .nth(1)
            .expect("PowerShell generated command");
        fs::write(
            codex_dir.join("hooks.json"),
            serde_json::to_string_pretty(&json!({
                "hooks": {
                    "Stop": [
                        {
                            "matcher": "*",
                            "hooks": [
                                {
                                    "command": powershell_stop_command,
                                    "type": "command"
                                }
                            ]
                        }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries_with_expected_bin(
            dir.path(),
            Some(expected_fallback),
        )
        .unwrap();

        assert_eq!(
            entries.len(),
            1,
            "Linux container registration must trust exact PowerShell-generated Codex hooks"
        );
    }

    #[test]
    fn portable_generated_hook_with_unexpected_fallback_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let hooks_content = fs::read_to_string(&hooks_path).unwrap();
        let mut hooks_json: Value = serde_json::from_str(&hooks_content).unwrap();
        hooks_json["hooks"]["Stop"][0]["hooks"][0]["command"] = Value::String(
            codex_event_hook_commands_with_bin("/tmp/attacker/gwtd", "Stop")
                .into_iter()
                .next()
                .unwrap(),
        );
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries(dir.path()).unwrap();

        assert_eq!(
            entries.len(),
            4,
            "unexpected portable fallback path must be left for Codex /hooks review"
        );
        assert!(
            entries.iter().all(|entry| !entry.key.contains(":stop:")),
            "unexpected fallback Stop hook must not be trusted; got {entries:?}"
        );
    }

    /// The exact repo-owned Stop hook committed in this repository's
    /// `.codex/hooks.json`. Kept verbatim so the fixture drifts loudly if the
    /// committed command ever changes shape.
    const REPO_OWNED_SELF_IMPROVEMENT_STOP_COMMAND: &str =
        "gwt_bin=\"${GWT_BIN_PATH:-gwtd}\"; \"$gwt_bin\" hook gwt-self-improvement-stop 2>/dev/null || true";

    fn self_improvement_stop_group() -> Value {
        json!({
            "matcher": "*",
            "hooks": [
                {
                    "command": REPO_OWNED_SELF_IMPROVEMENT_STOP_COMMAND,
                    "type": "command"
                }
            ]
        })
    }

    /// Issue #3967 AC-1: the repo-owned `gwt-self-improvement-stop` hook lives
    /// at Stop group index 1, which the trust collector never reached. Codex
    /// then reported it as "new or changed" and blocked the pane on
    /// `Hooks need review`.
    #[test]
    fn repo_owned_self_improvement_stop_hook_is_trusted_at_group_index_one() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let mut hooks_json: Value =
            serde_json::from_str(&fs::read_to_string(&hooks_path).unwrap()).unwrap();
        hooks_json["hooks"]["Stop"]
            .as_array_mut()
            .unwrap()
            .push(self_improvement_stop_group());
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();
        let canonical = dunce::canonicalize(&hooks_path).unwrap();

        let entries = collect_codex_managed_hook_trust_entries(dir.path()).unwrap();

        assert_eq!(
            entries.len(),
            6,
            "every gwt hook in the file must be trusted, including Stop group 1: {entries:?}"
        );
        let expected_key = format!("{}:stop:1:0", canonical.display());
        let entry = entries
            .iter()
            .find(|entry| entry.key == expected_key)
            .unwrap_or_else(|| panic!("missing repo-owned Stop trust key {expected_key}"));
        assert_eq!(
            entry.trusted_hash,
            "sha256:77ebe50138a4b5c5260d141ae62f91964676abd81a49780101e5f5685e6b0685"
        );
    }

    /// Issue #3967 AC-2: a Windows PowerShell hook command carrying a
    /// machine-local absolute path must produce the exact Codex trust identity,
    /// and every hook in the file must be covered so no `Hooks need review`
    /// prompt remains.
    #[test]
    fn windows_powershell_absolute_path_hooks_are_fully_trusted_with_codex_identity() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let windows_bin = r"C:\Users\akiojin\AppData\Local\Programs\GWT\gwtd.exe";
        let powershell_command = |event: &str| {
            codex_event_hook_commands_with_bin(windows_bin, event)
                .into_iter()
                .nth(1)
                .expect("PowerShell generated command")
        };
        let mut hooks = serde_json::Map::new();
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
        ] {
            hooks.insert(
                event.to_string(),
                json!([
                    {
                        "matcher": "*",
                        "hooks": [{ "command": powershell_command(event), "type": "command" }]
                    }
                ]),
            );
        }
        hooks.insert(
            "Stop".to_string(),
            json!([
                {
                    "matcher": "*",
                    "hooks": [{ "command": powershell_command("Stop"), "type": "command" }]
                },
                self_improvement_stop_group()
            ]),
        );
        fs::write(
            codex_dir.join("hooks.json"),
            serde_json::to_string_pretty(&json!({ "hooks": hooks })).unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries_with_expected_bin(
            dir.path(),
            Some(windows_bin),
        )
        .unwrap();

        assert_eq!(
            entries.len(),
            6,
            "PowerShell + machine-local absolute path hooks must all be trusted: {entries:?}"
        );
        let stop = entries
            .iter()
            .find(|entry| entry.key.ends_with(":stop:0:0"))
            .expect("PowerShell Stop trust entry");
        assert_eq!(
            stop.trusted_hash,
            "sha256:9d5d048e34bba9a9bc1fc4086052911570757a132a439f7d43f578359136c38a",
            "trusted hash must match the Codex identity for the PowerShell command"
        );
    }

    /// Issue #3967 AC-4: a gwt hook the pre-registration could not vouch for is
    /// reported, so the launch can fail loudly instead of dropping the agent
    /// into a human-only `Hooks need review` prompt.
    #[test]
    fn gwt_hook_left_untrusted_is_reported_as_untrusted_managed() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let mut hooks_json: Value =
            serde_json::from_str(&fs::read_to_string(&hooks_path).unwrap()).unwrap();
        hooks_json["hooks"]["Stop"][0]["hooks"][0]["command"] =
            Value::String("'/tmp/attacker/gwtd' hook event Stop".to_string());
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();
        let config_path = dir.path().join("codex-config.toml");

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(report.trusted_entries.len(), 4);
        assert_eq!(
            report.untrusted_gwt_hooks.len(),
            1,
            "the unvouched gwt hook must be reported: {report:?}"
        );
        assert!(
            report.untrusted_gwt_hooks[0].contains("stop:0:0"),
            "report must name the untrusted hook: {report:?}"
        );
    }

    /// A user's own hook is not gwt's business: Codex may prompt for it, but it
    /// must never be reported as a gwt pre-registration failure.
    #[test]
    fn user_hook_is_not_reported_as_untrusted_managed() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let mut hooks_json: Value =
            serde_json::from_str(&fs::read_to_string(&hooks_path).unwrap()).unwrap();
        hooks_json["hooks"]["PreToolUse"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "matcher": "Bash",
                "hooks": [{ "command": "echo user-hook", "type": "command" }]
            }));
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();
        let config_path = dir.path().join("codex-config.toml");

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(report.trusted_entries.len(), 5);
        assert!(
            report.untrusted_gwt_hooks.is_empty(),
            "user hooks must not be reported as gwt trust failures: {report:?}"
        );
    }

    /// A user hook that merely quotes a gwt transport token is still a user
    /// hook. Reporting it would abort every Codex launch in the worktree over
    /// text gwt does not own.
    #[test]
    fn user_hook_quoting_a_gwt_transport_token_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let mut hooks_json: Value =
            serde_json::from_str(&fs::read_to_string(&hooks_path).unwrap()).unwrap();
        hooks_json["hooks"]["Stop"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "matcher": "*",
                "hooks": [
                    { "command": "echo ' hook event '", "type": "command" }
                ]
            }));
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();
        let config_path = dir.path().join("codex-config.toml");

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(report.trusted_entries.len(), 5);
        assert!(
            report.untrusted_gwt_hooks.is_empty(),
            "quoting a transport token must not make a user hook gwt's problem: {report:?}"
        );
    }

    /// The PowerShell dispatch form invokes the binary through `& $gwtBin`, so
    /// the token preceding the transport marker is the variable, not a path.
    #[test]
    fn powershell_dispatch_form_is_recognised_as_a_gwt_transport() {
        let windows_bin = r"C:\Program Files\GWT\gwtd.exe";
        let powershell_stop = codex_event_hook_commands_with_bin(windows_bin, "Stop")
            .into_iter()
            .nth(1)
            .expect("PowerShell generated command");

        assert!(
            is_gwt_hook_transport_command(&powershell_stop),
            "PowerShell dispatch must be recognised: {powershell_stop}"
        );
        assert!(is_gwt_hook_transport_command(
            "'/tmp/attacker/gwtd' hook event Stop"
        ));
        assert!(is_gwt_hook_transport_command(
            REPO_OWNED_SELF_IMPROVEMENT_STOP_COMMAND
        ));
        assert!(!is_gwt_hook_transport_command("echo ' hook event '"));
    }

    #[test]
    fn registration_preserves_unrelated_config_and_skips_user_hooks() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let hooks_content = fs::read_to_string(&hooks_path).unwrap();
        let mut hooks_json: Value = serde_json::from_str(&hooks_content).unwrap();
        hooks_json["hooks"]["PreToolUse"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "matcher": "Bash",
                "hooks": [
                    {
                        "command": "echo user-hook",
                        "type": "command"
                    }
                ]
            }));
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();

        let config_path = dir.path().join("codex-config.toml");
        fs::write(
            &config_path,
            r#"
[profiles.default]
model = "gpt-5.2"

[hooks.state."custom:pre_tool_use:0:0"]
enabled = false
"#,
        )
        .unwrap();

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(report.trusted_entries.len(), 5);
        let config = fs::read_to_string(&config_path).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        assert_eq!(
            parsed["profiles"]["default"]["model"].as_str(),
            Some("gpt-5.2")
        );
        assert_eq!(
            parsed["hooks"]["state"]["custom:pre_tool_use:0:0"]["enabled"].as_bool(),
            Some(false)
        );
        assert!(
            parsed["hooks"]["state"]["custom:pre_tool_use:0:0"]
                .get("trusted_hash")
                .is_none(),
            "unrelated hook state must not receive a trusted hash"
        );
        let hooks_path = dunce::canonicalize(&hooks_path).unwrap();
        assert!(
            parsed["hooks"]["state"]
                .get(format!("{}:pre_tool_use:1:0", hooks_path.display()))
                .is_none(),
            "user hook entry must not be trusted"
        );
    }

    /// Also Issue #4071 AC-3 (existing worktree): a regenerated, machine-local
    /// hooks file is registered under the plain absolute path Codex reads.
    #[test]
    fn registration_enables_generated_managed_hooks() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dunce::canonicalize(dir.path().join(".codex/hooks.json")).unwrap();
        let config_path = dir.path().join("codex-config.toml");

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(report.trusted_entries.len(), 5);
        assert!(
            report
                .trusted_entries
                .iter()
                .all(|entry| !entry.key.starts_with(r"\\?\")),
            "Codex keys hooks by the plain absolute path; verbatim keys are inert: {:?}",
            report.trusted_entries
        );
        let config = fs::read_to_string(&config_path).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        for event_name in [
            "session_start",
            "user_prompt_submit",
            "pre_tool_use",
            "post_tool_use",
            "stop",
        ] {
            let key = format!("{}:{event_name}:0:0", hooks_path.display());
            let state = parsed["hooks"]["state"]
                .get(&key)
                .unwrap_or_else(|| panic!("missing managed Codex hook state entry: {key}"));
            assert_eq!(
                state.get("enabled").and_then(toml::Value::as_bool),
                Some(true),
                "managed Codex hook must be enabled: {key}"
            );
            assert!(
                state["trusted_hash"].as_str().is_some(),
                "managed Codex hook must still carry trusted_hash: {key}"
            );
        }
    }

    #[test]
    fn registration_preserves_explicit_managed_hook_opt_out() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dunce::canonicalize(dir.path().join(".codex/hooks.json")).unwrap();
        let pre_tool_key = format!("{}:pre_tool_use:0:0", hooks_path.display());
        let pre_tool_key_toml = pre_tool_key.replace('\\', "\\\\").replace('"', "\\\"");
        let config_path = dir.path().join("codex-config.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[hooks.state."{pre_tool_key_toml}"]
enabled = false
"#
            ),
        )
        .unwrap();

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(report.trusted_entries.len(), 5);
        let config = fs::read_to_string(&config_path).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        let state = &parsed["hooks"]["state"][&pre_tool_key];
        assert_eq!(
            state["enabled"].as_bool(),
            Some(false),
            "explicit managed hook opt-out must not be overwritten"
        );
        assert!(
            state["trusted_hash"].as_str().is_some(),
            "explicitly disabled managed hook should still receive the current trusted hash"
        );
    }

    #[test]
    fn registration_does_not_enable_user_or_modified_hooks() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let hooks_content = fs::read_to_string(&hooks_path).unwrap();
        let mut hooks_json: Value = serde_json::from_str(&hooks_content).unwrap();
        hooks_json["hooks"]["PreToolUse"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "matcher": "Bash",
                "hooks": [
                    {
                        "command": "echo user-hook",
                        "type": "command"
                    }
                ]
            }));
        hooks_json["hooks"]["Stop"][0]["hooks"][0]["command"] =
            Value::String("'gwtd' hook event Stop --unexpected".to_string());
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();
        let hooks_path = dunce::canonicalize(&hooks_path).unwrap();
        let config_path = dir.path().join("codex-config.toml");

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert_eq!(
            report.trusted_entries.len(),
            4,
            "only unchanged generated hooks should be trusted"
        );
        let config = fs::read_to_string(&config_path).unwrap();
        let parsed: toml::Value = toml::from_str(&config).unwrap();
        assert!(
            parsed["hooks"]["state"]
                .get(format!("{}:pre_tool_use:1:0", hooks_path.display()))
                .is_none(),
            "user hook entry must not be enabled or trusted"
        );
        assert!(
            parsed["hooks"]["state"]
                .get(format!("{}:stop:0:0", hooks_path.display()))
                .is_none(),
            "modified generated hook must not be enabled or trusted"
        );
    }

    #[test]
    fn modified_gwt_command_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let hooks_content = fs::read_to_string(&hooks_path).unwrap();
        let mut hooks_json: Value = serde_json::from_str(&hooks_content).unwrap();
        hooks_json["hooks"]["Stop"][0]["hooks"][0]["command"] =
            Value::String("'gwtd' hook event Stop --unexpected".to_string());
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries(dir.path()).unwrap();

        assert_eq!(
            entries.len(),
            4,
            "modified gwt command must be left for Codex /hooks review"
        );
        assert!(
            entries.iter().all(|entry| !entry.key.contains(":stop:")),
            "modified Stop hook must not be trusted; got {entries:?}"
        );
    }

    #[test]
    fn gwt_command_with_modified_binary_path_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        let hooks_content = fs::read_to_string(&hooks_path).unwrap();
        let mut hooks_json: Value = serde_json::from_str(&hooks_content).unwrap();
        hooks_json["hooks"]["Stop"][0]["hooks"][0]["command"] =
            Value::String("'/tmp/gwtd' hook event Stop".to_string());
        fs::write(
            &hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();

        let entries = collect_codex_managed_hook_trust_entries(dir.path()).unwrap();

        assert_eq!(
            entries.len(),
            4,
            "path-modified gwt command must be left for Codex /hooks review"
        );
        assert!(
            entries.iter().all(|entry| !entry.key.contains(":stop:")),
            "path-modified Stop hook must not be trusted; got {entries:?}"
        );
    }

    /// The five managed hooks exactly as a repository that tracks
    /// `.codex/hooks.json` commits them: the canonical portable fallback in the
    /// POSIX shape (#3567). A fresh linked worktree starts with these bytes.
    fn tracked_canonical_managed_hooks_json() -> String {
        let mut hooks = serde_json::Map::new();
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
        ] {
            hooks.insert(
                event.to_string(),
                json!([
                    {
                        "matcher": "*",
                        "hooks": [
                            {
                                "command": codex_event_hook_commands_with_bin(
                                    crate::CANONICAL_HOOK_BIN,
                                    event,
                                )
                                .into_iter()
                                .next()
                                .unwrap(),
                                "type": "command"
                            }
                        ]
                    }
                ]),
            );
        }
        serde_json::to_string_pretty(&json!({ "hooks": hooks })).unwrap()
    }

    fn git(cwd: &std::path::Path, args: &[&str]) {
        let status = gwt_core::process::hidden_command("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .status()
            .unwrap_or_else(|error| panic!("git {args:?} failed to start: {error}"));
        assert!(status.success(), "git {args:?} failed with {status}");
    }

    /// Issue #4071 AC-1 / AC-3: the exact shape of a Monitor launch into a fresh
    /// linked worktree. The launch refreshes only the workspace-home copy for a
    /// current Codex, so the worktree-local `.codex/hooks.json` is still the
    /// tracked canonical bytes git checked out — and every one of its hooks must
    /// be registered under the key Codex itself derives from the hooks path:
    /// the plain absolute path, never the `\\?\` verbatim form that
    /// `std::fs::canonicalize` produces on Windows. gwt 9.91.0 left all five
    /// untrusted (and, with nothing trusted, wrote nothing at all).
    #[test]
    fn fresh_linked_worktree_launch_trusts_tracked_hooks_under_codex_readable_keys() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(repo.join(".codex")).unwrap();
        fs::write(
            repo.join(".codex/hooks.json"),
            tracked_canonical_managed_hooks_json(),
        )
        .unwrap();
        git(&repo, &["init", "-q", "--initial-branch=develop"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "hooks"]);
        let worktree = dir.path().join("work").join("issue-4071");
        fs::create_dir_all(worktree.parent().unwrap()).unwrap();
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "work/issue-4071",
                worktree.to_str().unwrap(),
                "develop",
            ],
        );
        generate_codex_hooks_for_mode(&worktree, CodexHookDiscoveryMode::WorkspaceHome).unwrap();
        let config_path = dir.path().join("codex-config.toml");

        let report = register_codex_managed_hook_trust_for_mode(
            &worktree,
            &config_path,
            CodexHookDiscoveryMode::Both,
        )
        .unwrap();

        assert!(
            report.untrusted_gwt_hooks.is_empty(),
            "a fresh worktree's tracked canonical hooks must be trusted: {report:?}"
        );
        let worktree_hooks_path = dunce::canonicalize(worktree.join(".codex/hooks.json")).unwrap();
        let parsed: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        for event_name in [
            "session_start",
            "user_prompt_submit",
            "pre_tool_use",
            "post_tool_use",
            "stop",
        ] {
            let key = format!("{}:{event_name}:0:0", worktree_hooks_path.display());
            let state = parsed["hooks"]["state"].get(&key).unwrap_or_else(|| {
                panic!(
                    "missing Codex-readable trust key {key}; state keys: {:?}",
                    parsed["hooks"]["state"]
                        .as_table()
                        .map(|table| table.keys().collect::<Vec<_>>())
                )
            });
            assert!(
                state["trusted_hash"].as_str().is_some(),
                "fresh worktree hook must carry trusted_hash: {key}"
            );
        }
        assert!(
            report
                .trusted_entries
                .iter()
                .all(|entry| !entry.key.starts_with(r"\\?\")),
            "Codex keys hooks by the plain absolute path; verbatim keys are inert: {:?}",
            report.trusted_entries
        );
    }

    fn tamper_managed_hook_commands(hooks_path: &std::path::Path, events: &[&str]) {
        let mut hooks_json: Value =
            serde_json::from_str(&fs::read_to_string(hooks_path).unwrap()).unwrap();
        for event in events {
            hooks_json["hooks"][*event][0]["hooks"][0]["command"] =
                Value::String(format!("'/tmp/attacker/gwtd' hook event {event}"));
        }
        fs::write(
            hooks_path,
            serde_json::to_string_pretty(&hooks_json).unwrap(),
        )
        .unwrap();
    }

    /// Issue #4071 AC-2: the launch failure must say whether trust state was
    /// written at all. gwt 9.91.0 reported "trust is incomplete" for five
    /// hooks while the config had never been touched, and the report read as
    /// if registration had not run.
    #[test]
    fn hooks_need_review_reason_separates_skipped_registration_from_hash_mismatch() {
        // Every gwt hook mismatches: nothing can be vouched for, nothing is written.
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        tamper_managed_hook_commands(
            &hooks_path,
            &[
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "Stop",
            ],
        );
        let config_path = dir.path().join("codex-config.toml");

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert!(
            !report.wrote_trust_state,
            "nothing trusted, nothing written: {report:?}"
        );
        assert!(
            !config_path.exists(),
            "config must stay untouched when nothing is trusted"
        );
        let reason = report
            .hooks_need_review_reason()
            .expect("untrusted gwt hooks must produce a launch-blocking reason");
        assert!(
            reason.contains("Hooks need review") && reason.contains("wrote no trust entry"),
            "reason must say registration was skipped: {reason}"
        );
        let key_source = dunce::canonicalize(&hooks_path).unwrap();
        let expected_bin = crate::settings_local::managed_hook_bin_for_config_path(&hooks_path);
        assert!(
            reason.contains(&format!("{} => `{expected_bin}`", key_source.display())),
            "reason must name the fallback binary each hooks file was compared against: {reason}"
        );

        // One gwt hook mismatches: four entries are written, the fifth is a hash mismatch.
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let hooks_path = dir.path().join(".codex/hooks.json");
        tamper_managed_hook_commands(&hooks_path, &["Stop"]);
        let config_path = dir.path().join("codex-config.toml");

        let report = register_codex_managed_hook_trust(dir.path(), &config_path).unwrap();

        assert!(
            report.wrote_trust_state,
            "four trusted hooks must be written: {report:?}"
        );
        let reason = report
            .hooks_need_review_reason()
            .expect("one untrusted gwt hook must still block the launch");
        assert!(
            reason.contains("wrote 4 trusted entries")
                && reason.contains("trusted_hash mismatch")
                && reason.contains(":stop:0:0")
                // Issue #3967: quoting the command is what lets a recurrence be
                // diagnosed from the failure record instead of the machine.
                && reason.contains("First untrusted command: `'/tmp/attacker/gwtd' hook event Stop`"),
            "reason must separate a hash mismatch from a skipped registration: {reason}"
        );

        // Everything trusted: no reason, the launch proceeds.
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        let report =
            register_codex_managed_hook_trust(dir.path(), &dir.path().join("codex-config.toml"))
                .unwrap();
        assert_eq!(report.hooks_need_review_reason(), None, "{report:?}");
    }

    /// A worktree whose managed hooks are already generated, ready to register
    /// against a shared Codex config.
    fn worktree_with_generated_hooks() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        generate_codex_hooks(dir.path()).unwrap();
        dir
    }

    fn trust_state(config_path: &Path) -> toml::Table {
        let parsed: toml::Value =
            toml::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();
        parsed["hooks"]["state"].as_table().unwrap().clone()
    }

    /// Issue #4071: `$CODEX_HOME/config.toml` is one file shared by every Codex
    /// agent on the machine, and registration is a read-modify-write. Without
    /// serialization two concurrent launches both read the pre-write file and
    /// the later writer drops the earlier one's entries — the launch then fails
    /// on `Hooks need review` for a worktree gwt had just registered.
    #[test]
    fn concurrent_registrations_keep_every_worktree_entry() {
        const WORKTREES: usize = 8;

        let shared = tempfile::tempdir().unwrap();
        let config_path = shared.path().join("config.toml");
        // A shared config is never empty in practice: other projects' trust
        // state is what a lost update destroys.
        fs::write(
            &config_path,
            r#"model = "gpt-6-astra"

[hooks.state."/Workbench/160-Idina/.codex/hooks.json:post_tool_use:0:0"]
enabled = true
trusted_hash = "sha256:08adeab2"
"#,
        )
        .unwrap();

        let worktrees: Vec<_> = (0..WORKTREES)
            .map(|_| worktree_with_generated_hooks())
            .collect();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(WORKTREES));

        std::thread::scope(|scope| {
            for worktree in &worktrees {
                let barrier = std::sync::Arc::clone(&barrier);
                let config_path = config_path.clone();
                let worktree = worktree.path().to_path_buf();
                scope.spawn(move || {
                    barrier.wait();
                    register_codex_managed_hook_trust(&worktree, &config_path).unwrap();
                });
            }
        });

        let state = trust_state(&config_path);
        for worktree in &worktrees {
            let hooks_path =
                dunce::canonicalize(worktree.path().join(".codex/hooks.json")).unwrap();
            for event_name in [
                "session_start",
                "user_prompt_submit",
                "pre_tool_use",
                "post_tool_use",
                "stop",
            ] {
                let key = format!("{}:{event_name}:0:0", hooks_path.display());
                let entry = state.get(&key).unwrap_or_else(|| {
                    panic!(
                        "concurrent registration lost a trust entry: {key}\nstate keys: {:?}",
                        state.keys().collect::<Vec<_>>()
                    )
                });
                assert_eq!(
                    entry.get("enabled").and_then(toml::Value::as_bool),
                    Some(true),
                    "concurrent registration must keep the entry enabled: {key}"
                );
            }
        }
        assert!(
            state.contains_key("/Workbench/160-Idina/.codex/hooks.json:post_tool_use:0:0"),
            "another project's trust entry must survive concurrent registration"
        );
    }

    /// Issue #4071: the same lost update, made deterministic. Another writer
    /// holds the shared config across its own read-modify-write; a registration
    /// that starts while it is held must publish on top of that writer's
    /// result, not on the bytes it read before.
    #[test]
    fn registration_waits_for_another_writer_and_keeps_its_entry() {
        const FOREIGN_KEY: &str = "/Workbench/160-Idina/.codex/hooks.json:stop:0:0";

        let worktree = worktree_with_generated_hooks();
        let config_path = worktree.path().join("codex-config.toml");
        fs::write(&config_path, "model = \"gpt-6-astra\"\n").unwrap();

        let holder_has_lock = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                with_codex_config_lock(&config_path, || {
                    holder_has_lock.wait();
                    // Long enough for an unserialized registration to read the
                    // pre-write file and publish over this entry.
                    std::thread::sleep(Duration::from_millis(300));
                    let mut root = read_codex_config(&config_path)?;
                    let root_table = root.as_table_mut().unwrap();
                    let hooks_table = ensure_child_table(root_table, "hooks")?;
                    let state_table = ensure_child_table(hooks_table, "state")?;
                    let entry = ensure_child_table(state_table, FOREIGN_KEY)?;
                    entry.insert("enabled".to_string(), toml::Value::Boolean(true));
                    let rendered = toml::to_string_pretty(&root).unwrap();
                    write_text_atomically(&config_path, &rendered)
                })
                .unwrap();
            });

            holder_has_lock.wait();
            register_codex_managed_hook_trust(worktree.path(), &config_path).unwrap();
        });

        let state = trust_state(&config_path);
        assert!(
            state.contains_key(FOREIGN_KEY),
            "the concurrent writer's entry was lost: {:?}",
            state.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            state.len(),
            6,
            "both writers' entries must survive: {:?}",
            state.keys().collect::<Vec<_>>()
        );
    }

    /// Issue #4071: the shared config also carries other projects' trust state,
    /// manually added entries and top-level Codex settings. Registration is a
    /// partial update — everything it does not own comes back unchanged.
    #[test]
    fn registration_preserves_other_projects_state_and_top_level_settings() {
        let worktree = worktree_with_generated_hooks();
        let config_path = worktree.path().join("codex-config.toml");
        let before = r#"model = "gpt-6-astra"
model_reasoning_effort = "medium"

[features]
web_search = true

[hooks.state."/Workbench/160-Idina/.codex/hooks.json:post_tool_use:0:0"]
enabled = true
trusted_hash = "sha256:08adeab2"

[hooks.state."/Workbench/event-magazine/work/issue-2/.codex/hooks.json:stop:0:0"]
trusted_hash = "sha256:legacy-without-enabled"

[model_providers.gwt-anthropic]
name = "Anthropic"
base_url = "http://127.0.0.1:1234/v1"

[projects."/Workbench/gwt/develop"]
trust_level = "trusted"
"#;
        fs::write(&config_path, before).unwrap();
        let before: toml::Value = toml::from_str(before).unwrap();

        register_codex_managed_hook_trust(worktree.path(), &config_path).unwrap();

        let after: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        for key in [
            "model",
            "model_reasoning_effort",
            "model_providers",
            "projects",
            "features",
        ] {
            assert_eq!(
                after.get(key),
                before.get(key),
                "registration must not disturb the shared config's `{key}`"
            );
        }
        for foreign_key in [
            "/Workbench/160-Idina/.codex/hooks.json:post_tool_use:0:0",
            "/Workbench/event-magazine/work/issue-2/.codex/hooks.json:stop:0:0",
        ] {
            assert_eq!(
                after["hooks"]["state"].get(foreign_key),
                before["hooks"]["state"].get(foreign_key),
                "another project's trust entry must survive verbatim: {foreign_key}"
            );
        }
    }

    /// Issue #4071: a worktree is launched many times. The second registration
    /// must add nothing and drop nothing — same entries, `enabled` intact.
    #[test]
    fn repeated_registration_is_idempotent() {
        let worktree = worktree_with_generated_hooks();
        let config_path = worktree.path().join("codex-config.toml");

        register_codex_managed_hook_trust(worktree.path(), &config_path).unwrap();
        let first = fs::read_to_string(&config_path).unwrap();
        let first_state = trust_state(&config_path);

        register_codex_managed_hook_trust(worktree.path(), &config_path).unwrap();

        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            first,
            "a repeated registration must not change the shared config"
        );
        assert_eq!(first_state.len(), 5);
        assert_eq!(trust_state(&config_path), first_state);
    }
}
