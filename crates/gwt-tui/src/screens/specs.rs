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
//! filter controls are follow-up work.

use ratatui::prelude::*;
use ratatui::widgets::*;
use std::path::PathBuf;

use crate::theme;

/// In-memory state for the Specs tab.
///
/// SPECs are refreshed from `~/.gwt/cache/issues/` by scanning every
/// sub-directory containing a `meta.json` file. This is intentionally a
/// simple filesystem walk rather than an index: the cache is the source of
/// truth and walking 10–100 directories is negligible.
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
        let dir = match std::fs::read_dir(&self.cache_root) {
            Ok(d) => d,
            Err(e) => {
                self.last_error = Some(format!("cache unavailable: {e}"));
                return;
            }
        };
        let mut items: Vec<SpecListItem> = Vec::new();
        for entry in dir.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(String::from) else {
                continue;
            };
            let Ok(number) = name.parse::<u64>() else {
                continue;
            };
            let meta_path = entry.path().join("meta.json");
            let Ok(meta_bytes) = std::fs::read(&meta_path) else {
                continue;
            };
            let Ok(meta): Result<serde_json::Value, _> = serde_json::from_slice(&meta_bytes) else {
                continue;
            };
            let title = meta
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let state = meta
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("open")
                .to_string();
            let labels: Vec<String> = meta
                .get("labels")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            items.push(SpecListItem {
                number,
                title,
                state,
                labels,
            });
        }
        items.sort_by(|a, b| b.number.cmp(&a.number));
        self.items = items;
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
    use std::fs;
    use tempfile::TempDir;

    fn write_meta(dir: &std::path::Path, number: u64, title: &str, state: &str, phase: &str) {
        let issue_dir = dir.join(number.to_string());
        fs::create_dir_all(&issue_dir).unwrap();
        let meta = serde_json::json!({
            "number": number,
            "title": title,
            "labels": ["gwt-spec", phase],
            "state": state,
            "updated_at": "t1",
            "comment_ids": []
        });
        fs::write(issue_dir.join("meta.json"), meta.to_string()).unwrap();
    }

    #[test]
    fn reload_reads_meta_json_entries() {
        let tmp = TempDir::new().unwrap();
        write_meta(tmp.path(), 1, "Alpha", "open", "phase/draft");
        write_meta(tmp.path(), 2, "Beta", "closed", "phase/done");
        write_meta(tmp.path(), 3, "Gamma", "open", "phase/implementation");

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
    fn reload_skips_non_numeric_dirs() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("not-an-issue")).unwrap();
        write_meta(tmp.path(), 42, "Only", "open", "phase/draft");
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
