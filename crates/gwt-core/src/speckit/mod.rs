//! Spec Kit embedded module
//!
//! Provides LLM-powered specification workflow:
//! clarify -> specify -> plan -> tasks

pub mod analyze;
pub mod clarify;
pub mod plan;
pub mod specify;
pub mod tasks;
pub mod templates;

use serde::{Deserialize, Serialize};

/// A Spec Kit artifact reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecKitArtifact {
    /// Spec ID (e.g., "SPEC-ba3f610c")
    pub spec_id: String,
    /// Path to spec.md
    pub spec_path: Option<String>,
    /// Path to plan.md
    pub plan_path: Option<String>,
    /// Path to tasks.md
    pub tasks_path: Option<String>,
}

impl SpecKitArtifact {
    pub fn new(spec_id: impl Into<String>) -> Self {
        let id = spec_id.into();
        let base = format!("specs/{}", id);
        Self {
            spec_id: id,
            spec_path: Some(format!("{}/spec.md", base)),
            plan_path: Some(format!("{}/plan.md", base)),
            tasks_path: Some(format!("{}/tasks.md", base)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec_kit_artifact_new() {
        let artifact = SpecKitArtifact::new("SPEC-12345678");
        assert_eq!(artifact.spec_id, "SPEC-12345678");
        assert_eq!(
            artifact.spec_path.as_deref(),
            Some("specs/SPEC-12345678/spec.md")
        );
        assert_eq!(
            artifact.plan_path.as_deref(),
            Some("specs/SPEC-12345678/plan.md")
        );
        assert_eq!(
            artifact.tasks_path.as_deref(),
            Some("specs/SPEC-12345678/tasks.md")
        );
    }
}
