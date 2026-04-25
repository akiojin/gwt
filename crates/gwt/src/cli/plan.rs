//! `gwtd plan <start|phase|complete|abort> --spec <n> [...]`
//!
//! Exit CLI for the `gwt-plan-spec` skill (SPEC-1935 FR-014q). Writes
//! `.gwt/skill-state/plan-spec.json` via [`gwt_core::skill_state`].

use gwt_github::SpecOpsError;

use super::skill_state_runtime;
use crate::cli::{CliEnv, SkillStateAction};

pub(crate) const SKILL_NAME: &str = "plan-spec";
pub(crate) const SKILL_DISPLAY: &str = "gwt-plan-spec";
pub(crate) const VERB: &str = "plan";

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    action: SkillStateAction,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    skill_state_runtime::run(env, action, SKILL_NAME, SKILL_DISPLAY, VERB, out)
}
