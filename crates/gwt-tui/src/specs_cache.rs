use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedSpecSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub labels: Vec<String>,
}

pub fn load_specs(cache_root: &Path) -> Result<Vec<CachedSpecSummary>, String> {
    let cache = gwt_github::Cache::new(cache_root.to_path_buf());
    let mut specs: Vec<CachedSpecSummary> = cache
        .list_entries()
        .map_err(|err| err.to_string())?
        .into_iter()
        .filter(|entry| {
            entry
                .snapshot
                .labels
                .iter()
                .any(|label| label == "gwt-spec")
        })
        .map(|entry| CachedSpecSummary {
            number: entry.snapshot.number.0,
            title: entry.snapshot.title,
            state: match entry.snapshot.state {
                gwt_github::IssueState::Open => "open".to_string(),
                gwt_github::IssueState::Closed => "closed".to_string(),
            },
            labels: entry.snapshot.labels,
        })
        .collect();
    specs.sort_by(|left, right| right.number.cmp(&left.number));
    Ok(specs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_github::client::{IssueNumber, IssueSnapshot, IssueState, UpdatedAt};

    #[test]
    fn load_specs_uses_gwt_github_cache_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache = gwt_github::Cache::new(tmp.path().to_path_buf());
        cache
            .write_snapshot(&IssueSnapshot {
                number: IssueNumber(3),
                title: "Specs tab item".to_string(),
                body: "<!-- gwt-spec id=3 version=1 -->\n<!-- sections:\nspec=body\n-->\n<!-- artifact:spec BEGIN -->\nbody\n<!-- artifact:spec END -->\n".to_string(),
                labels: vec!["gwt-spec".to_string(), "phase/implementation".to_string()],
                state: IssueState::Open,
                updated_at: UpdatedAt::new("2026-04-12T00:00:00Z"),
                comments: vec![],
            })
            .expect("write spec snapshot");
        cache
            .write_snapshot(&IssueSnapshot {
                number: IssueNumber(4),
                title: "Plain issue".to_string(),
                body: "plain body".to_string(),
                labels: vec!["bug".to_string()],
                state: IssueState::Closed,
                updated_at: UpdatedAt::new("2026-04-12T00:00:00Z"),
                comments: vec![],
            })
            .expect("write plain issue snapshot");

        let specs = load_specs(tmp.path()).expect("load cached specs");

        assert_eq!(
            specs,
            vec![CachedSpecSummary {
                number: 3,
                title: "Specs tab item".to_string(),
                state: "open".to_string(),
                labels: vec!["gwt-spec".to_string(), "phase/implementation".to_string()],
            }]
        );
    }
}
