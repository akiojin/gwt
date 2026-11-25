//! `register.*` JSON lifecycle operations.
//!
//! Exit CLI for the `gwt-register-spec` skill (SPEC-2784). Writes
//! `.gwt/skill-state/register-spec.json` via [`gwt_core::skill_state`].
//!
//! The lifecycle mirrors `gwt-plan-spec` and `gwt-build-spec`: `start` is
//! called when the skill begins materializing a SPEC, `phase` records each
//! milestone (`validation`, `create`, `edit`, `roundtrip`), and
//! `complete` / `abort` flip `active: false` so the Stop-block handler stops
//! forcing continuation.
//!
//! Because the SPEC id is not known until `issue.spec.create` returns,
//! `gwt-register-spec` may legitimately call `register.start` with
//! `params.spec:0` and then re-emit `register.phase` with the real SPEC id once
//! the Issue is created.

use gwt_github::SpecOpsError;

use super::skill_state_runtime;
use crate::cli::{parse_skill_state_args, CliCommand, CliEnv, CliParseError, SkillStateAction};

/// Parse the legacy register argv model used by internal dispatcher tests.
pub fn parse_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    parse_skill_state_args(args).map(CliCommand::Register)
}

pub const SKILL_NAME: &str = "register-spec";
pub const SKILL_DISPLAY: &str = "gwt-register-spec";
pub const VERB: &str = "register";

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    action: SkillStateAction,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    skill_state_runtime::run(env, action, SKILL_NAME, SKILL_DISPLAY, VERB, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec(env: &mut crate::cli::TestEnv, action: SkillStateAction) -> (i32, String) {
        let mut out = String::new();
        let code = run(env, action, &mut out).unwrap();
        (code, out)
    }

    fn phase(spec: u64, label: &str) -> SkillStateAction {
        SkillStateAction::Phase {
            spec,
            label: label.to_string(),
        }
    }

    /// Issue #7342 / #6448: the documented gwt-register-spec handoff starts
    /// on placeholder SPEC-0 and binds the created Issue number at
    /// `register.phase` `create`.
    #[test]
    fn phase_binds_created_spec_after_placeholder_start() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(dir.path().to_path_buf());

        let (code, out) = exec(&mut env, SkillStateAction::Start { spec: 0 });
        assert_eq!(code, 0, "{out}");
        let (code, out) = exec(&mut env, phase(0, "validation"));
        assert_eq!(code, 0, "{out}");
        let (code, out) = exec(&mut env, phase(949, "create"));
        assert_eq!(code, 0, "{out}");
        let state = gwt_core::skill_state::load(dir.path(), SKILL_NAME)
            .unwrap()
            .unwrap();
        assert_eq!(state.owner_spec, Some(949));
        assert_eq!(state.phase.as_deref(), Some("create"));

        // Once bound, a different SPEC is still refused.
        let (code, out) = exec(&mut env, phase(950, "edit"));
        assert_eq!(code, 2, "{out}");
        assert!(out.contains("state owns SPEC-Some(949)"), "{out}");

        for label in ["edit", "roundtrip"] {
            let (code, out) = exec(&mut env, phase(949, label));
            assert_eq!(code, 0, "{out}");
        }
        let (code, out) = exec(
            &mut env,
            SkillStateAction::Complete {
                spec: 949,
                pr: None,
            },
        );
        assert_eq!(code, 0, "{out}");
        let state = gwt_core::skill_state::load(dir.path(), SKILL_NAME)
            .unwrap()
            .unwrap();
        assert!(!state.active);
        assert_eq!(state.owner_spec, Some(949));
    }

    /// A placeholder lifecycle can also be finalized directly under the
    /// created SPEC id.
    #[test]
    fn complete_binds_created_spec_after_placeholder_start() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(dir.path().to_path_buf());
        let (code, out) = exec(&mut env, SkillStateAction::Start { spec: 0 });
        assert_eq!(code, 0, "{out}");
        let (code, out) = exec(
            &mut env,
            SkillStateAction::Complete {
                spec: 949,
                pr: None,
            },
        );
        assert_eq!(code, 0, "{out}");
        let state = gwt_core::skill_state::load(dir.path(), SKILL_NAME)
            .unwrap()
            .unwrap();
        assert!(!state.active);
        assert_eq!(state.owner_spec, Some(949));
    }
}
