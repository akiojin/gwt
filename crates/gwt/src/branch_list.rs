use std::{cmp::Ordering, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchScope {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchListEntry {
    pub name: String,
    pub scope: BranchScope,
    pub is_head: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub last_commit_date: Option<String>,
}

pub fn list_branch_entries(repo_path: &Path) -> std::io::Result<Vec<BranchListEntry>> {
    let branches = gwt_git::branch::list_branches(repo_path)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(adapt_branches(branches))
}

fn adapt_branches(branches: Vec<gwt_git::Branch>) -> Vec<BranchListEntry> {
    let mut entries: Vec<BranchListEntry> = branches
        .into_iter()
        .map(|branch| BranchListEntry {
            name: branch.name,
            scope: if branch.is_remote {
                BranchScope::Remote
            } else {
                BranchScope::Local
            },
            is_head: branch.is_head,
            upstream: branch.upstream,
            ahead: branch.ahead,
            behind: branch.behind,
            last_commit_date: branch.last_commit_date,
        })
        .collect();

    entries.sort_by(compare_branch_entries);
    entries
}

fn compare_branch_entries(left: &BranchListEntry, right: &BranchListEntry) -> Ordering {
    right
        .is_head
        .cmp(&left.is_head)
        .then_with(|| match (left.scope, right.scope) {
            (BranchScope::Local, BranchScope::Remote) => Ordering::Less,
            (BranchScope::Remote, BranchScope::Local) => Ordering::Greater,
            _ => Ordering::Equal,
        })
        .then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
        .then_with(|| left.name.cmp(&right.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapt_branches_sorts_head_then_local_then_remote() {
        let branches = vec![
            gwt_git::Branch {
                name: "origin/main".to_string(),
                is_local: false,
                is_remote: true,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: None,
            },
            gwt_git::Branch {
                name: "feature/zeta".to_string(),
                is_local: true,
                is_remote: false,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: None,
            },
            gwt_git::Branch {
                name: "main".to_string(),
                is_local: true,
                is_remote: false,
                is_head: true,
                upstream: Some("origin/main".to_string()),
                ahead: 0,
                behind: 0,
                last_commit_date: None,
            },
            gwt_git::Branch {
                name: "feature/alpha".to_string(),
                is_local: true,
                is_remote: false,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: None,
            },
        ];

        let entries = adapt_branches(branches);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["main", "feature/alpha", "feature/zeta", "origin/main"]
        );
        assert_eq!(entries[0].scope, BranchScope::Local);
        assert!(entries[0].is_head);
        assert_eq!(entries[3].scope, BranchScope::Remote);
    }
}
