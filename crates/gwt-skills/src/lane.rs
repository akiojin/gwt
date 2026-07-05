//! Lane registry and lane profiles (SPEC-3248 hooks v2, P0 Foundational).
//!
//! A **lane** is the role a launched agent session plays. SPEC-3247 introduced
//! two ([`SessionKind::Intake`] / [`SessionKind::Execution`]) and branched
//! individual hooks on the `GWT_SESSION_KIND` env at runtime. hooks v2 makes
//! the lane a first-class, N-extensible dimension: each lane declares a
//! [`LaneProfile`] (guidance variant + policy flags + — later — skill set), and
//! hooks/guidance/skills consult the profile instead of re-deriving behavior
//! per hook. Adding a lane becomes "add a profile", not "wire every hook".
//!
//! P0 introduces the registry, the profiles for the two existing lanes, and the
//! worktree lane file that is the deterministic source of truth (env stays a
//! fast-path). It intentionally does not yet rewire the hooks — that is P1–P4,
//! migrated one handler at a time (strangler-fig). The `intake`/`execution`
//! profiles here encode exactly the behavior SPEC-3247 already ships, so P0 is
//! behavior-neutral.

use std::path::{Path, PathBuf};

use crate::session_kind::SessionKind;

/// Which coordination-guidance body a lane materializes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuidanceVariant {
    /// Curate lane: no Work-state (`workspace.update`) instructions.
    Curation,
    /// Execute lane: full producing-work guidance.
    ProducingWork,
}

/// Boolean switches a [`LaneProfile`] toggles for hooks/guidance/skills.
///
/// P0 wires none of these into hooks yet; they exist so P1–P4 can consult the
/// profile instead of the ad-hoc `SessionKind::from_env()` branches SPEC-3247
/// left behind. The `execution` and `intake` defaults below reproduce the
/// current shipped behavior exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanePolicyFlags {
    /// Emit the producing-work Work-state reminders (title/progress) in
    /// board_reminder. `false` for intake (it owns no Work).
    pub emit_work_state_reminders: bool,
    /// Let the gwt self-improvement Stop gate fire. `false` for intake.
    pub self_improvement_stop: bool,
    /// Block Edit/Write to production source at PreToolUse (P4). `false` today.
    pub block_production_code_edits: bool,
    /// Nudge to register the curated Issue/SPEC before Stop (P4). `false` today.
    pub completion_gate: bool,
    /// Emit a lane-specific SessionStart onboarding导线 (P4). `false` today.
    pub sessionstart_onboarding: bool,
    /// Distribute only the curation skill subset (P4). `false` today (all
    /// skills go to every lane).
    pub reduced_skill_set: bool,
}

/// A declarative profile for one lane. Adding a lane = adding a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneProfile {
    /// Stable lane id materialized into the lane file (`intake` / `execution` /
    /// future `review` …).
    pub id: &'static str,
    /// Coordination-guidance body this lane uses.
    pub guidance_variant: GuidanceVariant,
    /// Hook/guidance/skill policy switches.
    pub policy_flags: LanePolicyFlags,
}

/// The Execute lane: full producing-work behavior (the default, and the
/// behavior every non-intake launch has today).
pub const EXECUTION_PROFILE: LaneProfile = LaneProfile {
    id: "execution",
    guidance_variant: GuidanceVariant::ProducingWork,
    policy_flags: LanePolicyFlags {
        emit_work_state_reminders: true,
        self_improvement_stop: true,
        block_production_code_edits: false,
        completion_gate: false,
        sessionstart_onboarding: false,
        reduced_skill_set: false,
    },
};

/// The Curate lane: branchless intake. Encodes exactly what SPEC-3247 already
/// ships (no Work-state reminders, no self-improvement Stop, curation
/// guidance). The P4 flags stay `false` here until their phases land.
pub const INTAKE_PROFILE: LaneProfile = LaneProfile {
    id: "intake",
    guidance_variant: GuidanceVariant::Curation,
    policy_flags: LanePolicyFlags {
        emit_work_state_reminders: false,
        self_improvement_stop: false,
        block_production_code_edits: false,
        completion_gate: false,
        sessionstart_onboarding: false,
        reduced_skill_set: false,
    },
};

/// Registry of lane profiles. N-extensible: adding a lane means adding a
/// profile constant and a `resolve` arm.
pub struct LaneRegistry;

impl LaneRegistry {
    /// The default lane when nothing else is known. Execution preserves the
    /// current producing-work behavior for old launches / existing worktrees.
    #[must_use]
    pub fn default_profile() -> &'static LaneProfile {
        &EXECUTION_PROFILE
    }

    /// Resolve a lane id to its profile. Unknown / empty ids fall back to the
    /// default (execution) — the same fail-safe as the lane file and env.
    #[must_use]
    pub fn resolve(id: &str) -> &'static LaneProfile {
        match id.trim() {
            "intake" => &INTAKE_PROFILE,
            "execution" => &EXECUTION_PROFILE,
            _ => Self::default_profile(),
        }
    }

    /// Bridge from the SPEC-3247 [`SessionKind`] enum to a lane profile, so the
    /// two representations cannot drift while hooks migrate.
    #[must_use]
    pub fn for_session_kind(kind: SessionKind) -> &'static LaneProfile {
        match kind {
            SessionKind::Intake => &INTAKE_PROFILE,
            SessionKind::Execution => &EXECUTION_PROFILE,
        }
    }
}

/// Worktree-relative path of the lane file (the deterministic source of truth).
pub const LANE_FILE_RELATIVE: &str = ".gwt/session-kind.json";

/// Current lane-file schema version.
pub const LANE_FILE_VERSION: u32 = 1;

/// Absolute lane-file path for a worktree.
#[must_use]
pub fn lane_file_path(worktree: &Path) -> PathBuf {
    worktree.join(LANE_FILE_RELATIVE)
}

/// Materialize the lane file for `worktree` from a resolved [`LaneProfile`].
///
/// Called at launch with the authoritative lane (from `is_ephemeral`), this
/// writes `.gwt/session-kind.json` atomically so hooks read the lane
/// deterministically regardless of env propagation (SPEC-3247 hit a
/// production env dead-path; the lane file avoids that class of bug).
pub fn write_lane_file(worktree: &Path, profile: &LaneProfile) -> std::io::Result<()> {
    let path = lane_file_path(worktree);
    let body = format!(
        "{{\n  \"lane\": \"{lane}\",\n  \"profile_version\": {version}\n}}\n",
        lane = profile.id,
        version = LANE_FILE_VERSION,
    );
    crate::settings_local::write_text_atomically(&path, &body)
}

/// Read the lane profile from a worktree's lane file. A missing, unreadable, or
/// unparseable file — and any unknown lane id — resolves to the default
/// (execution), preserving backward compatibility for worktrees materialized
/// before hooks v2.
#[must_use]
pub fn read_lane_profile(worktree: &Path) -> &'static LaneProfile {
    let path = lane_file_path(worktree);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return LaneRegistry::default_profile();
    };
    match parse_lane_id(&text) {
        Some(id) => LaneRegistry::resolve(&id),
        None => LaneRegistry::default_profile(),
    }
}

/// Resolve the lane profile for a worktree at hook time.
///
/// The lane file is the source of truth; when it is present it wins
/// (deterministic, env-independent). For worktrees materialized before hooks v2
/// (no lane file yet) this falls back to the `GWT_SESSION_KIND` env fast-path,
/// so the transition period keeps SPEC-3247 behavior. Both paths default to
/// execution, so an unknown or absent signal never changes producing-work
/// behavior (FR-009 backward compatibility).
#[must_use]
pub fn resolve_lane_for_worktree(worktree: &Path) -> &'static LaneProfile {
    if lane_file_path(worktree).exists() {
        read_lane_profile(worktree)
    } else {
        LaneRegistry::for_session_kind(SessionKind::from_env())
    }
}

/// Extract the `"lane"` string from the lane file body without pulling in a
/// JSON dependency for such a tiny schema. Returns `None` on any shape it does
/// not recognize (→ caller falls back to the default profile).
fn parse_lane_id(text: &str) -> Option<String> {
    let after = text.split("\"lane\"").nth(1)?;
    let after = after.trim_start();
    let after = after.strip_prefix(':')?.trim_start();
    let rest = after.strip_prefix('"')?;
    let end = rest.find('"')?;
    let id = rest[..end].trim().to_string();
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use tempfile::TempDir;

    /// Serialize the one test that mutates the process-global
    /// `GWT_SESSION_KIND` env so parallel test threads cannot race on it.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn resolve_maps_known_lanes_and_defaults_execution() {
        assert_eq!(LaneRegistry::resolve("intake"), &INTAKE_PROFILE);
        assert_eq!(LaneRegistry::resolve("execution"), &EXECUTION_PROFILE);
        // Unknown / empty / whitespace → execution default (fail-safe).
        assert_eq!(LaneRegistry::resolve("review"), &EXECUTION_PROFILE);
        assert_eq!(LaneRegistry::resolve(""), &EXECUTION_PROFILE);
        assert_eq!(LaneRegistry::resolve("  "), &EXECUTION_PROFILE);
        assert_eq!(LaneRegistry::resolve(" intake "), &INTAKE_PROFILE);
        assert_eq!(LaneRegistry::default_profile(), &EXECUTION_PROFILE);
    }

    #[test]
    fn session_kind_bridge_matches_registry() {
        assert_eq!(
            LaneRegistry::for_session_kind(SessionKind::Intake),
            &INTAKE_PROFILE
        );
        assert_eq!(
            LaneRegistry::for_session_kind(SessionKind::Execution),
            &EXECUTION_PROFILE
        );
    }

    #[test]
    fn profiles_reproduce_spec_3247_behavior() {
        // Lock the whole profiles against explicit expected values (a
        // regression guard, and not a constant `assert!`). execution = full
        // producing-work baseline; intake = exactly what SPEC-3247 ships (no
        // Work-state nag, no self-improvement Stop, curation guidance) with all
        // P4 flags still off until their phase lands.
        assert_eq!(
            EXECUTION_PROFILE,
            LaneProfile {
                id: "execution",
                guidance_variant: GuidanceVariant::ProducingWork,
                policy_flags: LanePolicyFlags {
                    emit_work_state_reminders: true,
                    self_improvement_stop: true,
                    block_production_code_edits: false,
                    completion_gate: false,
                    sessionstart_onboarding: false,
                    reduced_skill_set: false,
                },
            }
        );
        assert_eq!(
            INTAKE_PROFILE,
            LaneProfile {
                id: "intake",
                guidance_variant: GuidanceVariant::Curation,
                policy_flags: LanePolicyFlags {
                    emit_work_state_reminders: false,
                    self_improvement_stop: false,
                    block_production_code_edits: false,
                    completion_gate: false,
                    sessionstart_onboarding: false,
                    reduced_skill_set: false,
                },
            }
        );
    }

    #[test]
    fn lane_file_roundtrips_through_write_and_read() {
        let dir = TempDir::new().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".gwt")).expect("mk .gwt");

        write_lane_file(dir.path(), &INTAKE_PROFILE).expect("write intake lane file");
        assert_eq!(read_lane_profile(dir.path()), &INTAKE_PROFILE);

        write_lane_file(dir.path(), &EXECUTION_PROFILE).expect("write execution lane file");
        assert_eq!(read_lane_profile(dir.path()), &EXECUTION_PROFILE);
    }

    #[test]
    fn read_lane_profile_defaults_execution_when_absent_or_malformed() {
        let dir = TempDir::new().expect("tempdir");
        // Absent file → execution.
        assert_eq!(read_lane_profile(dir.path()), &EXECUTION_PROFILE);

        // Malformed / unknown lane → execution.
        std::fs::create_dir_all(dir.path().join(".gwt")).expect("mk .gwt");
        std::fs::write(lane_file_path(dir.path()), "not json").expect("write junk");
        assert_eq!(read_lane_profile(dir.path()), &EXECUTION_PROFILE);
        std::fs::write(lane_file_path(dir.path()), "{\"lane\":\"review\"}").expect("write unknown");
        assert_eq!(read_lane_profile(dir.path()), &EXECUTION_PROFILE);
    }

    #[test]
    fn resolve_lane_prefers_lane_file_then_env_then_execution() {
        let _guard = env_lock();
        let dir = TempDir::new().expect("tempdir");
        // No lane file, no env → execution.
        std::env::remove_var(crate::GWT_SESSION_KIND_ENV);
        assert_eq!(resolve_lane_for_worktree(dir.path()), &EXECUTION_PROFILE);
        // No lane file, env=intake → env fast-path (transition worktrees).
        std::env::set_var(crate::GWT_SESSION_KIND_ENV, "intake");
        assert_eq!(resolve_lane_for_worktree(dir.path()), &INTAKE_PROFILE);
        // Lane file present wins over env (source of truth): file=execution
        // beats env=intake.
        write_lane_file(dir.path(), &EXECUTION_PROFILE).expect("write lane file");
        assert_eq!(resolve_lane_for_worktree(dir.path()), &EXECUTION_PROFILE);
        std::env::remove_var(crate::GWT_SESSION_KIND_ENV);
    }

    #[test]
    fn lane_file_path_is_worktree_relative() {
        let p = lane_file_path(Path::new("/tmp/wt"));
        assert!(p.ends_with(".gwt/session-kind.json"));
    }
}
