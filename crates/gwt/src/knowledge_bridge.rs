use std::{collections::HashMap, path::Path};

use gwt_core::paths::gwt_cache_dir;
use gwt_github::{Cache, CacheEntry, IssueState, SectionName};
use serde::{Deserialize, Serialize};

use crate::issue_cache::{
    issue_cache_has_entries, issue_cache_root_for_repo_path,
    issue_cache_root_for_repo_path_or_detached, sync_issue_cache_from_remote,
    sync_issue_cache_from_remote_if_stale, ISSUE_CACHE_TTL,
};

const SPEC_LABEL: &str = "gwt-spec";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    Issue,
    Spec,
    Pr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeListScope {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeListItem {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub meta: String,
    pub labels: Vec<String>,
    pub linked_branch_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeDetailSection {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeDetailView {
    pub number: Option<u64>,
    pub title: String,
    pub subtitle: String,
    pub state: String,
    pub labels: Vec<String>,
    pub sections: Vec<KnowledgeDetailSection>,
    pub launch_issue_number: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeBridgeView {
    pub kind: KnowledgeKind,
    pub entries: Vec<KnowledgeListItem>,
    pub selected_number: Option<u64>,
    pub empty_message: Option<String>,
    pub refresh_enabled: bool,
    pub detail: KnowledgeDetailView,
}

pub fn load_knowledge_bridge(
    repo_path: &Path,
    kind: KnowledgeKind,
    selected_number: Option<u64>,
    refresh: bool,
    list_scope: KnowledgeListScope,
) -> Result<KnowledgeBridgeView, String> {
    if !repo_path.is_dir() {
        return Err(format!(
            "project root is not available: {}",
            repo_path.display()
        ));
    }

    if matches!(kind, KnowledgeKind::Pr) {
        return Ok(disabled_pr_view());
    }

    if issue_cache_root_for_repo_path(repo_path).is_none() {
        return Ok(non_repo_view(kind));
    }

    let cache_root = issue_cache_root_for_repo_path_or_detached(repo_path);
    if refresh {
        sync_issue_cache_from_remote(repo_path, &cache_root)?;
    } else if let Err(error) =
        sync_issue_cache_from_remote_if_stale(repo_path, &cache_root, ISSUE_CACHE_TTL)
    {
        if !issue_cache_has_entries(&cache_root) {
            return Err(error);
        }
    }

    let cache = Cache::new(cache_root);
    let entries = load_cache_entries(&cache)?;
    let linked_branches = load_linked_branches(repo_path);
    Ok(match kind {
        KnowledgeKind::Issue => {
            build_issue_view(entries, linked_branches, selected_number, list_scope)
        }
        KnowledgeKind::Spec => build_spec_view(entries, linked_branches, selected_number),
        KnowledgeKind::Pr => disabled_pr_view(),
    })
}

fn load_cache_entries(cache: &Cache) -> Result<Vec<CacheEntry>, String> {
    match cache.list_entries() {
        Ok(entries) => Ok(entries),
        Err(gwt_github::CacheError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(Vec::new())
        }
        Err(error) => Err(format!("failed to read issue cache: {error}")),
    }
}

fn build_issue_view(
    mut entries: Vec<CacheEntry>,
    linked_branches: HashMap<u64, Vec<String>>,
    selected_number: Option<u64>,
    list_scope: KnowledgeListScope,
) -> KnowledgeBridgeView {
    entries.retain(|entry| !is_spec_entry(entry));
    entries.retain(|entry| match list_scope {
        KnowledgeListScope::Open => entry.snapshot.state == IssueState::Open,
        KnowledgeListScope::Closed => entry.snapshot.state == IssueState::Closed,
    });
    entries.sort_by(issue_entry_sort);

    let list_items = entries
        .iter()
        .map(|entry| KnowledgeListItem {
            number: entry.snapshot.number.0,
            title: entry.snapshot.title.clone(),
            state: issue_state_label(entry.snapshot.state),
            meta: format!("Updated {}", short_updated_at(&entry.snapshot.updated_at.0)),
            labels: entry.snapshot.labels.clone(),
            linked_branch_count: linked_branches
                .get(&entry.snapshot.number.0)
                .map(Vec::len)
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let selected_number = resolve_selected_number(&entries, selected_number);
    let detail = entries
        .iter()
        .find(|entry| Some(entry.snapshot.number.0) == selected_number)
        .map(|entry| issue_detail_view(entry, linked_branches.get(&entry.snapshot.number.0)))
        .unwrap_or_else(|| empty_detail("Issue Bridge", "No cached issues available."));

    KnowledgeBridgeView {
        kind: KnowledgeKind::Issue,
        entries: list_items,
        selected_number,
        empty_message: if selected_number.is_none() {
            Some("No cached issues. Use Refresh to sync the cache.".to_string())
        } else {
            None
        },
        refresh_enabled: true,
        detail,
    }
}

fn build_spec_view(
    mut entries: Vec<CacheEntry>,
    linked_branches: HashMap<u64, Vec<String>>,
    selected_number: Option<u64>,
) -> KnowledgeBridgeView {
    entries.retain(is_spec_entry);
    entries.sort_by(issue_entry_sort);

    let list_items = entries
        .iter()
        .map(|entry| KnowledgeListItem {
            number: entry.snapshot.number.0,
            title: entry.snapshot.title.clone(),
            state: issue_state_label(entry.snapshot.state),
            meta: spec_list_meta(entry),
            labels: entry.snapshot.labels.clone(),
            linked_branch_count: linked_branches
                .get(&entry.snapshot.number.0)
                .map(Vec::len)
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let selected_number = resolve_selected_number(&entries, selected_number);
    let detail = entries
        .iter()
        .find(|entry| Some(entry.snapshot.number.0) == selected_number)
        .map(spec_detail_view)
        .unwrap_or_else(|| empty_detail("SPEC Bridge", "No cached SPECs available."));

    KnowledgeBridgeView {
        kind: KnowledgeKind::Spec,
        entries: list_items,
        selected_number,
        empty_message: if selected_number.is_none() {
            Some("No cached SPECs. Use Refresh to sync the cache.".to_string())
        } else {
            None
        },
        refresh_enabled: true,
        detail,
    }
}

fn disabled_pr_view() -> KnowledgeBridgeView {
    KnowledgeBridgeView {
        kind: KnowledgeKind::Pr,
        entries: Vec::new(),
        selected_number: None,
        empty_message: Some(
            "PR Bridge is waiting for cache-backed PR list support before it can render data."
                .to_string(),
        ),
        refresh_enabled: false,
        detail: KnowledgeDetailView {
            number: None,
            title: "PR Bridge".to_string(),
            subtitle: "Unavailable".to_string(),
            state: "unavailable".to_string(),
            labels: Vec::new(),
            sections: vec![KnowledgeDetailSection {
                title: "Status".to_string(),
                body: "PR Bridge is waiting for cache-backed PR list support before it can render data."
                    .to_string(),
            }],
            launch_issue_number: None,
        },
    }
}

fn non_repo_view(kind: KnowledgeKind) -> KnowledgeBridgeView {
    let title = match kind {
        KnowledgeKind::Issue => "Issue Bridge",
        KnowledgeKind::Spec => "SPEC Bridge",
        KnowledgeKind::Pr => "PR Bridge",
    };
    KnowledgeBridgeView {
        kind,
        entries: Vec::new(),
        selected_number: None,
        empty_message: Some("Knowledge Bridge is available only for Git projects.".to_string()),
        refresh_enabled: false,
        detail: empty_detail(
            title,
            "Knowledge Bridge is available only for Git projects.",
        ),
    }
}

fn empty_detail(title: &str, body: &str) -> KnowledgeDetailView {
    KnowledgeDetailView {
        number: None,
        title: title.to_string(),
        subtitle: String::new(),
        state: "idle".to_string(),
        labels: Vec::new(),
        sections: vec![KnowledgeDetailSection {
            title: "Status".to_string(),
            body: body.to_string(),
        }],
        launch_issue_number: None,
    }
}

fn issue_detail_view(
    entry: &CacheEntry,
    linked_branches: Option<&Vec<String>>,
) -> KnowledgeDetailView {
    let mut sections = Vec::new();
    let body = entry.snapshot.body.trim();
    if !body.is_empty() {
        sections.push(KnowledgeDetailSection {
            title: "Description".to_string(),
            body: body.to_string(),
        });
    }
    for (index, comment) in entry.snapshot.comments.iter().enumerate() {
        let comment_body = comment.body.trim();
        if comment_body.is_empty() {
            continue;
        }
        sections.push(KnowledgeDetailSection {
            title: format!("Comment {}", index + 1),
            body: comment_body.to_string(),
        });
    }
    if let Some(branches) = linked_branches.filter(|branches| !branches.is_empty()) {
        sections.push(KnowledgeDetailSection {
            title: "Linked branches".to_string(),
            body: linked_branches_markdown(branches),
        });
    }
    if sections.is_empty() {
        sections.push(KnowledgeDetailSection {
            title: "Status".to_string(),
            body: "No cached issue details available.".to_string(),
        });
    }

    KnowledgeDetailView {
        number: Some(entry.snapshot.number.0),
        title: entry.snapshot.title.clone(),
        subtitle: format!(
            "#{} · {} · Updated {}",
            entry.snapshot.number.0,
            issue_state_label(entry.snapshot.state),
            short_updated_at(&entry.snapshot.updated_at.0)
        ),
        state: issue_state_label(entry.snapshot.state),
        labels: entry.snapshot.labels.clone(),
        sections,
        launch_issue_number: Some(entry.snapshot.number.0),
    }
}

fn linked_branches_markdown(branches: &[String]) -> String {
    branches
        .iter()
        .map(|branch| format!("- `{}`", branch.replace('`', "\\`")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn spec_detail_view(entry: &CacheEntry) -> KnowledgeDetailView {
    let mut sections = Vec::new();
    for name in ["spec", "plan", "tasks"] {
        if let Some(body) = entry.spec_body.sections.get(&SectionName(name.to_string())) {
            if !body.trim().is_empty() {
                sections.push(KnowledgeDetailSection {
                    title: name.to_string(),
                    body: body.trim().to_string(),
                });
            }
        }
    }
    for (name, body) in &entry.spec_body.sections {
        if matches!(name.0.as_str(), "spec" | "plan" | "tasks") || body.trim().is_empty() {
            continue;
        }
        sections.push(KnowledgeDetailSection {
            title: name.0.clone(),
            body: body.trim().to_string(),
        });
    }
    if sections.is_empty() {
        sections.push(KnowledgeDetailSection {
            title: "Status".to_string(),
            body: "No cached SPEC sections available.".to_string(),
        });
    }

    let phase = entry
        .snapshot
        .labels
        .iter()
        .find(|label| label.starts_with("phase/"))
        .cloned()
        .unwrap_or_else(|| "phase/unspecified".to_string());
    KnowledgeDetailView {
        number: Some(entry.snapshot.number.0),
        title: entry.snapshot.title.clone(),
        subtitle: format!(
            "#{} · {} · Updated {}",
            entry.snapshot.number.0,
            phase,
            short_updated_at(&entry.snapshot.updated_at.0)
        ),
        state: issue_state_label(entry.snapshot.state),
        labels: entry.snapshot.labels.clone(),
        sections,
        launch_issue_number: Some(entry.snapshot.number.0),
    }
}

fn spec_list_meta(entry: &CacheEntry) -> String {
    let phase = entry
        .snapshot
        .labels
        .iter()
        .find(|label| label.starts_with("phase/"))
        .cloned()
        .unwrap_or_else(|| "phase/unspecified".to_string());
    format!(
        "{phase} · Updated {}",
        short_updated_at(&entry.snapshot.updated_at.0)
    )
}

fn resolve_selected_number(entries: &[CacheEntry], selected_number: Option<u64>) -> Option<u64> {
    selected_number
        .filter(|selected| {
            entries
                .iter()
                .any(|entry| entry.snapshot.number.0 == *selected)
        })
        .or_else(|| entries.first().map(|entry| entry.snapshot.number.0))
}

fn issue_entry_sort(left: &CacheEntry, right: &CacheEntry) -> std::cmp::Ordering {
    let left_state = if left.snapshot.state == IssueState::Open {
        0
    } else {
        1
    };
    let right_state = if right.snapshot.state == IssueState::Open {
        0
    } else {
        1
    };
    left_state
        .cmp(&right_state)
        .then_with(|| right.snapshot.updated_at.0.cmp(&left.snapshot.updated_at.0))
        .then_with(|| left.snapshot.number.0.cmp(&right.snapshot.number.0))
}

fn issue_state_label(state: IssueState) -> String {
    match state {
        IssueState::Open => "open".to_string(),
        IssueState::Closed => "closed".to_string(),
    }
}

fn short_updated_at(updated_at: &str) -> String {
    updated_at.get(..10).unwrap_or(updated_at).to_string()
}

fn is_spec_entry(entry: &CacheEntry) -> bool {
    entry
        .snapshot
        .labels
        .iter()
        .any(|label| label == SPEC_LABEL)
}

#[derive(Debug, Default, Deserialize)]
struct IssueBranchLinkStore {
    #[serde(default)]
    branches: HashMap<String, u64>,
}

fn load_linked_branches(repo_path: &Path) -> HashMap<u64, Vec<String>> {
    let Some(repo_hash) = crate::index_worker::detect_repo_hash(repo_path) else {
        return HashMap::new();
    };
    let path = gwt_cache_dir()
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    let Ok(store) = serde_json::from_slice::<IssueBranchLinkStore>(&bytes) else {
        return HashMap::new();
    };

    let mut linked = HashMap::<u64, Vec<String>>::new();
    for (branch, issue_number) in store.branches {
        linked.entry(issue_number).or_default().push(branch);
    }
    for branches in linked.values_mut() {
        branches.sort();
    }
    linked
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, ffi::OsString, fs};

    use gwt_github::{
        client::{CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt},
        Cache,
    };

    use super::*;

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn init_repo(repo: &Path) {
        fs::create_dir_all(repo).expect("create repo");
        let init = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repo)
            .output()
            .expect("git init");
        assert!(init.status.success(), "git init failed");

        let remote = std::process::Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/example/repo.git",
            ])
            .current_dir(repo)
            .output()
            .expect("git remote add");
        assert!(remote.status.success(), "git remote add failed");
    }

    fn issue_snapshot(
        number: u64,
        title: &str,
        body: &str,
        labels: &[&str],
        state: IssueState,
    ) -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(number),
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            state,
            updated_at: UpdatedAt::new("2026-04-20T12:34:56Z"),
            comments: vec![CommentSnapshot {
                id: CommentId(41),
                body: "Follow-up detail".to_string(),
                updated_at: UpdatedAt::new("2026-04-20T12:35:00Z"),
            }],
        }
    }

    fn spec_snapshot(number: u64) -> IssueSnapshot {
        issue_snapshot(
            number,
            "Coverage SPEC",
            r#"<!-- gwt-spec id=2001 version=1 -->
<!-- sections:
spec=body
plan=body
tasks=body
notes=body
-->
<!-- artifact:spec BEGIN -->
Raise project coverage to 90%.
<!-- artifact:spec END -->

<!-- artifact:plan BEGIN -->
1. Add tests.
<!-- artifact:plan END -->

<!-- artifact:tasks BEGIN -->
- [ ] Add push-time gate.
<!-- artifact:tasks END -->

<!-- artifact:notes BEGIN -->
Extra context.
<!-- artifact:notes END -->
"#,
            &["gwt-spec", "phase/in-progress"],
            IssueState::Open,
        )
    }

    fn write_issue_links(repo_path: &Path, links: &[(&str, u64)]) {
        let repo_hash = crate::index_worker::detect_repo_hash(repo_path).expect("repo hash");
        let path = gwt_cache_dir()
            .join("issue-links")
            .join(format!("{}.json", repo_hash.as_str()));
        fs::create_dir_all(path.parent().expect("issue links dir"))
            .expect("create issue-links dir");
        let branches = links
            .iter()
            .map(|(branch, issue)| ((*branch).to_string(), *issue))
            .collect::<HashMap<_, _>>();
        let bytes = serde_json::to_vec(&serde_json::json!({ "branches": branches }))
            .expect("serialize links");
        fs::write(path, bytes).expect("write links");
    }

    #[test]
    fn load_knowledge_bridge_returns_non_repo_and_disabled_pr_views() {
        let dir = tempfile::tempdir().expect("tempdir");

        let issue_view = load_knowledge_bridge(
            dir.path(),
            KnowledgeKind::Issue,
            None,
            false,
            KnowledgeListScope::Open,
        )
        .expect("issue view");
        assert_eq!(issue_view.kind, KnowledgeKind::Issue);
        assert!(!issue_view.refresh_enabled);
        assert_eq!(
            issue_view.empty_message.as_deref(),
            Some("Knowledge Bridge is available only for Git projects.")
        );

        let pr_view = load_knowledge_bridge(
            dir.path(),
            KnowledgeKind::Pr,
            Some(12),
            false,
            KnowledgeListScope::Open,
        )
        .expect("pr view");
        assert_eq!(pr_view.kind, KnowledgeKind::Pr);
        assert!(!pr_view.refresh_enabled);
        assert_eq!(pr_view.detail.title, "PR Bridge");
        assert_eq!(pr_view.detail.state, "unavailable");
    }

    #[test]
    fn load_knowledge_bridge_builds_issue_and_spec_views_from_cache() {
        let _lock = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let repo = home.path().join("repo");
        init_repo(&repo);

        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("repo cache root");
        let cache = Cache::new(cache_root);
        cache
            .write_snapshot(&issue_snapshot(
                11,
                "Coverage bug",
                "Need more tests.",
                &["bug"],
                IssueState::Open,
            ))
            .expect("write issue snapshot");
        cache
            .write_snapshot(&spec_snapshot(22))
            .expect("write spec snapshot");
        write_issue_links(
            &repo,
            &[
                ("feature/coverage", 11),
                ("feature/coverage-followup", 11),
                ("spec/coverage", 22),
            ],
        );

        let issue_view = load_knowledge_bridge(
            &repo,
            KnowledgeKind::Issue,
            Some(11),
            false,
            KnowledgeListScope::Open,
        )
        .expect("issue bridge");
        let issue_entry = issue_view
            .entries
            .iter()
            .find(|entry| entry.number == 11)
            .expect("issue entry");
        assert_eq!(issue_entry.linked_branch_count, 2);
        assert_eq!(issue_view.selected_number, Some(11));
        assert_eq!(issue_view.detail.launch_issue_number, Some(11));
        assert!(issue_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "Description" && section.body == "Need more tests."));
        assert!(issue_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "Comment 1" && section.body == "Follow-up detail"));
        assert!(issue_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "Linked branches"
                && section.body == "- `feature/coverage`\n- `feature/coverage-followup`"));

        let spec_view = load_knowledge_bridge(
            &repo,
            KnowledgeKind::Spec,
            Some(22),
            false,
            KnowledgeListScope::Open,
        )
        .expect("spec bridge");
        let spec_entry = spec_view
            .entries
            .iter()
            .find(|entry| entry.number == 22)
            .expect("spec entry");
        assert_eq!(spec_entry.linked_branch_count, 1);
        assert!(spec_entry.meta.contains("phase/in-progress"));
        assert_eq!(spec_view.detail.launch_issue_number, Some(22));
        assert!(spec_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "spec"
                && section.body.contains("Raise project coverage")));
        assert!(spec_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "plan"));
        assert!(spec_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "tasks"));
        assert!(spec_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "notes"));
    }
}
