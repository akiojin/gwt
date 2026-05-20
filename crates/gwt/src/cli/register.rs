//! `gwtd register <start|phase|complete|abort> --spec <n> [...]`
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
//! Because the SPEC id is not known until `gwtd issue spec create` returns,
//! `gwt-register-spec` may legitimately call `start --spec 0` and then
//! re-emit `phase --spec <real-id>` once the Issue is created.

use gwt_github::SpecOpsError;

use super::skill_state_runtime;
use crate::cli::{CliEnv, SkillStateAction};

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
