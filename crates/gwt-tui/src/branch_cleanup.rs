use std::collections::{HashMap, HashSet};

use crate::screens::branches::{
    BranchItem, CleanupSelectionBlockedReason, CleanupSelectionRisk, MergeState,
};

pub struct CleanupPolicy<'a> {
    branches: &'a [BranchItem],
    current_head_branch: Option<&'a str>,
    active_session_branches: &'a HashSet<String>,
    merged_state: &'a HashMap<String, MergeState>,
}

impl<'a> CleanupPolicy<'a> {
    pub fn new(
        branches: &'a [BranchItem],
        current_head_branch: Option<&'a str>,
        active_session_branches: &'a HashSet<String>,
        merged_state: &'a HashMap<String, MergeState>,
    ) -> Self {
        Self {
            branches,
            current_head_branch,
            active_session_branches,
            merged_state,
        }
    }

    fn branch_item(&self, branch: &str) -> Option<&BranchItem> {
        self.branches.iter().find(|item| item.name == branch)
    }

    fn local_branch_for_remote_ref(name: &str) -> Option<&str> {
        name.strip_prefix("refs/remotes/origin/")
            .or_else(|| name.strip_prefix("origin/"))
    }

    fn merge_state(&self, branch: &str) -> MergeState {
        self.merged_state
            .get(branch)
            .copied()
            .unwrap_or(MergeState::Computing)
    }

    pub fn execution_branch(&self, branch: &str) -> Option<String> {
        let item = self.branch_item(branch)?;
        if item.is_local {
            return Some(item.name.clone());
        }
        let local_name = Self::local_branch_for_remote_ref(&item.name)?;
        self.branches
            .iter()
            .find(|candidate| candidate.is_local && candidate.name == local_name)
            .map(|candidate| candidate.name.clone())
    }

    pub fn blocked_reason(&self, branch: &str) -> Option<CleanupSelectionBlockedReason> {
        let item = self.branch_item(branch)?;
        let Some(execution_branch) = self.execution_branch(branch) else {
            return if item.is_local {
                Some(CleanupSelectionBlockedReason::Unknown)
            } else {
                Some(CleanupSelectionBlockedReason::RemoteTrackingWithoutLocal)
            };
        };
        if gwt_git::is_protected_branch(&execution_branch) {
            return Some(CleanupSelectionBlockedReason::ProtectedBranch);
        }
        if self
            .current_head_branch
            .is_some_and(|head| head == execution_branch)
        {
            return Some(CleanupSelectionBlockedReason::CurrentHead);
        }
        if self.active_session_branches.contains(&execution_branch) {
            return Some(CleanupSelectionBlockedReason::ActiveSession);
        }
        if matches!(self.merge_state(&execution_branch), MergeState::Computing) {
            return Some(CleanupSelectionBlockedReason::MergeCheckRunning);
        }
        None
    }

    pub fn risks(&self, branch: &str) -> Vec<CleanupSelectionRisk> {
        if self.blocked_reason(branch).is_some() {
            return Vec::new();
        }
        let Some(item) = self.branch_item(branch) else {
            return Vec::new();
        };
        let Some(execution_branch) = self.execution_branch(branch) else {
            return Vec::new();
        };
        let mut risks = Vec::new();
        if !item.is_local {
            risks.push(CleanupSelectionRisk::RemoteTracking);
        }
        if matches!(self.merge_state(&execution_branch), MergeState::NotMerged) {
            risks.push(CleanupSelectionRisk::Unmerged);
        }
        risks
    }

    pub fn target(&self, branch: &str) -> Option<gwt_git::MergeTarget> {
        let execution_branch = self.execution_branch(branch)?;
        match self.merge_state(&execution_branch) {
            MergeState::Cleanable(target) => Some(target),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::branches::BranchCategory;

    fn feature(name: &str) -> BranchItem {
        BranchItem {
            name: name.to_string(),
            is_head: false,
            is_local: true,
            category: BranchCategory::Feature,
            worktree_path: None,
            upstream: Some(format!("origin/{name}")),
        }
    }

    fn remote(name: &str) -> BranchItem {
        BranchItem {
            name: name.to_string(),
            is_head: false,
            is_local: false,
            category: BranchCategory::Feature,
            worktree_path: None,
            upstream: None,
        }
    }

    #[test]
    fn cleanup_policy_blocks_branch_with_active_session() {
        let branches = vec![feature("feature/foo")];
        let mut active_sessions = HashSet::new();
        active_sessions.insert("feature/foo".to_string());
        let mut merged_state = HashMap::new();
        merged_state.insert(
            "feature/foo".to_string(),
            MergeState::Cleanable(gwt_git::MergeTarget::Main),
        );

        let policy = CleanupPolicy::new(&branches, Some("main"), &active_sessions, &merged_state);

        assert_eq!(
            policy.blocked_reason("feature/foo"),
            Some(CleanupSelectionBlockedReason::ActiveSession)
        );
    }

    #[test]
    fn cleanup_policy_maps_remote_tracking_row_to_local_branch_and_risks() {
        let branches = vec![feature("feature/foo"), remote("origin/feature/foo")];
        let active_sessions = HashSet::new();
        let mut merged_state = HashMap::new();
        merged_state.insert("feature/foo".to_string(), MergeState::NotMerged);

        let policy = CleanupPolicy::new(&branches, Some("main"), &active_sessions, &merged_state);

        assert_eq!(
            policy.execution_branch("origin/feature/foo"),
            Some("feature/foo".to_string())
        );
        assert_eq!(
            policy.risks("origin/feature/foo"),
            vec![
                CleanupSelectionRisk::RemoteTracking,
                CleanupSelectionRisk::Unmerged,
            ]
        );
        assert_eq!(
            policy.target("origin/feature/foo"),
            None,
            "unmerged branch must not report a cleanup target"
        );
    }
}
