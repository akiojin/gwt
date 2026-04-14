//! Specs tab: lists `gwt-spec` labeled GitHub Issues from the local cache.
//!
//! SPEC-12 Phase 9: this tab is the UI surface for SPECs now that they live
//! as GitHub Issues rather than worktree-local files. The rendering path is
//! strictly cache-only per #1920 US-12 FR-NEW-SPECS-002 — it never touches
//! the network directly. Cache updates flow through `gwt issue spec pull`
//! or the startup sync.
//!
//! Current scope (MVP): list SPECs from the local cache with number, title,
//! state, and phase label. Detail view, section switching, search, and
//! filter controls are follow-up work. Cache schema access flows through the
//! typed `crate::specs_cache` read model rather than screen-local fs parsing.

use ratatui::prelude::*;
use ratatui::widgets::*;
use std::path::PathBuf;

use crate::theme;

/// In-memory state for the Specs tab.
#[derive(Debug, Default, Clone)]
pub struct SpecsState {
    pub cache_root: PathBuf,
    pub items: Vec<SpecListItem>,
    pub selected: usize,
    pub last_error: Option<String>,
}

/// Compact SPEC summary rendered as one row in the Specs tab list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecListItem {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub labels: Vec<String>,
}

impl SpecsState {
    /// Construct an empty state rooted at `cache_root`.
    pub fn new(cache_root: PathBuf) -> Self {
        SpecsState {
            cache_root,
            items: Vec::new(),
            selected: 0,
            last_error: None,
        }
    }

    /// Reload the list from the cache directory. Missing/unreadable entries
    /// are silently skipped; on catastrophic errors (e.g. the cache root
    /// does not exist), `last_error` is populated but `items` is left
    /// unchanged.
    pub fn reload_from_cache(&mut self) {
        self.last_error = None;
        let items = match crate::specs_cache::load_specs(&self.cache_root) {
            Ok(items) => items,
            Err(err) => {
                self.last_error = Some(format!("cache unavailable: {err}"));
                return;
            }
        };
        self.items = items
            .into_iter()
            .map(|item| SpecListItem {
                number: item.number,
                title: item.title,
                state: item.state,
                labels: item.labels,
            })
            .collect();
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
    }
}

/// Render the Specs tab into `area`.
pub fn render(state: &SpecsState, frame: &mut Frame<'_>, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Specs ")
        .border_style(theme::style::muted_text());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(err) = &state.last_error {
        let p = Paragraph::new(format!(
            "{err}\n\nRun `gwt issue spec pull --all` to refresh."
        ))
        .style(theme::style::error_text());
        frame.render_widget(p, inner);
        return;
    }

    if state.items.is_empty() {
        let p = Paragraph::new(
            "No SPECs cached yet.\n\nRun `gwt issue spec pull --all` to populate the cache.",
        )
        .style(theme::style::muted_text());
        frame.render_widget(p, inner);
        return;
    }

    let rows: Vec<ListItem> = state
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let phase = item
                .labels
                .iter()
                .find(|l| l.starts_with("phase/"))
                .map(String::as_str)
                .unwrap_or("phase/?");
            let state_marker = match item.state.as_str() {
                "closed" => "🔴",
                _ => "🟢",
            };
            let is_selected = i == state.selected;
            let style = if is_selected {
                theme::style::selected_item()
            } else {
                theme::style::text()
            };
            let spans = Line::from(vec![
                Span::raw(format!("{state_marker} #{:<5} ", item.number)),
                Span::styled(format!("[{phase}] "), theme::style::muted_text()),
                Span::raw(item.title.clone()),
            ]);
            ListItem::new(spans).style(style)
        })
        .collect();

    let list = List::new(rows);
    frame.render_widget(list, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_spec(
        dir: &std::path::Path,
        number: u64,
        title: &str,
        state: gwt_github::IssueState,
        phase: &str,
    ) {
        gwt_github::Cache::new(dir.to_path_buf())
            .write_snapshot(&gwt_github::client::IssueSnapshot {
                number: gwt_github::IssueNumber(number),
                title: title.to_string(),
                body: format!(
                    "<!-- gwt-spec id={number} version=1 -->\n<!-- sections:\nspec=body\n-->\n<!-- artifact:spec BEGIN -->\nbody\n<!-- artifact:spec END -->\n"
                ),
                labels: vec!["gwt-spec".to_string(), phase.to_string()],
                state,
                updated_at: gwt_github::UpdatedAt::new("2026-04-12T00:00:00Z"),
                comments: vec![],
            })
            .unwrap();
    }

    fn write_plain_issue(dir: &std::path::Path, number: u64, title: &str) {
        gwt_github::Cache::new(dir.to_path_buf())
            .write_snapshot(&gwt_github::client::IssueSnapshot {
                number: gwt_github::IssueNumber(number),
                title: title.to_string(),
                body: "plain body".to_string(),
                labels: vec!["bug".to_string()],
                state: gwt_github::IssueState::Open,
                updated_at: gwt_github::UpdatedAt::new("2026-04-12T00:00:00Z"),
                comments: vec![],
            })
            .unwrap();
    }

    #[test]
    fn reload_reads_cached_spec_entries() {
        let tmp = TempDir::new().unwrap();
        write_spec(
            tmp.path(),
            1,
            "Alpha",
            gwt_github::IssueState::Open,
            "phase/draft",
        );
        write_spec(
            tmp.path(),
            2,
            "Beta",
            gwt_github::IssueState::Closed,
            "phase/done",
        );
        write_spec(
            tmp.path(),
            3,
            "Gamma",
            gwt_github::IssueState::Open,
            "phase/implementation",
        );

        let mut state = SpecsState::new(tmp.path().to_path_buf());
        state.reload_from_cache();
        assert_eq!(state.items.len(), 3);
        // Sorted by number descending.
        assert_eq!(state.items[0].number, 3);
        assert_eq!(state.items[1].number, 2);
        assert_eq!(state.items[2].number, 1);
        assert_eq!(state.items[0].title, "Gamma");
    }

    #[test]
    fn reload_skips_plain_issues_and_non_numeric_dirs() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("not-an-issue")).unwrap();
        write_plain_issue(tmp.path(), 7, "Plain issue");
        write_spec(
            tmp.path(),
            42,
            "Only",
            gwt_github::IssueState::Open,
            "phase/draft",
        );
        let mut state = SpecsState::new(tmp.path().to_path_buf());
        state.reload_from_cache();
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].number, 42);
    }

    #[test]
    fn reload_records_missing_cache_root_as_error() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("nope");
        let mut state = SpecsState::new(missing);
        state.reload_from_cache();
        assert!(state.last_error.is_some());
        assert!(state.items.is_empty());
    }
}
