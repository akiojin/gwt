//! `gwtd build <start|phase|complete|abort> --spec <n> [...]`
//!
//! Exit CLI for the `gwt-build-spec` skill (SPEC-1935 FR-014r). Writes
//! `.gwt/skill-state/build-spec.json` via [`gwt_core::skill_state`].

use gwt_github::SpecOpsError;

use super::skill_state_runtime;
use crate::cli::{CliEnv, SkillStateAction};

pub(crate) const SKILL_NAME: &str = "build-spec";
pub(crate) const SKILL_DISPLAY: &str = "gwt-build-spec";
pub(crate) const VERB: &str = "build";

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    action: SkillStateAction,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    skill_state_runtime::run(env, action, SKILL_NAME, SKILL_DISPLAY, VERB, out)
}
