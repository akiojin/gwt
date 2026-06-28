//! SPEC #3200 — GitHub branch-protection adapter for autonomous-mode eligibility.
//! Verifies (read-only) required-status-checks existence, restricted merge perms,
//! and no-direct-push, distinguishing protection-absent from unverifiable-by-permission.
//!
//! Autonomous merge is only structurally safe when GitHub itself refuses to
//! merge an un-gated SHA. This adapter classifies the protection of the base
//! branch into one of three eligibility-relevant outcomes. The network fetch is
//! wired in a later phase; the pure classifier here is the testable core and
//! must fail closed: anything other than a fully verified protection is
//! gate-unavailable.

/// Branch-protection status relevant to autonomous-mode eligibility (FR-010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchProtectionStatus {
    /// All three structural conditions are verified present: at least one
    /// required status check exists, merge permissions are restricted, and
    /// direct pushes to the branch are disallowed. Carries the required check
    /// contexts so a `vacuous green` (zero required checks) can never qualify.
    Verified { required_checks: Vec<String> },
    /// Protection is provably absent (e.g. GitHub `404` / empty protection).
    /// Gate-unavailable; routes to `NeedsHuman` with a "protection absent" reason.
    Absent,
    /// Protection could not be read because of token permissions (e.g. `403` on
    /// the admin-scoped protection endpoint). Distinct from `Absent` (FR-010,
    /// Sc 4): protection may well exist, we just cannot verify it — so still
    /// gate-unavailable, but the human-facing reason differs.
    Unreadable(String),
}

impl BranchProtectionStatus {
    /// Only a `Verified` status with at least one required check structurally
    /// backs an autonomous merge. Everything else is gate-unavailable.
    pub fn is_verified(&self) -> bool {
        matches!(self, BranchProtectionStatus::Verified { required_checks } if !required_checks.is_empty())
    }
}

/// Parse a `gh api .../branches/{branch}/protection` JSON body into a
/// [`BranchProtectionStatus`]. This is the success path (HTTP 200): the three
/// structural conditions are read from the JSON. Non-200 outcomes
/// (`404`/`403`) are mapped by the caller to `Absent` / `Unreadable` and never
/// reach this parser. Fail-closed: a body missing any condition → not verified
/// (treated as `Absent` so callers route to gate-unavailable).
pub fn parse_branch_protection(json: &str) -> BranchProtectionStatus {
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(value) => value,
        Err(_) => return BranchProtectionStatus::Absent,
    };

    // (1) at least one required status check context must exist (vacuous-green guard).
    let required_checks: Vec<String> = value
        .get("required_status_checks")
        .and_then(|rsc| rsc.get("contexts"))
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .filter(|c| !c.trim().is_empty())
                .collect()
        })
        .unwrap_or_default();

    // (2) merge / push permissions are restricted (restrictions block present).
    let restricted = value
        .get("restrictions")
        .map(|r| !r.is_null())
        .unwrap_or(false);

    // (3) direct push is disallowed: branch protection is enabled AND either
    // restrictions exist or force pushes are disabled. GitHub exposes
    // `allow_force_pushes.enabled`; a protected branch without restrictions but
    // with required reviews/linear history still blocks ad-hoc direct push, but
    // we require the explicit restriction to stay fail-closed.
    let force_push_disabled = value
        .get("allow_force_pushes")
        .and_then(|fp| fp.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .map(|enabled| !enabled)
        .unwrap_or(false);

    if !required_checks.is_empty() && restricted && force_push_disabled {
        BranchProtectionStatus::Verified { required_checks }
    } else {
        // Protection exists but does not structurally back autonomous merge.
        BranchProtectionStatus::Absent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fully_protected_branch_is_verified() {
        let json = r#"{
            "required_status_checks": {"contexts": ["build", "test"]},
            "restrictions": {"users": [], "teams": ["maintainers"]},
            "allow_force_pushes": {"enabled": false}
        }"#;
        let status = parse_branch_protection(json);
        assert!(status.is_verified());
        assert_eq!(
            status,
            BranchProtectionStatus::Verified {
                required_checks: vec!["build".to_string(), "test".to_string()]
            }
        );
    }

    #[test]
    fn zero_required_checks_is_not_verified_vacuous_green_guard() {
        let json = r#"{
            "required_status_checks": {"contexts": []},
            "restrictions": {"users": []},
            "allow_force_pushes": {"enabled": false}
        }"#;
        let status = parse_branch_protection(json);
        assert!(
            !status.is_verified(),
            "zero required checks must not qualify"
        );
        assert_eq!(status, BranchProtectionStatus::Absent);
    }

    #[test]
    fn missing_restrictions_or_force_push_enabled_is_not_verified() {
        let no_restrictions = r#"{"required_status_checks":{"contexts":["build"]},"restrictions":null,"allow_force_pushes":{"enabled":false}}"#;
        assert!(!parse_branch_protection(no_restrictions).is_verified());
        let force_pushes = r#"{"required_status_checks":{"contexts":["build"]},"restrictions":{"users":[]},"allow_force_pushes":{"enabled":true}}"#;
        assert!(!parse_branch_protection(force_pushes).is_verified());
    }

    #[test]
    fn unparseable_body_fails_closed_to_absent() {
        assert_eq!(
            parse_branch_protection("not json"),
            BranchProtectionStatus::Absent
        );
        assert_eq!(
            parse_branch_protection("{}"),
            BranchProtectionStatus::Absent
        );
    }

    #[test]
    fn absent_and_unreadable_are_distinct_and_not_verified() {
        assert!(!BranchProtectionStatus::Absent.is_verified());
        assert!(!BranchProtectionStatus::Unreadable("403".into()).is_verified());
        assert_ne!(
            BranchProtectionStatus::Absent,
            BranchProtectionStatus::Unreadable("403".into())
        );
    }
}
