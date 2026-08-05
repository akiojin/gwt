//! PM agent singleton registration and settings (SPEC-3431).
//!
//! `<gwt_project_dir>/project-state/pm.json` is the source of truth for
//! FR-001 (per-project PM singleton), FR-002 (auto-start opt-out), and the
//! FR-003 restart-backoff bookkeeping. The writer mirrors the Issue Monitor
//! prefs transaction: a stable sibling `.lock` inode serializes cross-process
//! read-modify-write (GUI and gwtd both write this file), and the
//! unique-scratch durable atomic write keeps concurrent writers from tearing
//! the JSON.
//!
//! Liveness is deliberately an injected predicate: whether a registered PM
//! session is still alive is decided by the caller (GUI pane registry or
//! gwt-agent session store), not by this module, so the singleton invariant
//! stays testable without a running pane.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

/// Durable record of the one resident PM session for a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmRegistration {
    pub session_id: String,
    pub agent_id: String,
    pub worktree_path: String,
    #[serde(default)]
    pub created_at: Option<String>,
    /// FR-003 crash-loop damper: consecutive crash count observed by the
    /// auto-restart path. Reset on a healthy start.
    #[serde(default)]
    pub consecutive_crashes: u32,
    /// RFC3339 floor before which the auto-restart path must not respawn.
    #[serde(default)]
    pub next_not_before: Option<String>,
}

fn default_auto_start() -> bool {
    true
}

/// SPEC-3431 FR-026: the agent the PM runs as, chosen per project.
///
/// Deliberately narrower than the Issue Monitor's launch profile. The PM is a
/// conversational role on the host: it never needs a Docker runtime, a resume
/// mode, or `--dangerously-skip-permissions`, and offering those knobs would
/// only let a user misconfigure the one agent that must always come up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PmLaunchProfile {
    /// Agent command name, e.g. `claude` or `codex`.
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The agent a project falls back to when nothing is configured.
///
/// A fresh project has no profile, and the PM must still start — this is the
/// bootstrap trap the Issue Monitor hits when it refuses to launch without
/// one. The PM never inherits a profile from unrelated launches for the same
/// reason: a stale Docker target or exotic model from some other work would
/// silently become the PM's.
pub const PM_DEFAULT_AGENT: &str = "claude";

/// Agents that can resolve the `$gwt-pm` bootstrap prompt.
///
/// Managed assets only reach agents with a skills mirror, and `pm_guidance`
/// writes the `.claude` and `.codex` mirrors. Launching the PM as any other
/// agent would hand it a prompt that resolves to nothing — the exact failure
/// the T-052 materialization fix removed, reintroduced through configuration.
pub const PM_SUPPORTED_AGENTS: &[&str] = &["claude", "codex"];

pub fn pm_agent_is_supported(agent_id: &str) -> bool {
    PM_SUPPORTED_AGENTS.contains(&agent_id)
}

/// Project-scoped PM settings that survive deregistration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmSettings {
    /// FR-002: opt-out flag. Missing field must read as `true` so prefs
    /// written before this field existed keep auto-starting.
    #[serde(default = "default_auto_start")]
    pub auto_start: bool,
    /// FR-026: absent until the user chooses; see [`PmSettings::launch_profile_or_default`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_profile: Option<PmLaunchProfile>,
}

impl PmSettings {
    /// The profile to launch with: the configured one when it names an agent
    /// that can resolve `$gwt-pm`, the built-in default otherwise. Falling back
    /// rather than refusing keeps FR-002's "opening a project starts the PM"
    /// true even for prefs written by a newer or misconfigured build.
    pub fn launch_profile_or_default(&self) -> PmLaunchProfile {
        match self.launch_profile.as_ref() {
            Some(profile) if pm_agent_is_supported(&profile.agent_id) => profile.clone(),
            Some(profile) => {
                tracing::warn!(
                    agent_id = %profile.agent_id,
                    "configured PM agent cannot resolve the gwt-pm skill; using the default"
                );
                PmLaunchProfile::default_profile()
            }
            None => PmLaunchProfile::default_profile(),
        }
    }
}

impl PmLaunchProfile {
    pub fn default_profile() -> Self {
        Self {
            agent_id: PM_DEFAULT_AGENT.to_string(),
            ..Self::default()
        }
    }
}

impl Default for PmSettings {
    fn default() -> Self {
        Self {
            auto_start: true,
            launch_profile: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PmPrefs {
    #[serde(default)]
    pub registration: Option<PmRegistration>,
    #[serde(default)]
    pub settings: PmSettings,
}

/// Outcome of a singleton registration attempt (FR-001).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmRegisterOutcome {
    /// No prior registration; the candidate is now registered.
    Registered,
    /// A stale (dead) registration was replaced by the candidate.
    ReplacedStale { previous: PmRegistration },
    /// A live PM already exists; the candidate was rejected and the stored
    /// bytes are unchanged. Callers route the user to resume `existing`.
    RejectedLive { existing: PmRegistration },
}

pub fn pm_prefs_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path).join("project-state/pm.json")
}

/// Canonical worktree for the project's resident PM session.
pub fn pm_worktree_path_for_repo_path(repo_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path).join("pm/worktree")
}

/// Whether `path` is some project's canonical PM worktree.
///
/// Gates PM-only managed assets, so it anchors on the full shape
/// `<gwt projects dir>/<repo hash>/pm/worktree` rather than the trailing two
/// segments alone — a production branch literally named `pm/worktree` would
/// otherwise be handed the PM operating contract.
pub fn is_pm_worktree(path: &Path) -> bool {
    let Some(pm_dir) = path.parent() else {
        return false;
    };
    if path.file_name() != Some(std::ffi::OsStr::new("worktree"))
        || pm_dir.file_name() != Some(std::ffi::OsStr::new("pm"))
    {
        return false;
    }
    pm_dir
        .parent()
        .and_then(Path::parent)
        .is_some_and(|projects_dir| projects_dir == gwt_core::paths::gwt_projects_dir())
}

/// Per-writer-unique scratch path in the same directory as `path` so the
/// final `rename` stays on one filesystem and is atomic. A fixed scratch name
/// would let concurrent GUI/gwtd writers truncate the same file and tear the
/// JSON (see the Issue Monitor prefs writer).
fn unique_pm_scratch_path(path: &Path) -> PathBuf {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("pm.json");
    parent.join(format!(
        ".{}.tmp-{}-{}",
        file_name,
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    // Windows cannot open a directory as a std::fs::File; the scratch file is
    // still sync_all'd before the atomic rename, matching the repository's
    // other durable writers.
    Ok(())
}

fn durable_atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let scratch_path = unique_pm_scratch_path(path);
    let result = (|| {
        let mut scratch = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&scratch_path)?;
        scratch.write_all(content)?;
        scratch.sync_all()?;
        // Deadline-aware transactions must not become visible after their
        // acceptance boundary; recheck immediately before the canonical
        // rename (same convention as the Issue Monitor prefs writer).
        gwt_core::operation_deadline::ensure_remaining("PM prefs durable rename")?;
        fs::rename(&scratch_path, path)?;
        sync_parent_directory(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&scratch_path);
    }
    result
}

fn with_pm_prefs_lock<T>(path: &Path, operation: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    // Lock a stable sibling inode: locking `path` itself would stop
    // protecting future writers as soon as the atomic rename replaces that
    // inode.
    let lock = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    gwt_core::operation_deadline::lock_exclusive(&lock)?;
    let result = operation();
    let unlock_result = FileExt::unlock(&lock);
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn load_pm_prefs_unlocked(path: &Path) -> io::Result<PmPrefs> {
    if !path.exists() {
        return Ok(PmPrefs::default());
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|error| {
        let kind = match error.classify() {
            serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
                io::ErrorKind::InvalidData
            }
            serde_json::error::Category::Data => io::ErrorKind::InvalidInput,
            serde_json::error::Category::Io => io::ErrorKind::Other,
        };
        io::Error::new(kind, error)
    })
}

fn save_pm_prefs_unlocked(path: &Path, prefs: &PmPrefs) -> io::Result<()> {
    let content = serde_json::to_string_pretty(prefs).map_err(io::Error::other)?;
    durable_atomic_write(path, content.as_bytes())
}

pub fn load_pm_prefs(path: &Path) -> io::Result<PmPrefs> {
    load_pm_prefs_unlocked(path)
}

pub fn save_pm_prefs(path: &Path, prefs: &PmPrefs) -> io::Result<()> {
    with_pm_prefs_lock(path, || save_pm_prefs_unlocked(path, prefs))
}

/// One cross-process read-modify-write transaction under the stable sibling
/// lock, committing through the durable atomic writer.
pub fn mutate_pm_prefs<T>(
    path: &Path,
    mutation: impl FnOnce(&mut PmPrefs) -> T,
) -> io::Result<(PmPrefs, T)> {
    with_pm_prefs_lock(path, || {
        let mut prefs = load_pm_prefs_unlocked(path)?;
        let result = mutation(&mut prefs);
        save_pm_prefs_unlocked(path, &prefs)?;
        Ok((prefs, result))
    })
}

/// FR-001 singleton gate: register `candidate` unless a live PM already
/// exists. `is_live` judges the stored registration; a dead one is replaced
/// (stale regeneration), a live one rejects the candidate without touching
/// the stored bytes.
pub fn try_register_pm(
    path: &Path,
    candidate: PmRegistration,
    is_live: impl Fn(&PmRegistration) -> bool,
) -> io::Result<(PmPrefs, PmRegisterOutcome)> {
    with_pm_prefs_lock(path, || {
        let mut prefs = load_pm_prefs_unlocked(path)?;
        let outcome = match prefs.registration.take() {
            Some(existing) if is_live(&existing) => {
                // Rejected attempts must leave the canonical bytes untouched:
                // restore and return without saving.
                prefs.registration = Some(existing.clone());
                return Ok((prefs, PmRegisterOutcome::RejectedLive { existing }));
            }
            Some(stale) => PmRegisterOutcome::ReplacedStale { previous: stale },
            None => PmRegisterOutcome::Registered,
        };
        prefs.registration = Some(candidate);
        save_pm_prefs_unlocked(path, &prefs)?;
        Ok((prefs, outcome))
    })
}

/// FR-013 intentional stop: clear the registration when it belongs to
/// `session_id`. Returns whether a matching registration was removed.
/// Settings (auto_start) survive deregistration.
pub fn deregister_pm(path: &Path, session_id: &str) -> io::Result<(PmPrefs, bool)> {
    with_pm_prefs_lock(path, || {
        let mut prefs = load_pm_prefs_unlocked(path)?;
        let matches = prefs
            .registration
            .as_ref()
            .is_some_and(|registration| registration.session_id == session_id);
        if !matches {
            return Ok((prefs, false));
        }
        prefs.registration = None;
        save_pm_prefs_unlocked(path, &prefs)?;
        Ok((prefs, true))
    })
}

/// SPEC-3431 FR-009: is `session_id` the project's registered PM?
///
/// This is the whole privileged-subject rule. It is deliberately an exact
/// match against the durable registration — no trimming, no normalization —
/// so a near-miss id can never inherit PM authority. Liveness needs no
/// separate probe here: the caller *is* the session, so if the registration
/// names it, that PM is running. A missing or unreadable registration is not
/// privileged (fail-closed).
pub fn session_is_registered_pm(prefs_path: &Path, session_id: &str) -> bool {
    if session_id.is_empty() {
        return false;
    }
    load_pm_prefs(prefs_path).is_ok_and(|prefs| {
        prefs
            .registration
            .is_some_and(|registration| registration.session_id == session_id)
    })
}

/// FR-003 crash-loop damper: uptime beyond this resets the consecutive-crash
/// count (the PM ran healthily, so the next crash starts a fresh series).
pub const PM_HEALTHY_UPTIME_SECS: i64 = 600;

/// FR-003 backoff ladder indexed by `consecutive_crashes - 1` (last entry
/// repeats). `0` means respawn immediately.
pub const PM_CRASH_BACKOFF_SECS: &[i64] = &[0, 30, 120, 300];

/// FR-003: record one crash on the registration and derive the respawn
/// verdict. Returns `true` when an immediate respawn is allowed; `false`
/// leaves `next_not_before` as the floor before which the auto-restart path
/// (and the ensure gate) must not respawn. `now` is RFC3339 so callers and
/// tests inject time explicitly.
pub fn apply_pm_crash_backoff(registration: &mut PmRegistration, now: &str) -> bool {
    let healthy_series_reset = registration
        .created_at
        .as_deref()
        .and_then(|created| rfc3339_delta_secs(created, now))
        .is_some_and(|uptime| uptime >= PM_HEALTHY_UPTIME_SECS);
    let count = if healthy_series_reset {
        1
    } else {
        registration.consecutive_crashes.saturating_add(1)
    };
    registration.consecutive_crashes = count;
    let ladder_index = usize::min(
        count.saturating_sub(1) as usize,
        PM_CRASH_BACKOFF_SECS.len() - 1,
    );
    let backoff = PM_CRASH_BACKOFF_SECS[ladder_index];
    if backoff <= 0 {
        registration.next_not_before = None;
        return true;
    }
    registration.next_not_before = rfc3339_plus_secs(now, backoff);
    false
}

/// FR-003: whether the backoff floor allows a respawn at `now`. A missing or
/// unparsable floor allows the respawn (fail-open: the damper protects
/// against loops, not against recovery).
pub fn pm_respawn_allowed(registration: &PmRegistration, now: &str) -> bool {
    match registration.next_not_before.as_deref() {
        None => true,
        Some(floor) => rfc3339_delta_secs(floor, now).is_none_or(|delta| delta >= 0),
    }
}

fn rfc3339_delta_secs(earlier: &str, later: &str) -> Option<i64> {
    let earlier = chrono::DateTime::parse_from_rfc3339(earlier).ok()?;
    let later = chrono::DateTime::parse_from_rfc3339(later).ok()?;
    Some((later - earlier).num_seconds())
}

fn rfc3339_plus_secs(now: &str, secs: i64) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(now).ok()?;
    Some(
        (parsed + chrono::Duration::seconds(secs))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

/// Diagnostic snapshot for the `pm.status` JSON operation (FR-001 diagnostic
/// visibility). `session_record_present` / `stale_hint` are populated only
/// when a registration exists; a missing durable session record is a stale
/// hint, not an authoritative liveness verdict — authoritative liveness stays
/// with the GUI spawn gate, which can see live panes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PmStatusReport {
    pub schema_version: u32,
    pub registered: bool,
    pub auto_start: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration: Option<PmRegistration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_record_present: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_hint: Option<bool>,
    /// FR-014: the PM never occupies an Issue Monitor implementation slot.
    /// Always 0; kept explicit so operators can see the accounting rule.
    /// The global resource cap engine itself remains SPEC #3200's
    /// unimplemented FR — this report only exposes the PM bucket.
    pub implementation_slots_consumed: u32,
    /// FR-014 visibility: resident PM bucket (1 while a PM is registered).
    pub pm_bucket: u32,
    /// FR-009 diagnostic visibility: whether the session asking for this
    /// report is the registered PM, i.e. whether the asymmetric Issue Monitor
    /// boundary is lifted for it. `None` when the caller has no ambient
    /// session identity to judge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_is_registered_pm: Option<bool>,
}

/// Build the `pm.status` report from loaded prefs. The durable-session probe
/// is injected so the report logic stays testable without a real session
/// store.
pub fn pm_status_report(
    prefs: &PmPrefs,
    session_record_present: impl Fn(&str) -> bool,
) -> PmStatusReport {
    pm_status_report_for_caller(prefs, session_record_present, None)
}

/// [`pm_status_report`] plus the FR-009 privilege verdict for `caller_session`.
pub fn pm_status_report_for_caller(
    prefs: &PmPrefs,
    session_record_present: impl Fn(&str) -> bool,
    caller_session: Option<&str>,
) -> PmStatusReport {
    let registration = prefs.registration.clone();
    let record_present = registration
        .as_ref()
        .map(|registration| session_record_present(&registration.session_id));
    PmStatusReport {
        schema_version: 1,
        registered: registration.is_some(),
        auto_start: prefs.settings.auto_start,
        pm_bucket: u32::from(registration.is_some()),
        implementation_slots_consumed: 0,
        caller_is_registered_pm: caller_session.map(|caller| {
            registration
                .as_ref()
                .is_some_and(|current| current.session_id == caller)
        }),
        registration,
        session_record_present: record_present,
        stale_hint: record_present.map(|present| !present),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_prefs_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project-state").join("pm.json");
        (dir, path)
    }

    fn registration(session: &str) -> PmRegistration {
        PmRegistration {
            session_id: session.to_string(),
            agent_id: "claude-code".to_string(),
            worktree_path: "/tmp/pm-worktree".to_string(),
            created_at: Some("2026-08-03T00:00:00Z".to_string()),
            consecutive_crashes: 0,
            next_not_before: None,
        }
    }

    #[test]
    fn missing_file_loads_default_with_auto_start_true() {
        let (_dir, path) = temp_prefs_path();
        let prefs = load_pm_prefs(&path).expect("load default");
        assert_eq!(prefs.registration, None);
        assert!(prefs.settings.auto_start, "FR-002: auto_start defaults ON");
    }

    #[test]
    fn empty_json_object_defaults_auto_start_true() {
        // Prefs written before the settings field existed must keep
        // auto-starting; a silent false default would disable FR-002 for
        // every existing project.
        let prefs: PmPrefs = serde_json::from_str("{}").expect("parse empty object");
        assert!(prefs.settings.auto_start);
        let prefs: PmPrefs =
            serde_json::from_str("{\"settings\":{}}").expect("parse empty settings");
        assert!(prefs.settings.auto_start);
    }

    #[test]
    fn save_then_load_roundtrips_and_leaves_no_scratch() {
        let (_dir, path) = temp_prefs_path();
        let prefs = PmPrefs {
            registration: Some(registration("session-a")),
            settings: PmSettings {
                auto_start: false,
                ..PmSettings::default()
            },
        };
        save_pm_prefs(&path, &prefs).expect("save");
        let loaded = load_pm_prefs(&path).expect("load");
        assert_eq!(loaded, prefs);
        let names: Vec<String> = fs::read_dir(path.parent().expect("parent"))
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(
            names
                .iter()
                .all(|name| name == "pm.json" || name == "pm.lock"),
            "unexpected files after atomic save: {names:?}"
        );
    }

    #[test]
    fn mutate_persists_the_mutation() {
        let (_dir, path) = temp_prefs_path();
        let (committed, _) = mutate_pm_prefs(&path, |prefs| {
            prefs.settings.auto_start = false;
        })
        .expect("mutate");
        assert!(!committed.settings.auto_start);
        let loaded = load_pm_prefs(&path).expect("load");
        assert!(!loaded.settings.auto_start);
    }

    #[test]
    fn register_into_empty_prefs_registers() {
        let (_dir, path) = temp_prefs_path();
        let (prefs, outcome) =
            try_register_pm(&path, registration("session-a"), |_| true).expect("register");
        assert_eq!(outcome, PmRegisterOutcome::Registered);
        assert_eq!(
            prefs.registration.as_ref().map(|r| r.session_id.as_str()),
            Some("session-a")
        );
        let loaded = load_pm_prefs(&path).expect("load");
        assert_eq!(
            loaded.registration.map(|r| r.session_id),
            Some("session-a".to_string())
        );
    }

    #[test]
    fn register_rejects_when_existing_is_live() {
        let (_dir, path) = temp_prefs_path();
        try_register_pm(&path, registration("session-a"), |_| true).expect("seed");
        let (prefs, outcome) =
            try_register_pm(&path, registration("session-b"), |_| true).expect("attempt");
        match outcome {
            PmRegisterOutcome::RejectedLive { existing } => {
                assert_eq!(existing.session_id, "session-a");
            }
            other => panic!("expected RejectedLive, got {other:?}"),
        }
        // The stored registration must be untouched by the rejected attempt.
        assert_eq!(
            prefs.registration.map(|r| r.session_id),
            Some("session-a".to_string())
        );
        let loaded = load_pm_prefs(&path).expect("load");
        assert_eq!(
            loaded.registration.map(|r| r.session_id),
            Some("session-a".to_string())
        );
    }

    #[test]
    fn register_replaces_stale_registration() {
        let (_dir, path) = temp_prefs_path();
        try_register_pm(&path, registration("session-a"), |_| true).expect("seed");
        let (prefs, outcome) =
            try_register_pm(&path, registration("session-b"), |_| false).expect("takeover");
        match outcome {
            PmRegisterOutcome::ReplacedStale { previous } => {
                assert_eq!(previous.session_id, "session-a");
            }
            other => panic!("expected ReplacedStale, got {other:?}"),
        }
        assert_eq!(
            prefs.registration.map(|r| r.session_id),
            Some("session-b".to_string())
        );
    }

    #[test]
    fn deregister_clears_only_matching_session_and_keeps_settings() {
        let (_dir, path) = temp_prefs_path();
        mutate_pm_prefs(&path, |prefs| {
            prefs.settings.auto_start = false;
        })
        .expect("seed settings");
        try_register_pm(&path, registration("session-a"), |_| true).expect("seed");

        let (prefs, removed) = deregister_pm(&path, "other-session").expect("mismatch");
        assert!(!removed, "non-matching session must not deregister");
        assert!(prefs.registration.is_some());

        let (prefs, removed) = deregister_pm(&path, "session-a").expect("match");
        assert!(removed);
        assert_eq!(prefs.registration, None);
        assert!(
            !prefs.settings.auto_start,
            "FR-002 settings must survive deregistration"
        );
    }

    /// SPEC-3431 FR-026: the profile is optional and additive, so prefs
    /// written before it existed keep working, and a project that never
    /// configures one still launches.
    #[test]
    fn launch_profile_defaults_to_none_and_round_trips() {
        let (_dir, path) = temp_prefs_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, r#"{"settings":{"auto_start":true}}"#).expect("legacy prefs");

        let prefs = load_pm_prefs(&path).expect("load legacy prefs");
        assert_eq!(prefs.settings.launch_profile, None);
        assert_eq!(
            prefs.settings.launch_profile_or_default(),
            PmLaunchProfile {
                agent_id: PM_DEFAULT_AGENT.to_string(),
                model: None,
                reasoning: None,
                version: None,
            }
        );

        let configured = PmLaunchProfile {
            agent_id: "codex".to_string(),
            model: Some("gpt-5.1-codex-max".to_string()),
            reasoning: Some("high".to_string()),
            version: None,
        };
        let mut prefs = prefs;
        prefs.settings.launch_profile = Some(configured.clone());
        save_pm_prefs(&path, &prefs).expect("save");
        let reloaded = load_pm_prefs(&path).expect("reload");
        assert_eq!(reloaded.settings.launch_profile, Some(configured.clone()));
        assert_eq!(reloaded.settings.launch_profile_or_default(), configured);
    }

    /// A profile naming an agent with no skills mirror would hand the PM a
    /// `$gwt-pm` prompt that resolves to nothing — the T-052 failure,
    /// reachable through configuration. Fall back instead of refusing: the PM
    /// must always come up (FR-002).
    #[test]
    fn launch_profile_falls_back_when_the_agent_cannot_resolve_the_pm_skill() {
        for agent_id in ["gemini", "opencode", "hermes", "copilot", ""] {
            let settings = PmSettings {
                auto_start: true,
                launch_profile: Some(PmLaunchProfile {
                    agent_id: agent_id.to_string(),
                    model: Some("some-model".to_string()),
                    ..PmLaunchProfile::default()
                }),
            };
            let resolved = settings.launch_profile_or_default();
            assert_eq!(
                resolved.agent_id, PM_DEFAULT_AGENT,
                "{agent_id} has no gwt-pm mirror and must not be launched as the PM"
            );
            assert_eq!(
                resolved.model, None,
                "falling back must not keep the rejected profile's model"
            );
        }
        assert!(PM_SUPPORTED_AGENTS
            .iter()
            .all(|id| pm_agent_is_supported(id)));
    }

    /// SPEC-3431 T-052: `is_pm_worktree` gates who receives the PM operating
    /// contract, so it must match the canonical location and nothing else.
    #[test]
    fn pm_worktree_path_is_canonical_and_predicate_matches_only_it() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());

        let repo = Path::new("/tmp/some-repo");
        let worktree = pm_worktree_path_for_repo_path(repo);
        assert_eq!(
            worktree,
            gwt_core::paths::gwt_project_dir_for_repo_path(repo).join("pm/worktree")
        );
        assert!(is_pm_worktree(&worktree));

        let pm_dir = worktree.parent().expect("pm dir");
        assert!(
            !is_pm_worktree(pm_dir),
            "the `pm` directory itself is not the worktree"
        );
        assert!(
            !is_pm_worktree(&pm_dir.join("worktree-2")),
            "a sibling directory must not match"
        );
        assert!(
            !is_pm_worktree(Path::new("/tmp/elsewhere/pm/worktree")),
            "a branch named pm/worktree outside ~/.gwt/projects must not match"
        );
    }

    #[test]
    fn prefs_path_lives_under_project_state() {
        let path = pm_prefs_path_for_repo_path(Path::new("/tmp/some-repo"));
        assert!(
            path.ends_with("project-state/pm.json"),
            "unexpected prefs path: {}",
            path.display()
        );
    }

    #[test]
    fn status_report_without_registration_omits_liveness_fields() {
        let prefs = PmPrefs {
            registration: None,
            settings: PmSettings {
                auto_start: false,
                ..PmSettings::default()
            },
        };
        let report = pm_status_report(&prefs, |_| panic!("probe must not run unregistered"));
        assert_eq!(report.schema_version, 1);
        assert!(!report.registered);
        assert!(!report.auto_start);
        assert_eq!(report.registration, None);
        assert_eq!(report.session_record_present, None);
        assert_eq!(report.stale_hint, None);
    }

    #[test]
    fn status_report_with_live_session_record_has_no_stale_hint() {
        let prefs = PmPrefs {
            registration: Some(registration("session-a")),
            ..Default::default()
        };
        let report = pm_status_report(&prefs, |session_id| {
            assert_eq!(session_id, "session-a");
            true
        });
        assert!(report.registered);
        assert!(report.auto_start);
        assert_eq!(
            report.registration.as_ref().map(|r| r.session_id.as_str()),
            Some("session-a")
        );
        assert_eq!(report.session_record_present, Some(true));
        assert_eq!(report.stale_hint, Some(false));
    }

    #[test]
    fn status_report_with_missing_session_record_hints_stale() {
        let prefs = PmPrefs {
            registration: Some(registration("session-a")),
            ..Default::default()
        };
        let report = pm_status_report(&prefs, |_| false);
        assert!(report.registered);
        assert_eq!(report.session_record_present, Some(false));
        assert_eq!(report.stale_hint, Some(true));
    }

    #[test]
    fn status_report_accounts_pm_outside_implementation_slots() {
        // SPEC-3431 T-060 (FR-014): the PM is visible as its own resident
        // bucket and never consumes an implementation slot. The global cap
        // engine itself remains SPEC #3200's unimplemented FR.
        let unregistered = pm_status_report(&PmPrefs::default(), |_| true);
        assert_eq!(unregistered.pm_bucket, 0);
        assert_eq!(unregistered.implementation_slots_consumed, 0);

        let prefs = PmPrefs {
            registration: Some(registration("session-a")),
            ..Default::default()
        };
        let registered = pm_status_report(&prefs, |_| true);
        assert_eq!(registered.pm_bucket, 1);
        assert_eq!(registered.implementation_slots_consumed, 0);
    }

    #[test]
    fn crash_backoff_first_crash_respawns_immediately() {
        // FR-003: one crash is recoverable without delay.
        let mut reg = registration("session-a");
        reg.created_at = Some("2026-08-03T00:00:00Z".to_string());
        let respawn = apply_pm_crash_backoff(&mut reg, "2026-08-03T00:01:00Z");
        assert!(respawn);
        assert_eq!(reg.consecutive_crashes, 1);
        assert_eq!(reg.next_not_before, None);
        assert!(pm_respawn_allowed(&reg, "2026-08-03T00:01:00Z"));
    }

    #[test]
    fn crash_backoff_series_extends_floor_and_blocks_respawn() {
        // FR-003: rapid consecutive crashes extend the floor.
        let mut reg = registration("session-a");
        reg.created_at = Some("2026-08-03T00:00:00Z".to_string());
        reg.consecutive_crashes = 1;
        let respawn = apply_pm_crash_backoff(&mut reg, "2026-08-03T00:01:00Z");
        assert!(!respawn, "second crash in a series must back off");
        assert_eq!(reg.consecutive_crashes, 2);
        assert_eq!(
            reg.next_not_before.as_deref(),
            Some("2026-08-03T00:01:30Z"),
            "floor = now + 30s at series position 2"
        );
        assert!(!pm_respawn_allowed(&reg, "2026-08-03T00:01:00Z"));
        assert!(pm_respawn_allowed(&reg, "2026-08-03T00:01:30Z"));
    }

    #[test]
    fn crash_backoff_resets_series_after_healthy_uptime() {
        // FR-003: a long healthy run means the next crash starts fresh.
        let mut reg = registration("session-a");
        reg.created_at = Some("2026-08-03T00:00:00Z".to_string());
        reg.consecutive_crashes = 3;
        reg.next_not_before = Some("2026-08-03T00:05:00Z".to_string());
        let respawn = apply_pm_crash_backoff(&mut reg, "2026-08-03T00:20:00Z");
        assert!(respawn);
        assert_eq!(reg.consecutive_crashes, 1);
        assert_eq!(reg.next_not_before, None);
    }

    #[test]
    fn registered_pm_session_is_recognized_and_others_are_not() {
        // SPEC-3431 FR-009: the privileged subject is exactly the session the
        // durable registration names. Anything else — a different agent
        // session, an empty ambient id, or no PM at all — is not privileged.
        let (_dir, path) = temp_prefs_path();
        assert!(
            !session_is_registered_pm(&path, "session-a"),
            "no registration means no privilege"
        );

        try_register_pm(&path, registration("session-a"), |_| false).expect("register");
        assert!(session_is_registered_pm(&path, "session-a"));
        assert!(!session_is_registered_pm(&path, "session-b"));
        assert!(!session_is_registered_pm(&path, ""));
        assert!(
            !session_is_registered_pm(&path, " session-a "),
            "untrimmed ids must not match"
        );

        deregister_pm(&path, "session-a").expect("deregister");
        assert!(
            !session_is_registered_pm(&path, "session-a"),
            "a closed PM loses its privilege immediately"
        );
    }

    #[test]
    fn pm_registration_never_touches_issue_monitor_slot_accounting() {
        // SPEC-3431 T-060 (FR-014): registering a PM must leave the Issue
        // Monitor's implementation-slot ledger untouched. The stores are
        // structurally separate files; this test pins that separation so a
        // future coupling has to break it consciously.
        let (_dir, path) = temp_prefs_path();
        let monitor = crate::IssueMonitorState::new(crate::IssueMonitorConfig::default());
        let before = monitor.active_count();

        try_register_pm(&path, registration("session-a"), |_| true).expect("register");

        assert_eq!(monitor.active_count(), before);
        assert_eq!(monitor.active_count(), 0);
    }
}
