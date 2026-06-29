//! Default-branch protection verification for autonomous Issue Monitor merges
//! (SPEC #3200, FR-010).
//!
//! The autonomous merge gate must be backed by structural GitHub branch
//! protection — required status checks must exist (otherwise GitHub reports a
//! *vacuous green* combined status), merge permissions must be restricted, and
//! direct pushes to the default branch must be prohibited. Only when all three
//! hold *and* the protection is actually readable can an issue become
//! autonomous-eligible.
//!
//! This module owns the [`BranchProtectionStatus`] value type that the
//! eligibility predicate consumes. The live `gh api` adapter that populates it
//! lands in Phase 2 (T-080); the eligibility predicate is exercised in Phase 1
//! via an injected status value.

use serde::{Deserialize, Serialize};

/// Whether the default-branch protection could actually be read.
///
/// The classic `gh api repos/{owner}/{repo}/branches/{branch}/protection`
/// endpoint is admin-scoped and returns `403` for non-admin maintainers even
/// when protection exists. We must distinguish that *unverifiable* case from a
/// genuine *absent* protection: both route to `needs-human`, but with different
/// reasons (FR-010, Sc 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProtectionReadability {
    /// Protection was read successfully.
    Verified,
    /// Protection could not be read (e.g. 403 due to token scope). Carries a
    /// human-readable reason for the `needs-human` surface.
    Unreadable { reason: String },
    /// Protection is genuinely absent (e.g. 404 / empty protection object).
    Absent,
}

/// Structural branch-protection state for a repository's default branch.
///
/// `verified` is the conjunction of all four structural conditions and is the
/// single field the eligibility predicate reads (FR-003 (iv)). Construct via
/// [`BranchProtectionStatus::new`] so `verified` always stays consistent with
/// its inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchProtectionStatus {
    pub branch: String,
    pub required_checks_present: bool,
    pub merge_permissions_restricted: bool,
    pub direct_push_blocked: bool,
    pub readability: ProtectionReadability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
    pub verified: bool,
}

impl BranchProtectionStatus {
    /// Build a status with `verified` computed from the structural conditions.
    pub fn new(
        branch: impl Into<String>,
        required_checks_present: bool,
        merge_permissions_restricted: bool,
        direct_push_blocked: bool,
        readability: ProtectionReadability,
        verified_at: Option<String>,
    ) -> Self {
        let verified = required_checks_present
            && merge_permissions_restricted
            && direct_push_blocked
            && matches!(readability, ProtectionReadability::Verified);
        Self {
            branch: branch.into(),
            required_checks_present,
            merge_permissions_restricted,
            direct_push_blocked,
            readability,
            verified_at,
            verified,
        }
    }

    /// A fully-protected, readable default branch (autonomous-eligible w.r.t.
    /// FR-003 (iv)).
    pub fn fully_protected(branch: impl Into<String>) -> Self {
        Self::new(
            branch,
            true,
            true,
            true,
            ProtectionReadability::Verified,
            None,
        )
    }

    /// Protection is genuinely absent — gate-unavailable (FR-010).
    pub fn absent(branch: impl Into<String>) -> Self {
        Self::new(
            branch,
            false,
            false,
            false,
            ProtectionReadability::Absent,
            None,
        )
    }

    /// Protection could not be read due to permissions — gate-unavailable, with
    /// a reason distinct from `absent` (FR-010, Sc 4).
    pub fn unreadable(branch: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new(
            branch,
            false,
            false,
            false,
            ProtectionReadability::Unreadable {
                reason: reason.into(),
            },
            None,
        )
    }

    /// Human-readable reason a non-verified protection is gate-unavailable, for
    /// the `needs-human` surface. Returns `None` when verified.
    pub fn unavailable_reason(&self) -> Option<String> {
        if self.verified {
            return None;
        }
        match &self.readability {
            ProtectionReadability::Verified => {
                let mut missing = Vec::new();
                if !self.required_checks_present {
                    missing.push("required status checks are not configured (vacuous green)");
                }
                if !self.merge_permissions_restricted {
                    missing.push("merge permissions are not restricted");
                }
                if !self.direct_push_blocked {
                    missing.push("direct pushes to the default branch are not blocked");
                }
                Some(format!(
                    "branch protection incomplete: {}",
                    missing.join("; ")
                ))
            }
            ProtectionReadability::Unreadable { reason } => Some(format!(
                "branch protection could not be verified (insufficient permissions): {reason}"
            )),
            ProtectionReadability::Absent => {
                Some("branch protection is absent on the default branch".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fully_protected_is_verified() {
        let status = BranchProtectionStatus::fully_protected("main");
        assert!(status.verified);
        assert_eq!(status.unavailable_reason(), None);
    }

    #[test]
    fn absent_is_not_verified_with_reason() {
        let status = BranchProtectionStatus::absent("main");
        assert!(!status.verified);
        assert!(status.unavailable_reason().unwrap().contains("absent"));
    }

    #[test]
    fn unreadable_is_distinct_from_absent() {
        let status = BranchProtectionStatus::unreadable("main", "HTTP 403");
        assert!(!status.verified);
        let reason = status.unavailable_reason().unwrap();
        assert!(reason.contains("permissions"));
        assert!(reason.contains("403"));
        assert!(matches!(
            status.readability,
            ProtectionReadability::Unreadable { .. }
        ));
    }

    #[test]
    fn missing_required_checks_is_vacuous_green_unavailable() {
        let status = BranchProtectionStatus::new(
            "main",
            false,
            true,
            true,
            ProtectionReadability::Verified,
            None,
        );
        assert!(!status.verified);
        assert!(status
            .unavailable_reason()
            .unwrap()
            .contains("vacuous green"));
    }
}
