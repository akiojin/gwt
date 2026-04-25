use std::{collections::HashMap, fs};

use gwt::{load_knowledge_bridge, KnowledgeKind, KnowledgeListScope};
use gwt_core::{paths::gwt_cache_dir, repo_hash::detect_repo_hash};
use gwt_github::{
    Cache, CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt,
};
use tempfile::tempdir;

fn sample_issue(
    number: u64,
    title: &str,
    labels: &[&str],
    body: &str,
    updated_at: &str,
    state: IssueState,
) -> IssueSnapshot {
    IssueSnapshot {
        number: IssueNumber(number),
        title: title.to_string(),
        body: body.to_string(),
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
        state,
        updated_at: UpdatedAt::new(updated_at),
        comments: vec![CommentSnapshot {
            id: CommentId(number * 10),
            body: format!("Comment for #{number}"),
            updated_at: UpdatedAt::new(updated_at),
        }],
    }
}

#[test]
fn load_knowledge_bridge_filters_plain_issues_and_counts_linked_branches() {
    let dir = tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");
    fs::create_dir_all(&repo_path).expect("create repo");
    init_repo(&repo_path);

    let cache = Cache::new(issue_cache_root(&repo_path));
    cache
        .write_snapshot(&sample_issue(
            42,
            "Issue bridge",
            &["bug"],
            "Issue body",
            "2026-04-20T10:00:00Z",
            IssueState::Open,
        ))
        .expect("write issue");
    cache
        .write_snapshot(&sample_issue(
            2017,
            "SPEC bridge",
            &["gwt-spec", "phase/draft"],
            concat!(
                "<!-- gwt-spec id=2017 version=1 -->\n",
                "<!-- sections:\n",
                "spec=body\n",
                "-->\n\n",
                "<!-- artifact:spec BEGIN -->\n",
                "# Spec body\n",
                "<!-- artifact:spec END -->\n"
            ),
            "2026-04-19T09:00:00Z",
            IssueState::Open,
        ))
        .expect("write spec");

    write_issue_link_store(
        &repo_path,
        HashMap::from([("feature/issue-bridge".to_string(), 42)]),
    );

    let loaded = load_knowledge_bridge(
        &repo_path,
        KnowledgeKind::Issue,
        Some(42),
        false,
        KnowledgeListScope::Open,
    )
    .expect("load");

    assert_eq!(loaded.kind, KnowledgeKind::Issue);
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].number, 42);
    assert_eq!(loaded.entries[0].linked_branch_count, 1);
    assert_eq!(loaded.selected_number, Some(42));
    assert_eq!(loaded.detail.launch_issue_number, Some(42));
    assert!(loaded
        .detail
        .sections
        .iter()
        .any(|section| section.title == "Linked branches"
            && section.body.contains("feature/issue-bridge")));
}

#[test]
fn load_knowledge_bridge_filters_specs_and_exposes_cached_sections() {
    let dir = tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");
    fs::create_dir_all(&repo_path).expect("create repo");
    init_repo(&repo_path);

    let cache = Cache::new(issue_cache_root(&repo_path));
    cache
        .write_snapshot(&sample_issue(
            2017,
            "SPEC-2017: Knowledge Bridge",
            &["gwt-spec", "phase/draft"],
            concat!(
                "<!-- gwt-spec id=2017 version=1 -->\n",
                "<!-- sections:\n",
                "spec=body\n",
                "tasks=body\n",
                "-->\n\n",
                "<!-- artifact:spec BEGIN -->\n",
                "# Spec body\n",
                "## Summary\n",
                "Cache-backed issue view\n",
                "<!-- artifact:spec END -->\n\n",
                "<!-- artifact:tasks BEGIN -->\n",
                "- [ ] T-001\n",
                "<!-- artifact:tasks END -->\n"
            ),
            "2026-04-20T10:00:00Z",
            IssueState::Open,
        ))
        .expect("write spec");

    let loaded = load_knowledge_bridge(
        &repo_path,
        KnowledgeKind::Spec,
        Some(2017),
        false,
        KnowledgeListScope::Open,
    )
    .expect("load");

    assert_eq!(loaded.kind, KnowledgeKind::Spec);
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].number, 2017);
    assert!(
        loaded.entries[0].meta.contains("phase/draft"),
        "expected phase label in list metadata"
    );
    assert!(
        loaded
            .detail
            .sections
            .iter()
            .any(|section| section.title == "spec"
                && section.body.contains("Cache-backed issue view")),
        "sections: {:?}",
        loaded.detail.sections
    );
}

#[test]
fn load_knowledge_bridge_returns_disabled_pr_surface_until_cache_support_exists() {
    let dir = tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");
    fs::create_dir_all(&repo_path).expect("create repo");
    init_repo(&repo_path);

    let loaded = load_knowledge_bridge(
        &repo_path,
        KnowledgeKind::Pr,
        None,
        false,
        KnowledgeListScope::Open,
    )
    .expect("load");

    assert_eq!(loaded.kind, KnowledgeKind::Pr);
    assert!(loaded.entries.is_empty());
    assert!(!loaded.refresh_enabled);
    assert!(loaded
        .empty_message
        .as_deref()
        .is_some_and(|message| message.contains("cache-backed PR list support")));
    assert!(loaded
        .detail
        .sections
        .iter()
        .any(|section| section.body.contains("cache-backed PR list support")));
}

#[test]
fn load_knowledge_bridge_separates_open_and_closed_issue_lists() {
    let dir = tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");
    fs::create_dir_all(&repo_path).expect("create repo");
    init_repo(&repo_path);

    let cache = Cache::new(issue_cache_root(&repo_path));
    cache
        .write_snapshot(&sample_issue(
            42,
            "Open issue",
            &["bug"],
            "Open issue body",
            "2026-04-20T10:00:00Z",
            IssueState::Open,
        ))
        .expect("write open issue");
    cache
        .write_snapshot(&sample_issue(
            43,
            "Closed issue",
            &["bug"],
            "Closed issue body",
            "2026-04-19T10:00:00Z",
            IssueState::Closed,
        ))
        .expect("write closed issue");

    let open_view = load_knowledge_bridge(
        &repo_path,
        KnowledgeKind::Issue,
        None,
        false,
        KnowledgeListScope::Open,
    )
    .expect("load open issues");
    assert_eq!(open_view.entries.len(), 1);
    assert_eq!(open_view.entries[0].number, 42);
    assert_eq!(open_view.detail.number, Some(42));

    let closed_view = load_knowledge_bridge(
        &repo_path,
        KnowledgeKind::Issue,
        None,
        false,
        KnowledgeListScope::Closed,
    )
    .expect("load closed issues");
    assert_eq!(closed_view.entries.len(), 1);
    assert_eq!(closed_view.entries[0].number, 43);
    assert_eq!(closed_view.detail.number, Some(43));
}

fn init_repo(repo_path: &std::path::Path) {
    let remote = format!(
        "https://github.com/example/repo-{:x}.git",
        remote_suffix(repo_path)
    );
    for args in [
        ["init", "-q"].as_slice(),
        ["remote", "add", "origin", remote.as_str()].as_slice(),
    ] {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn remote_suffix(repo_path: &std::path::Path) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    repo_path.display().to_string().hash(&mut hasher);
    hasher.finish()
}

fn write_issue_link_store(repo_path: &std::path::Path, branches: HashMap<String, u64>) {
    let repo_hash = detect_repo_hash(repo_path).expect("repo hash");
    let path = gwt_cache_dir()
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    fs::create_dir_all(path.parent().expect("parent")).expect("create link dir");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({ "branches": branches }))
            .expect("serialize store"),
    )
    .expect("write link store");
}

fn issue_cache_root(repo_path: &std::path::Path) -> std::path::PathBuf {
    let repo_hash = detect_repo_hash(repo_path).expect("repo hash");
    gwt_cache_dir().join("issues").join(repo_hash.as_str())
}
