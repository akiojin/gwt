//! TUI Application with Elm Architecture (Model / View / Update)

use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::Terminal;

use crate::event::{self, EventLoop, TuiEvent};
use crate::input::keybind::{self, KeyAction, PrefixState};
use crate::message::Message;
use crate::model::{
    ActiveLayer, ErrorEntry, ErrorSeverity, ManagementTab, Model, OverlayMode, SelectionPoint,
    SessionLayoutMode, TerminalViewportState,
};
use crate::screens::{self, LogsMessage, SettingsMessage};
use crate::widgets;

/// Tick interval for background polling.
const TICK_INTERVAL: Duration = Duration::from_millis(250);

#[cfg(test)]
thread_local! {
    static TEST_CLIPBOARD: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn content_area_rect(cols: u16, rows: u16) -> Rect {
    let area = Rect::new(0, 0, cols, rows);
    let layout = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);
    layout[2]
}

fn format_issue_detail_markdown(issue: &gwt_core::git::GitHubIssue) -> String {
    let mut lines = vec![format!("# Issue #{}: {}", issue.number, issue.title)];
    lines.push(String::new());
    lines.push(format!("- State: `{}`", issue.state));
    if !issue.updated_at.trim().is_empty() {
        lines.push(format!("- Updated: `{}`", issue.updated_at));
    }
    if !issue.labels.is_empty() {
        let labels = issue
            .labels
            .iter()
            .map(|label| format!("`{}`", label.name))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("- Labels: {labels}"));
    }
    if !issue.html_url.trim().is_empty() {
        lines.push(format!("- URL: {}", issue.html_url));
    }
    if let Some(body) = issue.body.as_deref() {
        if !body.trim().is_empty() {
            lines.push(String::new());
            lines.push(body.trim().to_string());
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn load_issue_detail_markdown(repo_root: &Path, issue_number: u64) -> String {
    match gwt_core::git::fetch_issue_detail(repo_root, issue_number) {
        Ok(issue) => format_issue_detail_markdown(&issue),
        Err(error) => format!(
            "## GitHub Issue\nCould not load issue #{issue_number}.\n\n- Reason: `{error}`\n\nRun `gh issue view {issue_number}` for details.\n"
        ),
    }
}

fn normalize_branch_name_for_match(name: &str) -> &str {
    if let Some(stripped) = name.strip_prefix("remotes/") {
        if let Some((_, rest)) = stripped.split_once('/') {
            return rest;
        }
        return stripped;
    }

    if let Some(stripped) = name.strip_prefix("origin/") {
        return stripped;
    }
    if let Some(stripped) = name.strip_prefix("upstream/") {
        return stripped;
    }

    name
}

fn matching_session_indices(model: &Model, branch_name: &str) -> Vec<usize> {
    let normalized = normalize_branch_name_for_match(branch_name);
    model
        .session_tabs
        .iter()
        .enumerate()
        .filter_map(|(index, tab)| {
            let tab_branch = tab.branch.as_deref()?;
            (normalize_branch_name_for_match(tab_branch) == normalized).then_some(index)
        })
        .collect()
}

fn open_branch_wizard(
    model: &mut Model,
    branch_name: &str,
    worktree_path: Option<&str>,
    include_history: bool,
) {
    let history = if include_history {
        load_quick_start_history(&model.repo_root, branch_name, worktree_path.map(Path::new))
    } else {
        vec![]
    };
    model.wizard = Some(crate::screens::wizard::WizardState::open_for_branch(
        branch_name,
        history,
    ));
}

fn open_branch_session_selector(
    model: &mut Model,
    branch_name: &str,
    worktree_path: Option<String>,
    session_indices: Vec<usize>,
) {
    let mut options = Vec::with_capacity(session_indices.len() + 2);
    for index in session_indices {
        if let Some(session) = model.session_tabs.get(index) {
            options.push(crate::screens::branch_session_selector::BranchSessionOption {
                label: format!("Open {}", session.name),
                choice:
                    crate::screens::branch_session_selector::BranchSessionSelectorChoice::ExistingSession(
                        index,
                    ),
            });
        }
    }
    options.push(
        crate::screens::branch_session_selector::BranchSessionOption {
            label: "Add session".to_string(),
            choice:
                crate::screens::branch_session_selector::BranchSessionSelectorChoice::AddSession,
        },
    );
    options.push(
        crate::screens::branch_session_selector::BranchSessionOption {
            label: "Full wizard".to_string(),
            choice:
                crate::screens::branch_session_selector::BranchSessionSelectorChoice::FullWizard,
        },
    );

    model.branch_session_selector = Some(
        crate::screens::branch_session_selector::BranchSessionSelectorState::new(
            branch_name.to_string(),
            worktree_path,
            options,
        ),
    );
    model.overlay_mode = OverlayMode::BranchSessionSelector;
}

fn spec_default_branch_name(spec_id: &str) -> String {
    format!("feature/feature-{}", spec_id.trim_start_matches("SPEC-"))
}

fn open_spec_launch(
    model: &mut Model,
    spec_id: &str,
    branch_name: &str,
    is_new_branch: bool,
    worktree_path: Option<&Path>,
) {
    let history = if is_new_branch {
        vec![]
    } else {
        load_quick_start_history(&model.repo_root, branch_name, worktree_path)
    };
    model.wizard = Some(crate::screens::wizard::WizardState::open_for_spec(
        spec_id,
        branch_name,
        is_new_branch,
        history,
    ));
}

fn open_issue_launch(model: &mut Model, issue_number: u64) {
    let existing_branch = gwt_core::git::find_branch_for_issue(&model.repo_root, issue_number)
        .ok()
        .flatten();
    let branch_name = existing_branch
        .clone()
        .unwrap_or_else(|| gwt_core::git::generate_branch_name("feature/", issue_number));
    let history = if existing_branch.is_some() {
        load_quick_start_history(&model.repo_root, &branch_name, None)
    } else {
        vec![]
    };
    model.wizard = Some(crate::screens::wizard::WizardState::open_for_issue(
        issue_number,
        &branch_name,
        existing_branch.is_none(),
        history,
    ));
}

fn spawn_management_data_preload(
    repo_root: PathBuf,
) -> mpsc::Receiver<crate::model::ManagementDataUpdate> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let issues = crate::screens::issues::load_issues(&repo_root);
        let specs = crate::screens::specs::load_specs(&repo_root);
        let versions = crate::screens::versions::load_tags(&repo_root);
        let logs = crate::screens::logs::load_log_entries(&repo_root);
        let _ = tx.send(crate::model::ManagementDataUpdate {
            issues,
            specs,
            versions,
            logs,
        });
    });
    rx
}

fn spawn_branch_list_enrichment(
    repo_root: PathBuf,
) -> mpsc::Receiver<crate::model::BranchListUpdate> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let branches = crate::screens::branches::load_branches_enriched(&repo_root);
        let _ = tx.send(crate::model::BranchListUpdate { branches });
    });
    rx
}

fn wants_mouse_capture(model: &Model) -> bool {
    model.active_layer == ActiveLayer::Management
        || (model.active_layer == ActiveLayer::Main && !model.session_tabs.is_empty())
}

fn active_pane_id(model: &Model) -> Option<&str> {
    model
        .session_tabs
        .get(model.active_session)
        .map(|tab| tab.pane_id.as_str())
}

fn active_terminal_parser<'a>(model: &'a Model, pane_id: &str) -> Option<&'a vt100::Parser> {
    if model.active_history_pane_id.as_deref() == Some(pane_id) {
        return model
            .active_history_parser
            .as_ref()
            .or_else(|| model.vt_parsers.get(pane_id));
    }
    model.vt_parsers.get(pane_id)
}

fn build_history_view_parser(model: &mut Model, pane_id: &str) -> Option<(vt100::Parser, usize)> {
    let viewport = content_area_rect(model.terminal_cols, model.terminal_rows);
    let rows = viewport.height.max(1);
    let cols = viewport.width.max(1);

    let pane = model.pane_manager.pane_mut_by_id(pane_id)?;
    let raw = match pane.read_scrollback_raw() {
        Ok(raw) if !raw.is_empty() => raw,
        Ok(_) => return None,
        Err(error) => {
            tracing::warn!(
                message = "flow_failure",
                category = "ui",
                event = "load_copy_history_parser",
                result = "failure",
                workspace = "default",
                pane_id,
                error_code = "SCROLLBACK_READ_FAILED",
                error_detail = %error,
            );
            return None;
        }
    };

    let line_count = raw.iter().filter(|&&byte| byte == b'\n').count();
    let parser_rows = line_count
        .saturating_add(usize::from(rows))
        .saturating_add(256)
        .clamp(usize::from(rows), usize::from(u16::MAX)) as u16;

    let mut parser = vt100::Parser::new(parser_rows, cols, 0);
    parser.process(&raw);
    let (cursor_row, _) = parser.screen().cursor_position();
    let content_rows = usize::from(cursor_row)
        .saturating_add(1)
        .max(usize::from(rows));
    let max_scrollback = content_rows.saturating_sub(usize::from(rows));
    Some((parser, max_scrollback))
}

fn equal_grid_layout(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![area];
    }

    let columns = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(columns);
    let row_constraints = vec![Constraint::Ratio(1, rows as u32); rows];
    let col_constraints = vec![Constraint::Ratio(1, columns as u32); columns];

    let mut rects = Vec::with_capacity(count);
    for row in Layout::vertical(row_constraints)
        .split(area)
        .iter()
        .copied()
    {
        for cell in Layout::horizontal(col_constraints.clone())
            .split(row)
            .iter()
            .copied()
        {
            rects.push(cell);
            if rects.len() == count {
                return rects;
            }
        }
    }
    rects
}

fn render_session_grid(model: &Model, buf: &mut Buffer, area: Rect) {
    let rects = equal_grid_layout(area, model.session_tabs.len());
    for (index, (tab, rect)) in model.session_tabs.iter().zip(rects.into_iter()).enumerate() {
        let block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(if index == model.active_session {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            })
            .title(format!(" {} ", tab.name));
        let inner = block.inner(rect);
        ratatui::widgets::Widget::render(block, rect, buf);

        if inner.width == 0 || inner.height == 0 {
            continue;
        }

        let parser = active_terminal_parser(model, &tab.pane_id);
        let view = model.terminal_viewport(&tab.pane_id);
        let selection = view.and_then(|view| view.selection_anchor.zip(view.selection_focus));
        if model.active_history_pane_id.as_deref() == Some(tab.pane_id.as_str())
            && model.active_history_parser.is_some()
        {
            if let Some(parser) = parser {
                let _ = crate::screens::agent_pane::render_history(
                    buf,
                    inner,
                    parser,
                    view.map(terminal_view_origin).unwrap_or_default(),
                    selection,
                );
            }
        } else {
            let _ = crate::screens::agent_pane::render(buf, inner, parser, selection);
        }
    }
}

fn selection_active(view: &TerminalViewportState) -> bool {
    view.dragging || view.selection_anchor.is_some() || view.selection_focus.is_some()
}

fn active_terminal_parser_mut<'a>(
    model: &'a mut Model,
    pane_id: &str,
) -> Option<&'a mut vt100::Parser> {
    if model.active_history_pane_id.as_deref() == Some(pane_id)
        && model.active_history_parser.is_some()
    {
        return model.active_history_parser.as_mut();
    }
    model.vt_parsers.get_mut(pane_id)
}

fn sync_active_terminal_history(model: &mut Model) {
    let Some(pane_id) = active_pane_id(model).map(str::to_string) else {
        model.clear_active_history_view();
        return;
    };

    if model.active_layer != ActiveLayer::Main {
        model.clear_active_history_view();
        return;
    }

    let should_use_history = model
        .terminal_viewport(&pane_id)
        .is_some_and(|view| !view.follow_live || selection_active(view));

    if !should_use_history {
        model.clear_active_history_view();
        return;
    }

    if model.active_history_pane_id.as_deref() != Some(pane_id.as_str())
        || model.active_history_parser.is_none()
    {
        if let Some((parser, max_scrollback)) = build_history_view_parser(model, &pane_id) {
            model.active_history_pane_id = Some(pane_id.clone());
            model.active_history_parser = Some(parser);
            let view = model.terminal_viewport_mut(&pane_id);
            view.max_scrollback = max_scrollback;
            view.scrollback = view.scrollback.min(max_scrollback);
        } else {
            model.clear_active_history_view();
        }
    }

    if let Some(parser) = model.active_history_parser.as_ref() {
        let area = content_area_rect(model.terminal_cols, model.terminal_rows);
        let max_scrollback = history_parser_max_scrollback(parser, area.height.max(1));
        let view = model.terminal_viewport_mut(&pane_id);
        view.max_scrollback = max_scrollback;
        view.scrollback = view.scrollback.min(max_scrollback);
    }
}

fn set_terminal_follow_live(model: &mut Model, pane_id: &str, follow_live: bool) {
    {
        let view = model.terminal_viewport_mut(pane_id);
        view.follow_live = follow_live;
        if follow_live {
            view.scrollback = 0;
            view.max_scrollback = 0;
        }
    }
    sync_active_terminal_history(model);
}

fn jump_terminal_to_live(model: &mut Model, pane_id: &str) {
    {
        let view = model.terminal_viewport_mut(pane_id);
        view.selection_anchor = None;
        view.selection_focus = None;
        view.dragging = false;
    }
    set_terminal_follow_live(model, pane_id, true);
}

fn freeze_terminal_view(model: &mut Model, pane_id: &str) {
    {
        let view = model.terminal_viewport_mut(pane_id);
        view.follow_live = false;
    }
    sync_active_terminal_history(model);
}

fn set_terminal_scrollback(model: &mut Model, pane_id: &str, desired: usize) {
    let should_follow_live = desired == 0
        && !model
            .terminal_viewport(pane_id)
            .is_some_and(selection_active);
    if should_follow_live {
        set_terminal_follow_live(model, pane_id, true);
        return;
    }

    freeze_terminal_view(model, pane_id);
    let max_scrollback = model
        .terminal_viewport(pane_id)
        .map(|view| view.max_scrollback)
        .unwrap_or(0);
    let view = model.terminal_viewport_mut(pane_id);
    view.scrollback = desired.min(max_scrollback);
    view.follow_live = false;
}

fn adjust_terminal_scrollback(model: &mut Model, pane_id: &str, delta: isize) {
    if delta == 0 {
        return;
    }

    if delta.is_positive() {
        freeze_terminal_view(model, pane_id);
    }

    let current_scrollback = model
        .terminal_viewport(pane_id)
        .map(|view| view.scrollback)
        .unwrap_or(0);
    let desired = if delta.is_negative() {
        current_scrollback.saturating_sub(delta.unsigned_abs())
    } else {
        let max_scrollback = model
            .terminal_viewport(pane_id)
            .map(|view| view.max_scrollback)
            .unwrap_or(0);
        current_scrollback
            .saturating_add(delta as usize)
            .min(max_scrollback)
    };
    set_terminal_scrollback(model, pane_id, desired);
}

fn scroll_terminal_to_top(model: &mut Model, pane_id: &str) {
    freeze_terminal_view(model, pane_id);
    let max_scrollback = model
        .terminal_viewport(pane_id)
        .map(|view| view.max_scrollback)
        .unwrap_or(0);
    set_terminal_scrollback(model, pane_id, max_scrollback);
}

fn scroll_terminal_to_bottom(model: &mut Model, pane_id: &str) {
    set_terminal_scrollback(model, pane_id, 0);
}

fn clamp_point(point: SelectionPoint, rows: u16, cols: u16) -> SelectionPoint {
    SelectionPoint {
        row: point.row.min(rows.saturating_sub(1)),
        col: point.col.min(cols.saturating_sub(1)),
    }
}

fn main_area_point(model: &Model, mouse: MouseEvent) -> Option<SelectionPoint> {
    let area = content_area_rect(model.terminal_cols, model.terminal_rows);
    if mouse.column < area.x
        || mouse.column >= area.right()
        || mouse.row < area.y
        || mouse.row >= area.bottom()
    {
        return None;
    }
    Some(SelectionPoint {
        row: mouse.row.saturating_sub(area.y),
        col: mouse.column.saturating_sub(area.x),
    })
}

fn terminal_view_origin(view: &TerminalViewportState) -> u16 {
    view.max_scrollback
        .saturating_sub(view.scrollback)
        .min(usize::from(u16::MAX)) as u16
}

fn history_parser_max_scrollback(parser: &vt100::Parser, viewport_rows: u16) -> usize {
    let (cursor_row, _) = parser.screen().cursor_position();
    usize::from(cursor_row)
        .saturating_add(1)
        .max(usize::from(viewport_rows))
        .saturating_sub(usize::from(viewport_rows))
}

fn terminal_view_size(model: &Model, pane_id: &str) -> Option<(u16, u16)> {
    if model.active_history_pane_id.as_deref() == Some(pane_id)
        && model.active_history_parser.is_some()
    {
        let area = content_area_rect(model.terminal_cols, model.terminal_rows);
        return Some((area.height.max(1), area.width.max(1)));
    }
    active_terminal_parser(model, pane_id).map(|parser| parser.screen().size())
}

fn viewport_point_to_absolute(
    model: &Model,
    pane_id: &str,
    point: SelectionPoint,
) -> SelectionPoint {
    let row = if let Some(view) = model.terminal_viewport(pane_id) {
        point.row.saturating_add(terminal_view_origin(view))
    } else {
        point.row
    };
    SelectionPoint {
        row,
        col: point.col,
    }
}

fn copy_current_selection(model: &mut Model, pane_id: &str) {
    let Some(view) = model.terminal_viewport(pane_id) else {
        return;
    };
    let (Some(anchor), Some(focus)) = (view.selection_anchor, view.selection_focus) else {
        return;
    };
    let Some(parser) = active_terminal_parser(model, pane_id) else {
        return;
    };
    let text = crate::screens::agent_pane::selected_text(parser, anchor, focus);
    if text.is_empty() {
        return;
    }
    if let Err(error) = copy_text_to_clipboard(&text) {
        model.push_error(ErrorEntry {
            message: format!("Clipboard copy failed: {error}"),
            severity: ErrorSeverity::Minor,
        });
    }
}

fn handle_terminal_view_key(model: &mut Model, key: crossterm::event::KeyEvent) -> bool {
    if model.active_layer != ActiveLayer::Main {
        return false;
    }

    let Some(pane_id) = active_pane_id(model).map(str::to_string) else {
        return false;
    };
    let page = usize::from(model.terminal_rows.saturating_sub(4).max(1));

    match key.code {
        crossterm::event::KeyCode::PageUp => {
            adjust_terminal_scrollback(model, &pane_id, page as isize);
            true
        }
        crossterm::event::KeyCode::PageDown => {
            adjust_terminal_scrollback(model, &pane_id, -(page as isize));
            true
        }
        crossterm::event::KeyCode::Home => {
            scroll_terminal_to_top(model, &pane_id);
            true
        }
        crossterm::event::KeyCode::End => {
            scroll_terminal_to_bottom(model, &pane_id);
            true
        }
        crossterm::event::KeyCode::Esc => {
            let had_selection = model
                .terminal_viewport(&pane_id)
                .is_some_and(selection_active);
            if had_selection {
                let view = model.terminal_viewport_mut(&pane_id);
                view.selection_anchor = None;
                view.selection_focus = None;
                view.dragging = false;
                if view.scrollback == 0 {
                    view.follow_live = true;
                    view.max_scrollback = 0;
                }
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn handle_terminal_view_mouse(model: &mut Model, mouse: MouseEvent) -> bool {
    if model.active_layer != ActiveLayer::Main {
        return false;
    }

    let Some(pane_id) = active_pane_id(model).map(str::to_string) else {
        return false;
    };
    let Some((rows, cols)) = terminal_view_size(model, &pane_id) else {
        return false;
    };

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            adjust_terminal_scrollback(model, &pane_id, 1);
            true
        }
        MouseEventKind::ScrollDown => {
            adjust_terminal_scrollback(model, &pane_id, -1);
            true
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(point) = main_area_point(model, mouse) {
                let point = clamp_point(point, rows, cols);
                freeze_terminal_view(model, &pane_id);
                let absolute = viewport_point_to_absolute(model, &pane_id, point);
                let view = model.terminal_viewport_mut(&pane_id);
                view.selection_anchor = Some(absolute);
                view.selection_focus = Some(absolute);
                view.dragging = true;
                return true;
            }
            false
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(point) = main_area_point(model, mouse) {
                let point = clamp_point(point, rows, cols);
                let absolute = viewport_point_to_absolute(model, &pane_id, point);
                if let Some(view) = model.terminal_viewport(&pane_id) {
                    if !view.dragging {
                        return false;
                    }
                }
                let view = model.terminal_viewport_mut(&pane_id);
                view.selection_focus = Some(absolute);
                return true;
            }
            false
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some(point) = main_area_point(model, mouse) {
                let point = clamp_point(point, rows, cols);
                let absolute = viewport_point_to_absolute(model, &pane_id, point);
                let should_copy = model.terminal_viewport(&pane_id).is_some_and(|view| {
                    view.dragging
                        && view
                            .selection_anchor
                            .is_some_and(|anchor| anchor != absolute)
                });
                let return_to_live = model
                    .terminal_viewport(&pane_id)
                    .is_some_and(|view| view.scrollback == 0);
                {
                    let view = model.terminal_viewport_mut(&pane_id);
                    if view.dragging {
                        view.selection_focus = Some(absolute);
                    }
                    view.dragging = false;
                }
                if should_copy {
                    copy_current_selection(model, &pane_id);
                }
                let view = model.terminal_viewport_mut(&pane_id);
                view.selection_anchor = None;
                view.selection_focus = None;
                if return_to_live {
                    view.follow_live = true;
                    view.max_scrollback = 0;
                }
                return true;
            }
            false
        }
        _ => false,
    }
}

fn copy_text_to_clipboard(text: &str) -> Result<(), String> {
    #[cfg(test)]
    {
        TEST_CLIPBOARD.with(|storage| storage.borrow_mut().push(text.to_string()));
        Ok(())
    }

    #[cfg(not(test))]
    {
        let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
        clipboard
            .set_text(text.to_string())
            .map_err(|error| error.to_string())
    }
}

fn activate_management_tab(model: &mut Model, tab: ManagementTab) {
    model.management_tab = tab;
    if matches!(tab, ManagementTab::Profiles) {
        model.settings_state.category = crate::screens::settings::SettingsCategory::Environment;
    }
}

fn write_bytes_to_active_pane(model: &mut Model, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }

    if let Some(session) = model.session_tabs.get(model.active_session) {
        let pane_id = session.pane_id.clone();
        if let Some(pane) = model.pane_manager.pane_mut_by_id(&pane_id) {
            if let Err(error) = pane.write_input(bytes) {
                if let Some(active) = model.session_tabs.get_mut(model.active_session) {
                    active.status =
                        crate::model::SessionStatus::Error(format!("PTY write failed: {error}"));
                }
            }
        }
    }
}

#[cfg(test)]
fn clear_test_clipboard() {
    TEST_CLIPBOARD.with(|storage| storage.borrow_mut().clear());
}

#[cfg(test)]
fn take_test_clipboard() -> Vec<String> {
    TEST_CLIPBOARD.with(|storage| storage.take())
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Apply a message to the model (Elm Architecture update function).
pub fn update(model: &mut Model, msg: Message) {
    match msg {
        Message::Quit => {
            let agent_count = model.running_agent_count();
            if agent_count > 0 && model.confirm.is_none() {
                model.confirm = Some(
                    crate::screens::confirm::ConfirmState::exit_with_running_agents(agent_count),
                );
                model.overlay_mode = OverlayMode::Confirm;
            } else {
                model.should_quit = true;
            }
        }
        Message::ToggleLayer => {
            // Block layer toggle during Initialization (modal)
            if model.active_layer == ActiveLayer::Initialization {
                return;
            }
            model.toggle_layer();
            tracing::info!(
                message = "flow_success",
                category = "ui",
                event = "toggle_management_layer",
                result = "success",
                workspace = "default",
                active_layer = ?model.active_layer,
            );
        }
        Message::SwitchManagementTab(tab) => {
            activate_management_tab(model, tab);
            model.active_layer = ActiveLayer::Management;
            tracing::info!(
                message = "flow_success",
                category = "ui",
                event = "switch_management_tab",
                result = "success",
                workspace = "default",
                tab = model.management_tab.label(),
            );
        }
        Message::NextSession => {
            model.next_session();
            if !model.session_tabs.is_empty() {
                model.active_layer = ActiveLayer::Main;
            }
        }
        Message::PrevSession => {
            model.prev_session();
            if !model.session_tabs.is_empty() {
                model.active_layer = ActiveLayer::Main;
            }
        }
        Message::SwitchSession(index) => {
            // 1-based → 0-based
            let idx = index.saturating_sub(1);
            model.switch_session(idx);
            if idx < model.session_tabs.len() {
                model.active_layer = ActiveLayer::Main;
            }
        }
        Message::ToggleSessionLayout => {
            model.toggle_session_layout_mode();
        }
        Message::CloseSession => {
            model.close_active_session();
        }
        Message::NewShell => {
            if let Err(e) = spawn_shell_session(model) {
                model.push_error(ErrorEntry {
                    message: format!("Failed to spawn shell: {e}"),
                    severity: ErrorSeverity::Critical,
                });
            }
        }
        Message::TogglePtyCopyMode => {
            // Keep the legacy shortcut as a "jump back to live tail" alias.
            if let Some(pane_id) = active_pane_id(model).map(str::to_string) {
                jump_terminal_to_live(model, &pane_id);
            }
        }
        Message::WizardKey(key) => {
            use crossterm::event::KeyCode;
            if let Some(ref mut wiz) = model.wizard {
                match key.code {
                    KeyCode::Up => wiz.select_prev(),
                    KeyCode::Down => wiz.select_next(),
                    KeyCode::Enter => {
                        let action = wiz.confirm();
                        match action {
                            crate::screens::wizard::WizardAction::Complete => {
                                // Build config from wizard and launch agent
                                let launch_result = wiz.build_launch_config();
                                model.wizard = None;
                                match launch_result {
                                    Ok(config) => {
                                        // SPEC-1786: Codex hooks confirmation
                                        if check_codex_hooks_confirm(model, &config) {
                                            // Dialog shown; launch deferred
                                        } else if let Err(e) =
                                            spawn_agent_session(model, &config, false)
                                        {
                                            model.push_error(ErrorEntry {
                                                message: format!("Failed to launch agent: {e}"),
                                                severity: ErrorSeverity::Critical,
                                            });
                                        }
                                    }
                                    Err(e) => {
                                        model.push_error(ErrorEntry {
                                            message: format!("Invalid launch config: {e}"),
                                            severity: ErrorSeverity::Critical,
                                        });
                                    }
                                }
                            }
                            crate::screens::wizard::WizardAction::Cancel => {
                                model.wizard = None;
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Esc => {
                        let action = wiz.cancel();
                        if action == crate::screens::wizard::WizardAction::Cancel {
                            model.wizard = None;
                        }
                    }
                    KeyCode::Backspace => wiz.input_backspace(),
                    KeyCode::Char(ch) => wiz.input_char(ch),
                    _ => {}
                }
            }
        }
        Message::KeyInput(key) => {
            if let Some(selector) = model.branch_session_selector.as_mut() {
                match key.code {
                    crossterm::event::KeyCode::Up => selector.select_prev(),
                    crossterm::event::KeyCode::Down => selector.select_next(),
                    crossterm::event::KeyCode::Esc => {
                        model.branch_session_selector = None;
                        model.overlay_mode = OverlayMode::None;
                    }
                    crossterm::event::KeyCode::Enter => {
                        let Some(choice) = selector.selected_choice().cloned() else {
                            model.branch_session_selector = None;
                            model.overlay_mode = OverlayMode::None;
                            sync_active_terminal_history(model);
                            return;
                        };
                        let branch_name = selector.branch_name.clone();
                        let worktree_path = selector.worktree_path.clone();
                        model.branch_session_selector = None;
                        model.overlay_mode = OverlayMode::None;
                        match choice {
                            crate::screens::branch_session_selector::BranchSessionSelectorChoice::ExistingSession(index) => {
                                model.switch_session(index);
                                model.active_layer = ActiveLayer::Main;
                            }
                            crate::screens::branch_session_selector::BranchSessionSelectorChoice::AddSession => {
                                open_branch_wizard(
                                    model,
                                    &branch_name,
                                    worktree_path.as_deref(),
                                    true,
                                );
                            }
                            crate::screens::branch_session_selector::BranchSessionSelectorChoice::FullWizard => {
                                open_branch_wizard(
                                    model,
                                    &branch_name,
                                    worktree_path.as_deref(),
                                    false,
                                );
                            }
                        }
                    }
                    _ => {}
                }
                sync_active_terminal_history(model);
                return;
            }
            // Initialization layer: handle clone wizard keys or Esc to quit
            if model.active_layer == ActiveLayer::Initialization {
                if key.code == crossterm::event::KeyCode::Esc {
                    model.should_quit = true;
                    return;
                }
                if let Some(ref mut clone_wiz) = model.clone_wizard {
                    match key.code {
                        crossterm::event::KeyCode::Enter => {
                            clone_wiz.next();
                        }
                        crossterm::event::KeyCode::Backspace => {
                            if clone_wiz.step == screens::clone_wizard::CloneStep::Failed {
                                clone_wiz.prev();
                            } else {
                                clone_wiz.handle_backspace();
                            }
                        }
                        crossterm::event::KeyCode::Char(c) => {
                            clone_wiz.handle_char(c);
                        }
                        _ => {}
                    }
                }
                return;
            }

            // Error overlay: Enter/Esc dismisses the error
            if (!model.error_queue.is_empty() || !model.error_queue_v2.is_empty())
                && (key.code == crossterm::event::KeyCode::Enter
                    || key.code == crossterm::event::KeyCode::Esc)
            {
                model.dismiss_error();
                model.error_queue_v2.dismiss_current();
                sync_active_terminal_history(model);
                return;
            }

            if handle_terminal_view_key(model, key) {
                sync_active_terminal_history(model);
                return;
            }

            // Management layer: Tab key cycles management tabs
            // BUT only when the active screen is NOT in form/edit mode
            if model.active_layer == ActiveLayer::Management
                && key.code == crossterm::event::KeyCode::Tab
            {
                let screen_wants_tab = match model.management_tab {
                    ManagementTab::Profiles | ManagementTab::Settings => {
                        model.settings_state.is_form_mode()
                    }
                    _ => false,
                };
                if !screen_wants_tab {
                    let next_tab = match model.management_tab {
                        ManagementTab::Branches => ManagementTab::Specs,
                        ManagementTab::Specs => ManagementTab::Issues,
                        ManagementTab::Issues => ManagementTab::Profiles,
                        ManagementTab::Profiles => ManagementTab::Branches,
                        ManagementTab::Versions | ManagementTab::Settings | ManagementTab::Logs => {
                            ManagementTab::Branches
                        }
                    };
                    activate_management_tab(model, next_tab);
                    sync_active_terminal_history(model);
                    return;
                }
                // Fall through to screen handler when in form mode
            }
            // Forward to active screen handler
            match model.active_layer {
                ActiveLayer::Initialization => {} // Handled above
                ActiveLayer::Main => {
                    let bytes = key_event_to_bytes(&key);
                    if !bytes.is_empty() {
                        if let Some(pane_id) = active_pane_id(model).map(str::to_string) {
                            jump_terminal_to_live(model, &pane_id);
                        }
                    }
                    write_bytes_to_active_pane(model, &bytes);
                }
                ActiveLayer::Management => {
                    let sub_msg = match model.management_tab {
                        ManagementTab::Branches => {
                            crate::screens::branches::handle_key(&model.branches_state, &key)
                                .map(Message::BranchesMsg)
                        }
                        ManagementTab::Issues => {
                            let msg = crate::screens::issues::handle_key(&model.issues_state, &key);
                            // Intercept OpenDetail to load content
                            if let Some(crate::screens::issues::IssuesMessage::OpenDetail) = &msg {
                                if let Some(issue) = model.issues_state.selected_issue().cloned() {
                                    model.issues_state.detail_content =
                                        load_issue_detail_markdown(&model.repo_root, issue.number);
                                    tracing::info!(
                                        message = "flow_success",
                                        category = "ui",
                                        event = "open_github_issue_detail",
                                        result = "success",
                                        workspace = "default",
                                        issue_number = issue.number,
                                    );
                                }
                            }
                            msg.map(Message::IssuesMsg)
                        }
                        ManagementTab::Specs => {
                            if let Some(m) =
                                crate::screens::specs::handle_key(&model.specs_state, &key)
                            {
                                match m {
                                    crate::screens::specs::SpecsMessage::OpenDetail => {
                                        let visible = model.specs_state.visible_specs();
                                        if let Some(spec) = visible.get(model.specs_state.selected)
                                        {
                                            let detail_sections =
                                                crate::screens::specs::load_spec_detail(
                                                    &model.repo_root,
                                                    &spec.dir_name,
                                                );
                                            model.specs_state.set_detail_sections(detail_sections);
                                        }
                                        crate::screens::specs::update(
                                            &mut model.specs_state,
                                            crate::screens::specs::SpecsMessage::OpenDetail,
                                        );
                                    }
                                    crate::screens::specs::SpecsMessage::LaunchAgent => {
                                        let selected_spec = model
                                            .specs_state
                                            .visible_specs()
                                            .get(model.specs_state.selected)
                                            .map(|spec| (*spec).clone());
                                        if let Some(spec) = selected_spec {
                                            if spec.branches.is_empty() {
                                                let branch_name = spec_default_branch_name(&spec.id);
                                                open_spec_launch(
                                                    model,
                                                    &spec.id,
                                                    &branch_name,
                                                    true,
                                                    None,
                                                );
                                            } else {
                                                model.specs_state.confirm_spec_index =
                                                    model.specs_state.selected;
                                                model.specs_state.branch_candidates =
                                                    spec.branches.clone();
                                                model.specs_state.branch_selected = 0;
                                                model.specs_state.branch_select_mode = true;
                                            }
                                        }
                                    }
                                    crate::screens::specs::SpecsMessage::SelectBranch => {
                                        let selected_spec = model
                                            .specs_state
                                            .visible_specs()
                                            .get(model.specs_state.confirm_spec_index)
                                            .map(|spec| (*spec).clone());
                                        if let Some(spec) = selected_spec {
                                            let branch_idx = model.specs_state.branch_selected;
                                            let branch_name = if branch_idx
                                                < model.specs_state.branch_candidates.len()
                                            {
                                                model.specs_state.branch_candidates[branch_idx]
                                                    .clone()
                                            } else {
                                                spec_default_branch_name(&spec.id)
                                            };
                                            let is_new =
                                                branch_idx >= model.specs_state.branch_candidates.len();
                                            model.specs_state.branch_select_mode = false;
                                            open_spec_launch(
                                                model,
                                                &spec.id,
                                                &branch_name,
                                                is_new,
                                                None,
                                            );
                                        }
                                    }
                                    crate::screens::specs::SpecsMessage::NewSpec => {
                                        model.wizard = Some(
                                            crate::screens::wizard::WizardState::open_for_spec_drafting(),
                                        );
                                    }
                                    other => {
                                        crate::screens::specs::update(&mut model.specs_state, other);
                                    }
                                }
                                Some(Message::Tick)
                            } else {
                                None
                            }
                        }
                        ManagementTab::Settings => {
                            crate::screens::settings::handle_key(&model.settings_state, &key)
                                .map(Message::SettingsMsg)
                        }
                        ManagementTab::Profiles => {
                            crate::screens::settings::handle_key(&model.settings_state, &key)
                                .map(Message::SettingsMsg)
                        }
                        ManagementTab::Logs => {
                            crate::screens::logs::handle_key(&model.logs_state, &key)
                                .map(Message::LogsMsg)
                        }
                        ManagementTab::Versions => {
                            crate::screens::versions::handle_key(&model.versions_state, &key).map(
                                |m| {
                                    // Intercept OpenDetail to load tag detail
                                    if matches!(
                                        m,
                                        crate::screens::versions::VersionsMessage::OpenDetail
                                    ) {
                                        if let Some(tag) = model
                                            .versions_state
                                            .tags
                                            .get(model.versions_state.selected)
                                        {
                                            tracing::info!(
                                                message = "flow_start",
                                                category = "ui",
                                                event = "open_version_detail",
                                                result = "start",
                                                workspace = "default",
                                                tag = tag.label.as_str(),
                                            );
                                            model.versions_state.detail_content =
                                                crate::screens::versions::load_tag_detail(
                                                    &model.repo_root,
                                                    &tag.label,
                                                );
                                            tracing::info!(
                                                message = "flow_success",
                                                category = "ui",
                                                event = "open_version_detail",
                                                result = "success",
                                                workspace = "default",
                                                tag = tag.label.as_str(),
                                            );
                                        }
                                    }
                                    crate::screens::versions::update(&mut model.versions_state, m);
                                    Message::Tick // dummy
                                },
                            )
                        }
                    };
                    // Recursively apply sub-message if any
                    if let Some(sub_msg) = sub_msg {
                        update(model, sub_msg);
                    }
                }
            }
        }
        Message::Paste(text) => {
            if model.active_layer == ActiveLayer::Main {
                if let Some(pane_id) = active_pane_id(model).map(str::to_string) {
                    jump_terminal_to_live(model, &pane_id);
                }
                write_bytes_to_active_pane(model, text.as_bytes());
            }
        }
        Message::MouseInput(mouse) => {
            if handle_terminal_view_mouse(model, mouse) {
                sync_active_terminal_history(model);
                return;
            }
            if model.active_layer == ActiveLayer::Management
                && model.management_tab == ManagementTab::Logs
                && model.overlay_mode == OverlayMode::None
            {
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        handle_logs_msg(model, LogsMessage::SelectPrev);
                    }
                    MouseEventKind::ScrollDown => {
                        handle_logs_msg(model, LogsMessage::SelectNext);
                    }
                    _ => {}
                }
            }
        }
        Message::Resize(w, h) => {
            model.terminal_cols = w;
            model.terminal_rows = h;
        }
        Message::PtyOutput { pane_id, data } => {
            if let Some(pane) = model.pane_manager.pane_mut_by_id(&pane_id) {
                if let Err(error) = pane.process_bytes(&data) {
                    tracing::warn!(
                        message = "flow_failure",
                        category = "ui",
                        event = "persist_pty_scrollback",
                        result = "failure",
                        workspace = "default",
                        pane_id = pane_id.as_str(),
                        error_code = "SCROLLBACK_WRITE_FAILED",
                        error_detail = %error,
                    );
                }
            }

            // Feed data to VT100 parser
            if let Some(parser) = model.vt_parsers.get_mut(&pane_id) {
                parser.process(&data);
            }

            if model.active_history_pane_id.as_deref() == Some(pane_id.as_str()) {
                let follow_live = model
                    .terminal_viewport(&pane_id)
                    .map(|view| view.follow_live)
                    .unwrap_or(true);
                if !follow_live {
                    let old_max = model
                        .terminal_viewport(&pane_id)
                        .map(|view| view.max_scrollback)
                        .unwrap_or(0);
                    let old_scroll = model
                        .terminal_viewport(&pane_id)
                        .map(|view| view.scrollback)
                        .unwrap_or(0);
                    let updated_state =
                        if let Some(copy_parser) = model.active_history_parser.as_mut() {
                            let area = content_area_rect(model.terminal_cols, model.terminal_rows);
                            let old_top = old_max.saturating_sub(old_scroll);

                            copy_parser.process(&data);

                            let new_max =
                                history_parser_max_scrollback(copy_parser, area.height.max(1));
                            let new_scroll = new_max.saturating_sub(old_top).min(new_max);
                            Some((new_scroll, new_max))
                        } else {
                            model.vt_parsers.get(&pane_id).map(|parser| {
                                (parser.screen().scrollback(), parser.screen().scrollback())
                            })
                        };
                    if let Some((scrollback, max_scrollback)) = updated_state {
                        let view = model.terminal_viewport_mut(&pane_id);
                        view.scrollback = scrollback;
                        view.max_scrollback = max_scrollback;
                    }
                }
            }
        }
        Message::Tick => {
            model.apply_background_updates();
        }
        Message::PushError(entry) => {
            model.push_error(entry);
        }
        Message::DismissError => {
            model.dismiss_error();
        }
        // -- Overlay / dialog messages ------------------------------------------
        Message::OpenCloneWizard => {
            model.clone_wizard = Some(screens::clone_wizard::CloneWizardState::new());
            model.overlay_mode = OverlayMode::CloneWizard;
        }
        Message::CloseCloneWizard => {
            model.clone_wizard = None;
            model.overlay_mode = OverlayMode::None;
        }
        Message::OpenSpecKitWizard => {
            model.speckit_wizard.open();
            model.overlay_mode = OverlayMode::SpecKitWizard;
        }
        Message::CloseSpecKitWizard => {
            model.speckit_wizard.close();
            model.overlay_mode = OverlayMode::None;
        }
        Message::ConfirmAccepted => {
            let action = model.confirm.as_ref().map(|c| c.on_confirm.clone());
            model.confirm = None;
            model.overlay_mode = OverlayMode::None;
            match action {
                Some(crate::screens::confirm::ConfirmAction::QuitWithAgents) => {
                    model.should_quit = true;
                }
                Some(crate::screens::confirm::ConfirmAction::EmbedCodexHooks) => {
                    // SPEC-1786: Embed accepted — run full skill registration then launch
                    if let Some(config) = model.pending_codex_launch.take() {
                        if let Err(e) = spawn_agent_session(model, &config, false) {
                            model.push_error(ErrorEntry {
                                message: format!("Failed to launch agent: {e}"),
                                severity: ErrorSeverity::Critical,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        Message::ConfirmCancelled => {
            let action = model.confirm.as_ref().map(|c| c.on_confirm.clone());
            model.confirm = None;
            model.overlay_mode = OverlayMode::None;
            // SPEC-1786: Skip hooks — launch agent without skill registration
            if let Some(crate::screens::confirm::ConfirmAction::EmbedCodexHooks) = action {
                if let Some(config) = model.pending_codex_launch.take() {
                    if let Err(e) = spawn_agent_session(model, &config, true) {
                        model.push_error(ErrorEntry {
                            message: format!("Failed to launch agent: {e}"),
                            severity: ErrorSeverity::Critical,
                        });
                    }
                }
            }
        }
        Message::ProgressAdvance => {
            if let Some(ref mut progress) = model.progress {
                progress.advance();
            }
        }
        Message::ProgressError(msg) => {
            if let Some(ref mut progress) = model.progress {
                progress.set_error(msg);
            }
        }

        // Screen-specific messages
        Message::BranchesMsg(msg) => {
            use crate::screens::branches::BranchesMessage;
            // Intercept Enter to open Wizard with selected branch
            if matches!(msg, BranchesMessage::Enter) {
                if let Some(branch) = model.branches_state.selected_branch().cloned() {
                    let session_indices = matching_session_indices(model, &branch.name);
                    match session_indices.len() {
                        0 => open_branch_wizard(
                            model,
                            &branch.name,
                            branch.worktree_path.as_deref(),
                            true,
                        ),
                        1 => {
                            model.switch_session(session_indices[0]);
                            model.active_layer = ActiveLayer::Main;
                        }
                        _ => open_branch_session_selector(
                            model,
                            &branch.name,
                            branch.worktree_path.clone(),
                            session_indices,
                        ),
                    }
                }
                sync_active_terminal_history(model);
                return;
            }
            if matches!(msg, BranchesMessage::Refresh) {
                model.branches_state.loading = true;
                model.branches_state.branches = crate::screens::branches::load_branches(&model.repo_root);
                model.sync_branch_session_counts();
                model.branch_list_rx = Some(spawn_branch_list_enrichment(model.repo_root.clone()));
                sync_active_terminal_history(model);
                return;
            }
            crate::screens::branches::update(&mut model.branches_state, msg);
        }
        Message::IssuesMsg(msg) => match msg {
            crate::screens::issues::IssuesMessage::Refresh => {
                crate::screens::issues::update(
                    &mut model.issues_state,
                    crate::screens::issues::IssuesMessage::Refresh,
                );
                match crate::screens::issues::refresh_issues(&model.repo_root) {
                    Ok(issues) => crate::screens::issues::update(
                        &mut model.issues_state,
                        crate::screens::issues::IssuesMessage::Loaded(issues),
                    ),
                    Err(error) => {
                        model.push_error(ErrorEntry {
                            message: format!("Failed to refresh issues: {error}"),
                            severity: ErrorSeverity::Minor,
                        });
                        crate::screens::issues::update(
                            &mut model.issues_state,
                            crate::screens::issues::IssuesMessage::Loaded(
                                crate::screens::issues::load_issues(&model.repo_root),
                            ),
                        );
                    }
                }
            }
            crate::screens::issues::IssuesMessage::LaunchAgent => {
                if let Some(issue) = model.issues_state.selected_issue().cloned() {
                    open_issue_launch(model, issue.number);
                }
            }
            other => {
                crate::screens::issues::update(&mut model.issues_state, other);
            }
        },
        Message::VersionsMsg(_) => {
            // Versions messages are handled inline via the key handler
        }
        Message::SettingsMsg(msg) => {
            handle_settings_msg(model, msg);
        }
        Message::LogsMsg(msg) => {
            handle_logs_msg(model, msg);
        }
    }

    sync_active_terminal_history(model);
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Render the model to the terminal frame (Elm Architecture view function).
pub fn view(model: &Model, frame: &mut Frame) {
    let area = frame.area();
    let layout = Layout::vertical([
        Constraint::Length(1), // Tab bar
        Constraint::Length(1), // Separator line
        Constraint::Min(1),    // Main area
        Constraint::Length(1), // Status bar
    ])
    .split(area);

    let mut cursor_pos: Option<(u16, u16)> = None;

    {
        let buf = frame.buffer_mut();

        // Tab bar
        widgets::tab_bar::render(model, buf, layout[0]);

        // Separator line between tab bar and content
        for x in layout[1].x..layout[1].right() {
            if let Some(cell) = buf.cell_mut((x, layout[1].y)) {
                cell.set_char('\u{2500}'); // horizontal line ─
                cell.set_style(Style::default().fg(Color::DarkGray));
            }
        }

        // Main content area
        match model.active_layer {
            ActiveLayer::Initialization => {
                // Fullscreen clone wizard for initialization
                if let Some(ref clone_wiz) = model.clone_wizard {
                    screens::clone_wizard::render_fullscreen(clone_wiz, buf, area);
                }
            }
            ActiveLayer::Main => {
                if model.session_tabs.is_empty() {
                    let center = centered_text(
                        "No sessions. Press Enter on Branches for agent or Ctrl+G, c for shell.",
                    );
                    let text_area = centered_rect(60, 3, layout[2]);
                    ratatui::widgets::Widget::render(center, text_area, buf);
                } else {
                    match model.session_layout_mode {
                        SessionLayoutMode::Grid => {
                            render_session_grid(model, buf, layout[2]);
                        }
                        SessionLayoutMode::Maximized => {
                            let pane_id = &model.session_tabs[model.active_session].pane_id;
                            let parser = active_terminal_parser(model, pane_id);
                            let view = model.terminal_viewport(pane_id);
                            let selection = view
                                .and_then(|view| view.selection_anchor.zip(view.selection_focus));
                            cursor_pos = if model.active_history_pane_id.as_deref()
                                == Some(pane_id.as_str())
                                && model.active_history_parser.is_some()
                            {
                                parser.and_then(|parser| {
                                    crate::screens::agent_pane::render_history(
                                        buf,
                                        layout[2],
                                        parser,
                                        view.map(terminal_view_origin).unwrap_or_default(),
                                        selection,
                                    )
                                })
                            } else {
                                crate::screens::agent_pane::render(
                                    buf, layout[2], parser, selection,
                                )
                            };
                        }
                    }
                }
            }
            ActiveLayer::Management => match model.management_tab {
                ManagementTab::Branches => {
                    crate::screens::branches::render(&model.branches_state, buf, layout[2]);
                }
                ManagementTab::Issues => {
                    crate::screens::issues::render(&model.issues_state, buf, layout[2]);
                }
                ManagementTab::Specs => {
                    crate::screens::specs::render(&model.specs_state, buf, layout[2]);
                }
                ManagementTab::Settings => {
                    crate::screens::settings::render(&model.settings_state, buf, layout[2]);
                }
                ManagementTab::Profiles => {
                    crate::screens::settings::render_profiles_tab(
                        &model.settings_state,
                        buf,
                        layout[2],
                    );
                }
                ManagementTab::Logs => {
                    crate::screens::logs::render(&model.logs_state, buf, layout[2]);
                }
                ManagementTab::Versions => {
                    crate::screens::versions::render(&model.versions_state, buf, layout[2]);
                }
            },
        }

        // Status bar
        widgets::status_bar::render(model, buf, layout[3]);

        // Overlays (on top of everything, priority order)
        // Wizard overlay
        if let Some(ref wizard) = model.wizard {
            crate::screens::wizard::render(buf, area, wizard);
        }

        if let Some(ref selector) = model.branch_session_selector {
            crate::screens::branch_session_selector::render(selector, buf, area);
        }

        // Error overlay (v2 queue)
        if !model.error_queue_v2.is_empty() {
            screens::error::render_error_with_queue(&model.error_queue_v2, buf, area);
        } else if !model.error_queue.is_empty() {
            // Legacy error overlay
            render_error_overlay(buf, area, &model.error_queue[0]);
        }

        // Confirm dialog
        if let Some(ref confirm) = model.confirm {
            screens::confirm::render_confirm(confirm, buf, area);
        }

        // Progress modal
        if let Some(ref progress) = model.progress {
            widgets::progress_modal::render(buf, area, progress);
        }

        // Clone wizard
        if let Some(ref clone_wiz) = model.clone_wizard {
            screens::clone_wizard::render_clone_wizard(clone_wiz, buf, area);
        }

        // SpecKit wizard
        screens::speckit_wizard::render_speckit_wizard(&model.speckit_wizard, buf, area);
    } // end buf borrow scope

    // Set cursor position (outside buf borrow)
    if let Some((cx, cy)) = cursor_pos {
        frame.set_cursor_position((cx, cy));
    }
}

/// Render a simple error overlay.
fn render_error_overlay(buf: &mut Buffer, area: Rect, entry: &ErrorEntry) {
    use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

    let modal_width = 60.min(area.width.saturating_sub(4));
    let modal_height = 5.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
    let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect::new(x, y, modal_width, modal_height);

    Clear.render(modal_area, buf);

    let border_color = match entry.severity {
        ErrorSeverity::Critical => Color::Red,
        ErrorSeverity::Minor => Color::Yellow,
    };

    let para = Paragraph::new(entry.message.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(" Error (Enter to dismiss) "),
        )
        .wrap(Wrap { trim: true });

    ratatui::widgets::Widget::render(para, modal_area, buf);
}

/// Helper: create a centered Paragraph.
fn centered_text(text: &str) -> ratatui::widgets::Paragraph<'_> {
    ratatui::widgets::Paragraph::new(text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray))
}

/// Helper: create a centered rect within `area`.
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width * percent_x / 100).max(1);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

// ---------------------------------------------------------------------------
// Settings message handler
// ---------------------------------------------------------------------------

fn handle_settings_msg(model: &mut Model, msg: SettingsMessage) {
    let state = &mut model.settings_state;
    match msg {
        SettingsMessage::Refresh => {
            state.load_settings();
        }
        SettingsMessage::NextCategory => state.next_category(),
        SettingsMessage::PrevCategory => state.prev_category(),
        SettingsMessage::SelectNext => state.select_next(),
        SettingsMessage::SelectPrev => state.select_prev(),
        SettingsMessage::Edit => {
            if state.category == crate::screens::settings::SettingsCategory::CustomAgents {
                if state.is_add_agent_selected() {
                    state.enter_add_mode();
                } else {
                    state.enter_edit_mode();
                }
            }
        }
        SettingsMessage::Delete => {
            if state.category == crate::screens::settings::SettingsCategory::CustomAgents {
                state.enter_delete_mode();
            }
        }
        SettingsMessage::Save => {
            if matches!(
                state.custom_agent_mode,
                crate::screens::settings::CustomAgentMode::Add
                    | crate::screens::settings::CustomAgentMode::Edit(_)
            ) {
                if let Err(e) = state.save_agent() {
                    state.error_message = Some(e.to_string());
                }
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                if let Err(e) = state.save_profile() {
                    state.error_message = Some(e.to_string());
                }
            }
        }
        SettingsMessage::Cancel => {
            if state.is_form_mode() || state.is_delete_mode() {
                state.cancel_mode();
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::ConfirmDelete(_)
            ) {
                state.cancel_profile_mode();
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::EnvEdit(_)
            ) {
                // Save env edits before leaving
                let _ = state.persist_env_edit();
                state.cancel_profile_mode();
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                state.cancel_profile_mode();
            }
        }
        SettingsMessage::FormChar(c) => {
            if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                state.profile_form.insert_char(c);
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::EnvEdit(_)
            ) {
                handle_env_edit_char(state, c);
            } else {
                state.agent_form.insert_char(c);
            }
        }
        SettingsMessage::FormBackspace => {
            if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                state.profile_form.delete_char();
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::EnvEdit(_)
            ) {
                handle_env_edit_backspace(state);
            } else {
                state.agent_form.delete_char();
            }
        }
        SettingsMessage::FormNextField => {
            if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                state.profile_form.next_field();
            } else {
                state.agent_form.next_field();
            }
        }
        SettingsMessage::FormPrevField => {
            if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::Add
                    | crate::screens::settings::ProfileMode::Edit(_)
            ) {
                state.profile_form.prev_field();
            } else {
                state.agent_form.prev_field();
            }
        }
        SettingsMessage::FormCycleType => {
            state.agent_form.cycle_type();
        }
        SettingsMessage::ToggleDeleteConfirm => {
            if state.is_delete_mode() {
                state.delete_confirm = !state.delete_confirm;
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::ConfirmDelete(_)
            ) {
                state.profile_delete_confirm = !state.profile_delete_confirm;
            }
        }
        SettingsMessage::ConfirmDelete => {
            if state.is_delete_mode() {
                if state.delete_confirm {
                    state.delete_agent();
                } else {
                    state.cancel_mode();
                }
            } else if matches!(
                state.profile_mode,
                crate::screens::settings::ProfileMode::ConfirmDelete(_)
            ) {
                if state.profile_delete_confirm {
                    state.delete_profile();
                } else {
                    state.cancel_profile_mode();
                }
            }
        }
        SettingsMessage::Activate => {}
        SettingsMessage::ProfileAdd => state.enter_profile_add_mode(),
        SettingsMessage::ProfileEdit => state.enter_profile_edit_mode(),
        SettingsMessage::ProfileDelete => state.enter_profile_delete_mode(),
        SettingsMessage::ProfileToggleActive => state.toggle_active_profile(),
        SettingsMessage::ProfileEnvEdit => state.enter_env_edit_mode(),
        SettingsMessage::EnvNew => state.env_state.add_new_var(),
        SettingsMessage::EnvDelete => state.env_state.delete_selected(),
        SettingsMessage::EnvToggleDisabled => {
            state.env_state.toggle_selected_disabled();
        }
        SettingsMessage::EnvToggleKeyValue => state.env_state.toggle_key_value(),
        SettingsMessage::EnvStartEdit => {
            state.env_state.start_edit_selected();
        }
        SettingsMessage::EnvConfirm => {
            state.env_state.editing = None;
        }
    }
}

fn handle_env_edit_char(state: &mut crate::screens::SettingsState, c: char) {
    let Some(idx) = state.env_state.selected_profile_index() else {
        return;
    };
    if let Some(ref mode) = state.env_state.editing.clone() {
        match mode {
            crate::screens::settings::EnvEditMode::Key(cursor) => {
                let cursor = *cursor;
                state.env_state.vars[idx].0.insert(cursor, c);
                state.env_state.editing =
                    Some(crate::screens::settings::EnvEditMode::Key(cursor + 1));
            }
            crate::screens::settings::EnvEditMode::Value(cursor) => {
                let cursor = *cursor;
                state.env_state.vars[idx].1.insert(cursor, c);
                state.env_state.editing =
                    Some(crate::screens::settings::EnvEditMode::Value(cursor + 1));
            }
        }
    }
}

fn handle_env_edit_backspace(state: &mut crate::screens::SettingsState) {
    let Some(idx) = state.env_state.selected_profile_index() else {
        return;
    };
    if let Some(ref mode) = state.env_state.editing.clone() {
        match mode {
            crate::screens::settings::EnvEditMode::Key(cursor) => {
                if *cursor > 0 {
                    let new_cursor = cursor - 1;
                    state.env_state.vars[idx].0.remove(new_cursor);
                    state.env_state.editing =
                        Some(crate::screens::settings::EnvEditMode::Key(new_cursor));
                }
            }
            crate::screens::settings::EnvEditMode::Value(cursor) => {
                if *cursor > 0 {
                    let new_cursor = cursor - 1;
                    state.env_state.vars[idx].1.remove(new_cursor);
                    state.env_state.editing =
                        Some(crate::screens::settings::EnvEditMode::Value(new_cursor));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Logs message handler
// ---------------------------------------------------------------------------

fn handle_logs_msg(model: &mut Model, msg: LogsMessage) {
    let state = &mut model.logs_state;
    match msg {
        LogsMessage::Refresh => {
            tracing::info!(
                message = "flow_start",
                category = "ui",
                event = "refresh_logs",
                result = "start",
                workspace = "default",
            );
            let entries = crate::screens::logs::load_log_entries(&model.repo_root);
            *state = crate::screens::LogsState::new().with_entries(entries);
            tracing::info!(
                message = "flow_success",
                category = "ui",
                event = "refresh_logs",
                result = "success",
                workspace = "default",
                entry_count = state.entries.len(),
            );
        }
        LogsMessage::SelectPrev => state.select_prev(),
        LogsMessage::SelectNext => state.select_next(),
        LogsMessage::PageUp => state.page_up(10),
        LogsMessage::PageDown => state.page_down(10),
        LogsMessage::GoHome => state.go_home(),
        LogsMessage::GoEnd => state.go_end(),
        LogsMessage::CycleFilter => state.cycle_filter(),
        LogsMessage::ToggleSearch => state.toggle_search(),
        LogsMessage::ToggleDetail => {
            state.toggle_detail();
            if state.show_detail {
                tracing::info!(
                    message = "flow_success",
                    category = "ui",
                    event = "open_log_detail",
                    result = "success",
                    workspace = "default",
                );
            }
        }
        LogsMessage::CloseDetail => state.close_detail(),
        LogsMessage::SearchChar(c) => {
            state.search.push(c);
            state.selected = 0;
            state.offset = 0;
        }
        LogsMessage::SearchBackspace => {
            state.search.pop();
            state.selected = 0;
            state.offset = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// Shell session spawning
// ---------------------------------------------------------------------------

fn spawn_shell_session(model: &mut Model) -> Result<(), Box<dyn std::error::Error>> {
    use gwt_core::agent::launch::ShellLaunchBuilder;
    use gwt_core::terminal::AgentColor;

    let config = ShellLaunchBuilder::new(&model.repo_root).build();
    let rows = model.terminal_rows.saturating_sub(2);
    let cols = model.terminal_cols;

    let pane_id = model
        .pane_manager
        .spawn_shell(&model.repo_root, config, rows, cols)?;

    // Start PTY reader thread
    let pane = model
        .pane_manager
        .panes()
        .iter()
        .find(|p| p.pane_id() == pane_id)
        .ok_or("pane not found")?;
    let mut reader = pane.take_reader()?;
    let tx = model
        .pty_tx
        .as_ref()
        .ok_or("pty_tx not initialized")?
        .clone();
    let id = pane_id.clone();
    std::thread::Builder::new()
        .name(format!("pty-reader-{id}"))
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        use crate::event::PtyOutputMsg;
                        if tx
                            .send(PtyOutputMsg {
                                pane_id: id.clone(),
                                data: buf[..n].to_vec(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })?;

    // Register VT100 parser
    model
        .vt_parsers
        .insert(pane_id.clone(), vt100::Parser::new(rows, cols, 1000));

    // Add session tab
    model.add_session(crate::model::SessionTab {
        pane_id,
        name: "shell".to_string(),
        tab_type: crate::model::SessionTabType::Shell,
        color: AgentColor::White,
        status: crate::model::SessionStatus::Running,
        branch: None,
        spec_id: None,
    });

    // Switch to Main layer
    model.active_layer = ActiveLayer::Main;

    // Save session entry for branch tool history (agent_id = "shell")
    let _ = gwt_core::config::save_session_entry(
        &model.repo_root,
        gwt_core::config::ToolSessionEntry {
            branch: "terminal".to_string(),
            worktree_path: Some(model.repo_root.to_string_lossy().to_string()),
            tool_id: "shell".to_string(),
            tool_label: "Shell".to_string(),
            session_id: None,
            mode: None,
            model: None,
            reasoning_level: None,
            skip_permissions: None,
            tool_version: None,
            collaboration_modes: None,
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            docker_container_name: None,
            docker_compose_args: None,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        },
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Worktree resolution for branch-targeted agent launch
// ---------------------------------------------------------------------------

/// Resolve the worktree path for the given branch.
///
/// Returns the existing worktree path if one is already checked out for
/// *branch_name*, creates a new worktree otherwise, and falls back to
/// *repo_root* when neither succeeds.
fn resolve_branch_working_dir(
    repo_root: &std::path::Path,
    branch_name: &str,
    is_new_branch: bool,
    base_branch: Option<&str>,
) -> std::path::PathBuf {
    use gwt_core::error::GwtError;
    use gwt_core::worktree::WorktreeManager;
    match WorktreeManager::new(repo_root) {
        Ok(wt_manager) => {
            let resolved = if is_new_branch {
                match wt_manager.create_new_branch(branch_name, base_branch) {
                    Ok(wt) => Ok(wt),
                    Err(GwtError::BranchAlreadyExists { .. }) => {
                        wt_manager.create_for_branch(branch_name)
                    }
                    Err(error) => Err(error),
                }
            } else {
                match wt_manager.get_by_branch(branch_name) {
                    Ok(Some(wt)) => return wt.path,
                    Ok(None) => wt_manager.create_for_branch(branch_name),
                    Err(_) => return repo_root.to_path_buf(),
                }
            };
            resolved
                .map(|wt| wt.path)
                .unwrap_or_else(|_| repo_root.to_path_buf())
        }
        Err(_) => repo_root.to_path_buf(),
    }
}

fn resolve_wizard_working_dir(
    repo_root: &std::path::Path,
    wiz_config: &crate::screens::wizard::WizardLaunchConfig,
) -> std::path::PathBuf {
    if wiz_config.branch_name.is_empty() {
        repo_root.to_path_buf()
    } else {
        resolve_branch_working_dir(
            repo_root,
            &wiz_config.branch_name,
            wiz_config.is_new_branch,
            wiz_config.base_branch.as_deref(),
        )
    }
}

fn build_agent_session_entry(
    worktree_path: &std::path::Path,
    wiz_config: &crate::screens::wizard::WizardLaunchConfig,
    tool_label: String,
    session_id: Option<String>,
) -> gwt_core::config::ToolSessionEntry {
    gwt_core::config::ToolSessionEntry {
        branch: wiz_config.branch_name.clone(),
        worktree_path: Some(worktree_path.to_string_lossy().to_string()),
        tool_id: wiz_config.agent_id.clone(),
        tool_label,
        session_id,
        mode: Some(wiz_config.execution_mode.label().to_string()),
        model: wiz_config.model.clone(),
        reasoning_level: wiz_config
            .reasoning_level
            .as_ref()
            .map(|r| r.label().to_string()),
        skip_permissions: Some(wiz_config.skip_permissions),
        tool_version: wiz_config.version.clone(),
        collaboration_modes: None,
        docker_service: None,
        docker_force_host: None,
        docker_recreate: None,
        docker_build: None,
        docker_keep: None,
        docker_container_name: None,
        docker_compose_args: None,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
    }
}

// ---------------------------------------------------------------------------
// Agent session spawning (from Wizard)
// ---------------------------------------------------------------------------

/// Check if a Codex agent launch needs hooks confirmation and show the dialog.
/// Returns `true` if a confirm dialog was shown (launch should be deferred).
fn check_codex_hooks_confirm(
    model: &mut Model,
    wiz_config: &crate::screens::wizard::WizardLaunchConfig,
) -> bool {
    if wiz_config.agent_id != "codex" {
        return false;
    }

    let working_dir = resolve_wizard_working_dir(&model.repo_root, wiz_config);

    let codex_root = working_dir.join(".codex");
    if gwt_core::config::codex_hooks_needs_update(&codex_root) {
        // Store pending launch config and show confirmation dialog (FR-031)
        model.pending_codex_launch = Some(wiz_config.clone());
        model.confirm = Some(crate::screens::confirm::ConfirmState::embed_codex_hooks());
        model.overlay_mode = OverlayMode::Confirm;
        true
    } else {
        false
    }
}

fn spawn_agent_session(
    model: &mut Model,
    wiz_config: &crate::screens::wizard::WizardLaunchConfig,
    skip_hooks_registration: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use gwt_core::agent::launch::AgentLaunchBuilder;
    use gwt_core::config::skill_registration::{
        register_agent_skills_with_settings_at_project_root, SkillAgentType,
    };

    let agent_id = &wiz_config.agent_id;
    let working_dir = resolve_wizard_working_dir(&model.repo_root, wiz_config);

    // Register managed skills/hooks for this agent (SPEC-1438 FR-REG-001)
    if !skip_hooks_registration {
        if let Some(agent_type) = SkillAgentType::from_agent_id(agent_id) {
            match gwt_core::config::Settings::load(&working_dir) {
                Ok(settings) => {
                    if let Err(e) = register_agent_skills_with_settings_at_project_root(
                        agent_type,
                        &settings,
                        Some(&working_dir),
                    ) {
                        tracing::warn!(
                            agent = agent_id,
                            error = %e,
                            "Skill registration failed; continuing with agent launch"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "Failed to load settings for skill registration; continuing with agent launch"
                    );
                }
            }
        }
    }

    // Build launch config via gwt-core
    let mut builder = AgentLaunchBuilder::new(agent_id, &working_dir);
    if !wiz_config.branch_name.is_empty() {
        builder = builder.branch_name(&wiz_config.branch_name);
    }
    if let Some(ref m) = wiz_config.model {
        builder = builder.model(Some(m.as_str()));
    }
    if let Some(ref v) = wiz_config.version {
        builder = builder.agent_version(Some(v.as_str()));
    }
    builder = builder.skip_permissions(wiz_config.skip_permissions);

    // Apply execution mode
    let session_mode = match wiz_config.execution_mode {
        crate::screens::wizard::WizardExecutionMode::Normal
        | crate::screens::wizard::WizardExecutionMode::Convert => {
            gwt_core::agent::launch::SessionMode::Normal
        }
        crate::screens::wizard::WizardExecutionMode::Resume => {
            gwt_core::agent::launch::SessionMode::Resume
        }
    };
    builder = builder.session_mode(session_mode);
    if let Some(ref id) = wiz_config.session_id {
        builder = builder.resume_session_id(id.clone());
    }

    // Apply fast mode (Codex)
    if wiz_config.fast_mode {
        builder = builder.fast_mode(true);
    }

    // Apply reasoning level (Codex)
    if let Some(ref level) = wiz_config.reasoning_level {
        builder = builder.reasoning_level(Some(level.label()));
    }

    let config = builder.build()?;

    let rows = model.terminal_rows.saturating_sub(3);
    let cols = model.terminal_cols;

    // Spawn PTY via PaneManager
    let pane_id = model
        .pane_manager
        .spawn_shell(&model.repo_root, config, rows, cols)?;

    // Start PTY reader thread
    let pane = model
        .pane_manager
        .panes()
        .iter()
        .find(|p| p.pane_id() == pane_id)
        .ok_or("pane not found")?;
    let mut reader = pane.take_reader()?;
    let tx = model
        .pty_tx
        .as_ref()
        .ok_or("pty_tx not initialized")?
        .clone();
    let id = pane_id.clone();
    std::thread::Builder::new()
        .name(format!("pty-reader-{id}"))
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        use crate::event::PtyOutputMsg;
                        if tx
                            .send(PtyOutputMsg {
                                pane_id: id.clone(),
                                data: buf[..n].to_vec(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })?;

    // Register VT100 parser
    model
        .vt_parsers
        .insert(pane_id.clone(), vt100::Parser::new(rows, cols, 1000));

    // Determine display name and color
    let color = gwt_core::agent::launch::agent_color_for(agent_id);
    let display_name = format!("{}: {}", agent_id, wiz_config.branch_name);

    // Add session tab
    model.add_session(crate::model::SessionTab {
        pane_id,
        name: display_name,
        tab_type: crate::model::SessionTabType::Agent,
        color,
        status: crate::model::SessionStatus::Running,
        branch: if wiz_config.branch_name.is_empty() {
            None
        } else {
            Some(wiz_config.branch_name.clone())
        },
        spec_id: None,
    });

    // Switch to Main layer
    model.active_layer = ActiveLayer::Main;

    // Save session entry for branch tool history (populates Quick Start)
    let agent_label = gwt_core::agent::launch::find_agent_def(agent_id)
        .map(|d| d.display_name.to_string())
        .unwrap_or_else(|| agent_id.to_string());
    let _ = gwt_core::config::save_session_entry(
        &model.repo_root,
        build_agent_session_entry(
            &working_dir,
            wiz_config,
            agent_label,
            wiz_config.session_id.clone(),
        ),
    );

    // Background session_id detection (SPEC-1782 FR-050, NFR-002)
    {
        let repo_root = model.repo_root.clone();
        let working_dir = working_dir.clone();
        let tool_id = wiz_config.agent_id.clone();
        let agent_label_bg = gwt_core::agent::launch::find_agent_def(&tool_id)
            .map(|d| d.display_name.to_string())
            .unwrap_or_else(|| tool_id.clone());
        let wiz_config_bg = wiz_config.clone();

        std::thread::Builder::new()
            .name("session-id-detect".into())
            .spawn(move || {
                // Wait for the agent to initialize and create a session file
                std::thread::sleep(std::time::Duration::from_secs(5));
                if let Some(session_id) =
                    gwt_core::ai::detect_session_id_for_tool(&tool_id, &working_dir)
                {
                    let _ = gwt_core::config::save_session_entry(
                        &repo_root,
                        build_agent_session_entry(
                            &working_dir,
                            &wiz_config_bg,
                            agent_label_bg,
                            Some(session_id),
                        ),
                    );
                }
            })
            .ok();
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Quick Start history loading
// ---------------------------------------------------------------------------

/// Load branch tool history from gwt-core and convert to QuickStartEntry.
/// Load Quick Start history: find the latest tool with a session_id (SPEC-1782 FR-001, FR-002).
/// Returns at most 1 entry. Returns empty if no session_id exists.
fn load_quick_start_history(
    repo_root: &std::path::Path,
    branch: &str,
    expected_worktree_path: Option<&std::path::Path>,
) -> Vec<crate::screens::wizard::QuickStartEntry> {
    let history = gwt_core::config::get_branch_tool_history_for_worktree(
        repo_root,
        branch,
        expected_worktree_path,
    );
    // Find the first entry (newest) that has a session_id
    let entry = history.into_iter().find(|e| e.session_id.is_some());
    match entry {
        Some(e) => vec![crate::screens::wizard::QuickStartEntry {
            tool_id: e.tool_id,
            tool_label: e.tool_label,
            model: e.model,
            version: e.tool_version,
            session_id: e.session_id,
            skip_permissions: e.skip_permissions,
            reasoning_level: e.reasoning_level,
            fast_mode: None, // not stored in ToolSessionEntry yet
            collaboration_modes: e.collaboration_modes,
            branch: e.branch,
        }],
        None => vec![],
    }
}

// ---------------------------------------------------------------------------
// Key → bytes conversion (for PTY input)
// ---------------------------------------------------------------------------

fn key_event_to_bytes(key: &crossterm::event::KeyEvent) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};

    // Alt modifier: send ESC prefix + the key bytes
    if key.modifiers.contains(KeyModifiers::ALT) {
        let inner_key =
            crossterm::event::KeyEvent::new(key.code, key.modifiers - KeyModifiers::ALT);
        let inner = key_event_to_bytes(&inner_key);
        if !inner.is_empty() {
            let mut out = vec![0x1b]; // ESC prefix for Alt
            out.extend_from_slice(&inner);
            return out;
        }
    }

    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                let ctrl_byte = (c as u8).wrapping_sub(b'a').wrapping_add(1);
                if ctrl_byte <= 26 {
                    return vec![ctrl_byte];
                }
            }
            let mut buf = [0u8; 4];
            c.encode_utf8(&mut buf).as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => vec![],
        },
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Key → Message conversion
// ---------------------------------------------------------------------------

/// Convert a KeyAction to an optional Message.
fn action_to_message(action: KeyAction, key: crossterm::event::KeyEvent) -> Option<Message> {
    match action {
        KeyAction::None => None,
        KeyAction::Forward(k) => Some(Message::KeyInput(k)),
        KeyAction::ToggleLayer => Some(Message::ToggleLayer),
        KeyAction::NextSession => Some(Message::NextSession),
        KeyAction::PrevSession => Some(Message::PrevSession),
        KeyAction::SwitchSession(n) => Some(Message::SwitchSession(n)),
        KeyAction::ToggleSessionLayout => Some(Message::ToggleSessionLayout),
        KeyAction::CloseSession => Some(Message::CloseSession),
        KeyAction::NewShell => Some(Message::NewShell),
        KeyAction::TogglePtyCopyMode => Some(Message::TogglePtyCopyMode),
        KeyAction::ShowHelp => {
            // Phase 2: open help screen
            let _ = key;
            None
        }
        KeyAction::Quit => Some(Message::Quit),
    }
}

// ---------------------------------------------------------------------------
// Run (event loop)
// ---------------------------------------------------------------------------

/// Run the TUI application.
pub fn run(repo_root: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initialize model
    let mut model = Model::new(repo_root.clone());

    // Sync terminal size from actual terminal
    if let Ok((cols, rows)) = crossterm::terminal::size() {
        model.terminal_cols = cols;
        model.terminal_rows = rows;
    }

    // Load initial data for management screens (skip if in Initialization mode)
    if model.active_layer != ActiveLayer::Initialization {
        tracing::info!(
            message = "flow_start",
            category = "ui",
            event = "load_management_data",
            result = "start",
            workspace = "default",
        );
        model.load_all_data();
        tracing::info!(
            message = "flow_success",
            category = "ui",
            event = "load_management_data",
            result = "success",
            workspace = "default",
            branches = model.branches_state.branches.len(),
            background_preload = true,
        );
        // Install develop branch protection hook if not already installed
        if !gwt_core::git::hooks::is_develop_guard_installed(&repo_root) {
            if let Err(e) = gwt_core::git::hooks::install_pre_commit_hook(&repo_root) {
                tracing::warn!(
                    error = %e,
                    "Failed to install develop branch protection hook"
                );
            }
        }
    } else {
        // In Initialization mode, auto-open the clone wizard
        model.clone_wizard = Some(screens::clone_wizard::CloneWizardState::new());
    }

    // PTY output channel
    let (pty_tx, pty_rx) = event::pty_output_channel();
    model.pty_tx = Some(pty_tx);

    // Event loop
    let event_loop = EventLoop::new(pty_rx);
    let mut prefix_state = PrefixState::default();
    let mut last_tick = Instant::now();
    let mut mouse_capture_enabled = false;

    if wants_mouse_capture(&model) {
        execute!(terminal.backend_mut(), EnableMouseCapture)?;
        mouse_capture_enabled = true;
    }

    loop {
        // View
        terminal.draw(|f| view(&model, f))?;

        // Event → Message
        let evt = event_loop.next()?;
        let msg = match evt {
            TuiEvent::Key(key) => {
                // Only handle key Press events (ignore Release/Repeat/IME)
                if key.kind != crossterm::event::KeyEventKind::Press {
                    None
                }
                // When confirm dialog is open, intercept all keys
                else if model.confirm.is_some() {
                    match key.code {
                        crossterm::event::KeyCode::Enter => {
                            if model.confirm.as_ref().is_some_and(|c| c.selected_confirm) {
                                Some(Message::ConfirmAccepted)
                            } else {
                                Some(Message::ConfirmCancelled)
                            }
                        }
                        crossterm::event::KeyCode::Esc => Some(Message::ConfirmCancelled),
                        crossterm::event::KeyCode::Left | crossterm::event::KeyCode::Right => {
                            if let Some(ref mut c) = model.confirm {
                                c.toggle_selection();
                            }
                            None
                        }
                        _ => None,
                    }
                }
                // When wizard is open, intercept all keys
                else if model.wizard.is_some() {
                    Some(Message::WizardKey(key))
                } else if keybind::is_ctrl_c(&key) {
                    if model.handle_ctrl_c() {
                        Some(Message::Quit)
                    } else {
                        // Single Ctrl+C: forward to active pane in Main layer
                        if model.active_layer == ActiveLayer::Main {
                            Some(Message::KeyInput(key))
                        } else {
                            None
                        }
                    }
                } else {
                    let action = keybind::process_key(&mut prefix_state, key);
                    action_to_message(action, key)
                }
            }
            TuiEvent::Paste(text) => Some(Message::Paste(text)),
            TuiEvent::Mouse(mouse) => Some(Message::MouseInput(mouse)),
            TuiEvent::Resize(w, h) => Some(Message::Resize(w, h)),
            TuiEvent::PtyOutput { pane_id, data } => Some(Message::PtyOutput { pane_id, data }),
            TuiEvent::Tick => {
                if last_tick.elapsed() >= TICK_INTERVAL {
                    last_tick = Instant::now();
                    Some(Message::Tick)
                } else {
                    None
                }
            }
        };

        // Update
        if let Some(msg) = msg {
            update(&mut model, msg);
            let desired_mouse_capture = wants_mouse_capture(&model);
            if desired_mouse_capture != mouse_capture_enabled {
                if desired_mouse_capture {
                    execute!(terminal.backend_mut(), EnableMouseCapture)?;
                } else {
                    execute!(terminal.backend_mut(), DisableMouseCapture)?;
                }
                mouse_capture_enabled = desired_mouse_capture;
            }
        }

        // Quit check
        if model.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ActiveLayer, ErrorEntry, ErrorSeverity, ManagementTab, OverlayMode, SessionStatus,
        SessionTab, SessionTabType,
    };
    use crate::screens::logs::LogEntry;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    };
    use gwt_core::terminal::pane::{PaneConfig, TerminalPane};
    use gwt_core::terminal::AgentColor;
    use std::collections::{BTreeMap, HashMap};
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::time::Duration;
    use tempfile::TempDir;

    static HOME_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &std::path::Path) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.take() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn test_model() -> Model {
        let mut m = Model::new(PathBuf::from("/tmp/test"));
        m.active_layer = crate::model::ActiveLayer::Management; // Force Management for tests
        m
    }

    fn run_git_in(dir: &Path, args: &[&str]) {
        let output = gwt_core::process::command("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(dir: &Path, args: &[&str]) -> String {
        let output = gwt_core::process::command("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().expect("tempdir");
        run_git_in(temp.path(), &["init"]);
        run_git_in(temp.path(), &["config", "user.email", "test@example.com"]);
        run_git_in(temp.path(), &["config", "user.name", "Test User"]);
        std::fs::write(temp.path().join("README.md"), "hello").expect("seed repo");
        run_git_in(temp.path(), &["add", "README.md"]);
        run_git_in(temp.path(), &["commit", "-m", "init"]);
        temp
    }

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn test_session(name: &str) -> SessionTab {
        SessionTab {
            pane_id: format!("pane-{name}"),
            name: name.to_string(),
            tab_type: SessionTabType::Shell,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: None,
            spec_id: None,
        }
    }

    fn test_branch_session(name: &str, branch: &str) -> SessionTab {
        SessionTab {
            pane_id: format!("pane-{name}"),
            name: name.to_string(),
            tab_type: SessionTabType::Agent,
            color: AgentColor::Blue,
            status: SessionStatus::Running,
            branch: Some(branch.to_string()),
            spec_id: None,
        }
    }

    fn test_log_entry(timestamp: &str, message: &str) -> LogEntry {
        LogEntry {
            timestamp: timestamp.to_string(),
            level: "INFO".to_string(),
            message: message.to_string(),
            target: "gwt".to_string(),
            category: Some("ui".to_string()),
            event: Some("refresh_logs".to_string()),
            result: Some("success".to_string()),
            workspace: Some("feature-1776".to_string()),
            error_code: None,
            error_detail: None,
            extra: BTreeMap::new(),
        }
    }

    fn make_mouse(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 2,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn buffer_text(area: Rect, buffer: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let symbol = buffer[(x, y)].symbol();
                out.push_str(symbol);
            }
            out.push('\n');
        }
        out
    }

    fn seed_scrollback(parser: &mut vt100::Parser, lines: usize) {
        for index in 0..lines {
            parser.process(format!("line-{index}\r\n").as_bytes());
        }
    }

    fn add_cat_session(model: &mut Model, name: &str) -> Box<dyn std::io::Read + Send> {
        let pane_id = format!("pane-{name}");
        let pane = TerminalPane::new(PaneConfig {
            pane_id: pane_id.clone(),
            command: "/bin/cat".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "feature/test".to_string(),
            agent_name: "test-agent".to_string(),
            agent_color: AgentColor::Green,
            rows: 24,
            cols: 80,
            env_vars: HashMap::new(),
            terminal_shell: None,
            interactive: false,
            windows_force_utf8: false,
            project_root: std::env::temp_dir(),
        })
        .expect("pane should be created");

        let reader = pane.take_reader().expect("reader should be available");
        model
            .pane_manager
            .add_pane(pane)
            .expect("pane should be added");
        model.add_session(SessionTab {
            pane_id,
            name: name.to_string(),
            tab_type: SessionTabType::Shell,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: None,
            spec_id: None,
        });
        model.active_layer = ActiveLayer::Main;
        reader
    }

    fn seed_session_transcript(model: &mut Model, name: &str, lines: usize) {
        let _reader = add_cat_session(model, name);
        let pane_id = format!("pane-{name}");
        let area = content_area_rect(model.terminal_cols, model.terminal_rows);
        model.vt_parsers.insert(
            pane_id.clone(),
            vt100::Parser::new(area.height.max(1), area.width.max(1), 2),
        );
        for index in 0..lines {
            update(
                model,
                Message::PtyOutput {
                    pane_id: pane_id.clone(),
                    data: format!("line-{index}\r\n").into_bytes(),
                },
            );
        }
    }

    fn read_from_reader_with_timeout(
        reader: Box<dyn std::io::Read + Send>,
        timeout: Duration,
    ) -> Vec<u8> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            let result = std::io::Read::read(&mut reader, &mut buf)
                .map(|n| buf[..n].to_vec())
                .unwrap_or_default();
            let _ = tx.send(result);
        });
        rx.recv_timeout(timeout).expect("reader timed out")
    }

    #[test]
    fn resolve_branch_working_dir_creates_new_branch_worktree_from_selected_base() {
        let temp = create_test_repo();
        run_git_in(temp.path(), &["branch", "develop"]);

        let working_dir =
            resolve_branch_working_dir(temp.path(), "feature/add-login", true, Some("develop"));

        assert_ne!(working_dir, temp.path());
        assert!(working_dir.exists());
        assert_eq!(
            git_stdout(&working_dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
            "feature/add-login"
        );
        assert_eq!(
            git_stdout(&working_dir, &["rev-parse", "feature/add-login"]),
            git_stdout(&working_dir, &["rev-parse", "develop"])
        );
    }

    #[test]
    fn build_agent_session_entry_preserves_actual_worktree_path() {
        let wiz_config = crate::screens::wizard::WizardLaunchConfig {
            agent_id: "codex".to_string(),
            model: Some("gpt-5".to_string()),
            version: Some("1.2.3".to_string()),
            branch_name: "feature/add-login".to_string(),
            base_branch: Some("develop".to_string()),
            is_new_branch: true,
            execution_mode: crate::screens::wizard::WizardExecutionMode::Normal,
            session_id: None,
            skip_permissions: true,
            fast_mode: true,
            reasoning_level: Some(crate::screens::wizard::ReasoningLevel::High),
        };

        let entry = build_agent_session_entry(
            Path::new("/repo/.worktrees/feature-add-login"),
            &wiz_config,
            "Codex CLI".to_string(),
            Some("sess-123".to_string()),
        );

        assert_eq!(
            entry.worktree_path.as_deref(),
            Some("/repo/.worktrees/feature-add-login")
        );
        assert_eq!(entry.branch, "feature/add-login");
        assert_eq!(entry.session_id.as_deref(), Some("sess-123"));
        assert_eq!(entry.mode.as_deref(), Some("Normal"));
    }

    #[test]
    fn check_codex_hooks_confirm_stores_pending_launch() {
        let repo = tempfile::tempdir().unwrap();
        let mut model = Model::new(repo.path().to_path_buf());
        let wiz_config = crate::screens::wizard::WizardLaunchConfig {
            agent_id: "codex".to_string(),
            model: Some("gpt-5".to_string()),
            version: Some("1.2.3".to_string()),
            branch_name: String::new(),
            base_branch: None,
            is_new_branch: false,
            execution_mode: crate::screens::wizard::WizardExecutionMode::Normal,
            session_id: None,
            skip_permissions: true,
            fast_mode: false,
            reasoning_level: None,
        };

        let shown = check_codex_hooks_confirm(&mut model, &wiz_config);

        assert!(shown);
        assert!(model.pending_codex_launch.is_some());
        assert!(model.confirm.is_some());
        assert_eq!(model.overlay_mode, OverlayMode::Confirm);
    }

    #[test]
    fn check_codex_hooks_confirm_ignores_non_codex_agents() {
        let repo = tempfile::tempdir().unwrap();
        let mut model = Model::new(repo.path().to_path_buf());
        let wiz_config = crate::screens::wizard::WizardLaunchConfig {
            agent_id: "claude".to_string(),
            model: None,
            version: None,
            branch_name: String::new(),
            base_branch: None,
            is_new_branch: false,
            execution_mode: crate::screens::wizard::WizardExecutionMode::Normal,
            session_id: None,
            skip_permissions: false,
            fast_mode: false,
            reasoning_level: None,
        };

        let shown = check_codex_hooks_confirm(&mut model, &wiz_config);

        assert!(!shown);
        assert!(model.pending_codex_launch.is_none());
        assert!(model.confirm.is_none());
    }

    // -- Update tests ---------------------------------------------------------

    #[test]
    fn update_quit_sets_should_quit() {
        let mut m = test_model();
        update(&mut m, Message::Quit);
        assert!(m.should_quit);
    }

    #[test]
    fn update_toggle_layer() {
        let mut m = test_model();
        m.add_session(test_session("s1"));
        update(&mut m, Message::ToggleLayer);
        assert_eq!(m.active_layer, ActiveLayer::Management);
        update(&mut m, Message::ToggleLayer);
        assert_eq!(m.active_layer, ActiveLayer::Main);
    }

    #[test]
    fn update_switch_management_tab() {
        let mut m = test_model();
        update(
            &mut m,
            Message::SwitchManagementTab(ManagementTab::Profiles),
        );
        assert_eq!(m.management_tab, ManagementTab::Profiles);
        assert_eq!(m.active_layer, ActiveLayer::Management);
        assert_eq!(
            m.settings_state.category,
            crate::screens::settings::SettingsCategory::Environment
        );
    }

    #[test]
    fn update_session_navigation() {
        let mut m = test_model();
        m.add_session(test_session("s1"));
        m.add_session(test_session("s2"));
        m.add_session(test_session("s3"));

        update(&mut m, Message::SwitchSession(1)); // 1-based
        assert_eq!(m.active_session, 0);

        update(&mut m, Message::NextSession);
        assert_eq!(m.active_session, 1);

        update(&mut m, Message::PrevSession);
        assert_eq!(m.active_session, 0);
    }

    #[test]
    fn update_toggle_session_layout_switches_between_grid_and_maximized() {
        let mut m = test_model();
        m.add_session(test_session("s1"));
        assert_eq!(m.session_layout_mode, SessionLayoutMode::Grid);

        update(&mut m, Message::ToggleSessionLayout);
        assert_eq!(m.session_layout_mode, SessionLayoutMode::Maximized);

        update(&mut m, Message::ToggleSessionLayout);
        assert_eq!(m.session_layout_mode, SessionLayoutMode::Grid);
    }

    #[test]
    fn maximize_mode_keeps_tab_switch_behavior() {
        let mut m = test_model();
        m.add_session(test_session("s1"));
        m.add_session(test_session("s2"));
        m.switch_session(0);
        update(&mut m, Message::ToggleSessionLayout);
        assert_eq!(m.session_layout_mode, SessionLayoutMode::Maximized);

        update(&mut m, Message::NextSession);
        assert_eq!(m.active_session, 1);
        assert_eq!(m.session_layout_mode, SessionLayoutMode::Maximized);
    }

    #[test]
    fn toggling_management_preserves_session_layout_mode() {
        let mut m = test_model();
        m.add_session(test_session("s1"));
        update(&mut m, Message::ToggleSessionLayout);
        assert_eq!(m.session_layout_mode, SessionLayoutMode::Maximized);

        update(&mut m, Message::ToggleLayer);
        assert_eq!(m.active_layer, ActiveLayer::Management);
        update(&mut m, Message::ToggleLayer);
        assert_eq!(m.active_layer, ActiveLayer::Main);
        assert_eq!(m.session_layout_mode, SessionLayoutMode::Maximized);
    }

    #[test]
    fn equal_grid_layout_returns_one_rect_per_session() {
        let rects = equal_grid_layout(Rect::new(0, 0, 100, 40), 4);
        assert_eq!(rects.len(), 4);
        assert!(rects.iter().all(|rect| rect.width > 0 && rect.height > 0));
    }

    #[test]
    fn update_close_session() {
        let mut m = test_model();
        m.add_session(test_session("s1"));
        assert_eq!(m.session_tabs.len(), 1);
        update(&mut m, Message::CloseSession);
        assert!(m.session_tabs.is_empty());
        assert_eq!(m.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn branch_enter_without_sessions_opens_wizard() {
        let mut m = test_model();
        m.branches_state.branches = vec![crate::screens::branches::BranchItem {
            name: "feature/demo".into(),
            is_current: false,
            has_worktree: true,
            worktree_path: Some("/tmp/feature-demo".into()),
            has_changes: false,
            running_session_count: 0,
            stopped_session_count: 0,
            worktree_indicator: 'w',
            has_unpushed: false,
            is_protected: false,
            last_tool_usage: None,
            last_tool_id: None,
            quick_start_available: false,
            linked_issue_number: None,
            linked_issue_state: None,
            pr_title: None,
            pr_number: None,
            pr_state: None,
            safety_status: crate::screens::branches::SafetyStatus::Safe,
            is_remote: false,
            last_commit_timestamp: None,
            session_count: 0,
        }];

        update(
            &mut m,
            Message::BranchesMsg(crate::screens::branches::BranchesMessage::Enter),
        );

        assert!(m.wizard.is_some(), "expected wizard to open");
        assert!(m.branch_session_selector.is_none());
    }

    #[test]
    fn branch_enter_with_one_session_focuses_existing_session() {
        let mut m = test_model();
        m.branches_state.branches = vec![crate::screens::branches::BranchItem {
            name: "feature/demo".into(),
            is_current: false,
            has_worktree: true,
            worktree_path: Some("/tmp/feature-demo".into()),
            has_changes: false,
            running_session_count: 0,
            stopped_session_count: 0,
            worktree_indicator: 'w',
            has_unpushed: false,
            is_protected: false,
            last_tool_usage: None,
            last_tool_id: None,
            quick_start_available: false,
            linked_issue_number: None,
            linked_issue_state: None,
            pr_title: None,
            pr_number: None,
            pr_state: None,
            safety_status: crate::screens::branches::SafetyStatus::Safe,
            is_remote: false,
            last_commit_timestamp: None,
            session_count: 1,
        }];
        m.add_session(test_branch_session("demo-1", "feature/demo"));
        m.active_layer = ActiveLayer::Management;

        update(
            &mut m,
            Message::BranchesMsg(crate::screens::branches::BranchesMessage::Enter),
        );

        assert_eq!(m.active_layer, ActiveLayer::Main);
        assert_eq!(m.active_session, 0);
        assert!(m.wizard.is_none());
        assert!(m.branch_session_selector.is_none());
    }

    #[test]
    fn branch_enter_with_multiple_sessions_opens_selector() {
        let mut m = test_model();
        m.branches_state.branches = vec![crate::screens::branches::BranchItem {
            name: "feature/demo".into(),
            is_current: false,
            has_worktree: true,
            worktree_path: Some("/tmp/feature-demo".into()),
            has_changes: false,
            running_session_count: 0,
            stopped_session_count: 0,
            worktree_indicator: 'w',
            has_unpushed: false,
            is_protected: false,
            last_tool_usage: None,
            last_tool_id: None,
            quick_start_available: false,
            linked_issue_number: None,
            linked_issue_state: None,
            pr_title: None,
            pr_number: None,
            pr_state: None,
            safety_status: crate::screens::branches::SafetyStatus::Safe,
            is_remote: false,
            last_commit_timestamp: None,
            session_count: 2,
        }];
        m.add_session(test_branch_session("demo-1", "feature/demo"));
        m.add_session(test_branch_session("demo-2", "feature/demo"));
        m.active_layer = ActiveLayer::Management;

        update(
            &mut m,
            Message::BranchesMsg(crate::screens::branches::BranchesMessage::Enter),
        );

        assert!(m.branch_session_selector.is_some(), "expected selector");
        assert!(m.wizard.is_none());
        assert_eq!(m.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn selector_add_session_uses_quick_start_history() {
        let _lock = HOME_LOCK.lock().unwrap();
        let home = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("HOME", home.path());
        let repo = create_test_repo();

        let entry = gwt_core::config::ToolSessionEntry {
            branch: "feature/demo".to_string(),
            worktree_path: Some("/tmp/feature-demo".to_string()),
            tool_id: "codex-cli".to_string(),
            tool_label: "Codex".to_string(),
            session_id: Some("sess-123".to_string()),
            mode: Some("Normal".to_string()),
            model: Some("gpt-5".to_string()),
            reasoning_level: Some("high".to_string()),
            skip_permissions: Some(true),
            tool_version: Some("1.2.3".to_string()),
            collaboration_modes: Some(false),
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            docker_container_name: None,
            docker_compose_args: None,
            timestamp: 1_800_000_000_000,
        };
        gwt_core::config::save_session_entry(repo.path(), entry).unwrap();

        let mut m = Model::new(repo.path().to_path_buf());
        m.active_layer = ActiveLayer::Management;
        m.branch_session_selector = Some(
            crate::screens::branch_session_selector::BranchSessionSelectorState::new(
                "feature/demo",
                Some("/tmp/feature-demo".to_string()),
                vec![crate::screens::branch_session_selector::BranchSessionOption {
                    label: "Add session".to_string(),
                    choice:
                        crate::screens::branch_session_selector::BranchSessionSelectorChoice::AddSession,
                }],
            ),
        );

        update(&mut m, Message::KeyInput(make_key(KeyCode::Enter, KeyModifiers::NONE)));

        let wizard = m.wizard.expect("wizard should open");
        assert!(wizard.has_quick_start);
        assert_eq!(wizard.quick_start_entries.len(), 1);
        assert_eq!(wizard.quick_start_entries[0].session_id.as_deref(), Some("sess-123"));
    }

    #[test]
    fn selector_full_wizard_skips_quick_start_history() {
        let _lock = HOME_LOCK.lock().unwrap();
        let home = TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("HOME", home.path());
        let repo = create_test_repo();

        let entry = gwt_core::config::ToolSessionEntry {
            branch: "feature/demo".to_string(),
            worktree_path: Some("/tmp/feature-demo".to_string()),
            tool_id: "codex-cli".to_string(),
            tool_label: "Codex".to_string(),
            session_id: Some("sess-123".to_string()),
            mode: Some("Normal".to_string()),
            model: Some("gpt-5".to_string()),
            reasoning_level: Some("high".to_string()),
            skip_permissions: Some(true),
            tool_version: Some("1.2.3".to_string()),
            collaboration_modes: Some(false),
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            docker_container_name: None,
            docker_compose_args: None,
            timestamp: 1_800_000_000_000,
        };
        gwt_core::config::save_session_entry(repo.path(), entry).unwrap();

        let mut m = Model::new(repo.path().to_path_buf());
        m.active_layer = ActiveLayer::Management;
        m.branch_session_selector = Some(
            crate::screens::branch_session_selector::BranchSessionSelectorState::new(
                "feature/demo",
                Some("/tmp/feature-demo".to_string()),
                vec![crate::screens::branch_session_selector::BranchSessionOption {
                    label: "Full wizard".to_string(),
                    choice:
                        crate::screens::branch_session_selector::BranchSessionSelectorChoice::FullWizard,
                }],
            ),
        );

        update(&mut m, Message::KeyInput(make_key(KeyCode::Enter, KeyModifiers::NONE)));

        let wizard = m.wizard.expect("wizard should open");
        assert!(!wizard.has_quick_start);
        assert!(wizard.quick_start_entries.is_empty());
    }

    #[test]
    fn issues_launch_agent_opens_issue_wizard() {
        let mut m = test_model();
        m.issues_state.issues = vec![crate::screens::issues::IssueItem {
            number: 42,
            title: "Demo issue".to_string(),
            state: "OPEN".to_string(),
            labels: vec![],
        }];

        update(
            &mut m,
            Message::IssuesMsg(crate::screens::issues::IssuesMessage::LaunchAgent),
        );

        let wizard = m.wizard.expect("wizard should open");
        assert_eq!(wizard.branch_name, "feature/issue-42");
        assert!(wizard.is_new_branch);
        assert_eq!(wizard.spec_id.as_deref(), Some("issue-42"));
    }

    #[test]
    fn specs_launch_agent_without_existing_branch_opens_new_branch_wizard() {
        let mut m = test_model();
        m.specs_state.specs = vec![crate::screens::specs::SpecItem {
            dir_name: "SPEC-1776".to_string(),
            id: "1776".to_string(),
            title: "Rebuild".to_string(),
            status: "open".to_string(),
            phase: "draft".to_string(),
            branches: vec![],
        }];
        m.management_tab = ManagementTab::Specs;

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::Enter, KeyModifiers::SHIFT)),
        );

        let wizard = m.wizard.expect("wizard should open");
        assert_eq!(wizard.branch_name, "feature/feature-1776");
        assert!(wizard.is_new_branch);
        assert_eq!(wizard.spec_id.as_deref(), Some("1776"));
    }

    #[test]
    fn specs_launch_agent_with_existing_branches_opens_branch_selector() {
        let mut m = test_model();
        m.specs_state.specs = vec![crate::screens::specs::SpecItem {
            dir_name: "SPEC-1776".to_string(),
            id: "1776".to_string(),
            title: "Rebuild".to_string(),
            status: "open".to_string(),
            phase: "draft".to_string(),
            branches: vec!["feature/feature-1776".to_string()],
        }];
        m.management_tab = ManagementTab::Specs;

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::Enter, KeyModifiers::SHIFT)),
        );

        assert!(m.wizard.is_none());
        assert!(m.specs_state.branch_select_mode);
        assert_eq!(m.specs_state.branch_candidates, vec!["feature/feature-1776"]);
    }

    #[test]
    fn update_paste_writes_raw_text_to_active_pane() {
        let mut m = test_model();
        let reader = add_cat_session(&mut m, "paste");

        update(&mut m, Message::Paste("hello\nworld".to_string()));

        let output = read_from_reader_with_timeout(reader, Duration::from_secs(5));
        let output_str = String::from_utf8_lossy(&output).replace("\r\n", "\n");
        assert!(
            output_str.contains("hello\nworld"),
            "expected pasted text in output, got: {output_str:?}"
        );
    }

    #[test]
    fn update_terminal_view_typing_returns_to_live_follow() {
        let mut m = test_model();
        m.terminal_cols = 20;
        m.terminal_rows = 10;
        let reader = add_cat_session(&mut m, "typed");
        m.vt_parsers
            .insert("pane-typed".to_string(), vt100::Parser::new(6, 20, 2));
        for index in 0..15 {
            update(
                &mut m,
                Message::PtyOutput {
                    pane_id: "pane-typed".into(),
                    data: format!("line-{index}\r\n").into_bytes(),
                },
            );
        }

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::PageUp, KeyModifiers::NONE)),
        );
        assert!(!m.terminal_viewport("pane-typed").unwrap().follow_live);

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::Char('a'), KeyModifiers::NONE)),
        );

        let output = read_from_reader_with_timeout(reader, Duration::from_secs(5));
        let output_str = String::from_utf8_lossy(&output);
        assert!(
            output_str.contains('a'),
            "expected typed input in output, got: {output_str:?}"
        );

        let viewport = m.terminal_viewport("pane-typed").expect("viewport state");
        assert!(viewport.follow_live);
        assert_eq!(viewport.scrollback, 0);
        assert!(m.active_history_parser.is_none());
    }

    #[test]
    fn update_terminal_view_paste_returns_to_live_follow() {
        let mut m = test_model();
        m.terminal_cols = 20;
        m.terminal_rows = 10;
        let reader = add_cat_session(&mut m, "paste-live");
        m.vt_parsers
            .insert("pane-paste-live".to_string(), vt100::Parser::new(6, 20, 2));
        for index in 0..15 {
            update(
                &mut m,
                Message::PtyOutput {
                    pane_id: "pane-paste-live".into(),
                    data: format!("line-{index}\r\n").into_bytes(),
                },
            );
        }

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::PageUp, KeyModifiers::NONE)),
        );
        assert!(!m.terminal_viewport("pane-paste-live").unwrap().follow_live);

        update(&mut m, Message::Paste("hello".to_string()));

        let output = read_from_reader_with_timeout(reader, Duration::from_secs(5));
        let output_str = String::from_utf8_lossy(&output);
        assert!(
            output_str.contains("hello"),
            "expected pasted input in output, got: {output_str:?}"
        );

        let viewport = m
            .terminal_viewport("pane-paste-live")
            .expect("viewport state");
        assert!(viewport.follow_live);
        assert_eq!(viewport.scrollback, 0);
        assert!(m.active_history_parser.is_none());
    }

    #[test]
    fn update_resize() {
        let mut m = test_model();
        update(&mut m, Message::Resize(120, 40));
        assert_eq!(m.terminal_cols, 120);
        assert_eq!(m.terminal_rows, 40);
    }

    #[test]
    fn update_terminal_view_scrolls_scrollback_with_keyboard() {
        let mut m = test_model();
        m.terminal_cols = 20;
        m.terminal_rows = 10;
        seed_session_transcript(&mut m, "s1", 15);

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::PageUp, KeyModifiers::NONE)),
        );

        let viewport = m.terminal_viewport("pane-s1").expect("viewport state");
        assert_eq!(viewport.scrollback, 6);
        assert!(!viewport.follow_live);
        assert_eq!(m.active_history_pane_id.as_deref(), Some("pane-s1"));
    }

    #[test]
    fn update_terminal_view_end_returns_to_live_follow() {
        let mut m = test_model();
        m.terminal_cols = 20;
        m.terminal_rows = 10;
        seed_session_transcript(&mut m, "s1", 15);

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::PageUp, KeyModifiers::NONE)),
        );
        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::End, KeyModifiers::NONE)),
        );

        let viewport = m.terminal_viewport("pane-s1").expect("viewport state");
        assert_eq!(viewport.scrollback, 0);
        assert!(viewport.follow_live);
        assert!(m.active_history_parser.is_none());
    }

    #[test]
    fn update_terminal_view_mouse_scroll_uses_transcript_history() {
        let mut m = test_model();
        m.terminal_cols = 40;
        m.terminal_rows = 10;
        let _reader = add_cat_session(&mut m, "s1");
        m.active_session = 0;
        m.vt_parsers
            .insert("pane-s1".to_string(), vt100::Parser::new(6, 40, 2));
        for index in 0..12 {
            update(
                &mut m,
                Message::PtyOutput {
                    pane_id: "pane-s1".into(),
                    data: format!("line-{index}\r\n").into_bytes(),
                },
            );
        }

        update(
            &mut m,
            Message::MouseInput(make_mouse(MouseEventKind::ScrollUp)),
        );

        let viewport = m.terminal_viewport("pane-s1").expect("viewport state");
        assert_eq!(viewport.scrollback, 1);
        assert!(!viewport.follow_live);
        assert_eq!(m.active_history_pane_id.as_deref(), Some("pane-s1"));
    }

    #[test]
    fn update_terminal_view_mouse_drag_copies_selection() {
        clear_test_clipboard();

        let mut m = test_model();
        m.terminal_cols = 40;
        m.terminal_rows = 10;
        m.add_session(test_session("s1"));
        let mut parser = vt100::Parser::new(7, 40, 100);
        parser.process(b"hello world");
        m.vt_parsers.insert("pane-s1".to_string(), parser);

        update(
            &mut m,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 0,
                row: 2,
                modifiers: KeyModifiers::NONE,
            }),
        );
        update(
            &mut m,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: 4,
                row: 2,
                modifiers: KeyModifiers::NONE,
            }),
        );
        update(
            &mut m,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 4,
                row: 2,
                modifiers: KeyModifiers::NONE,
            }),
        );

        assert_eq!(take_test_clipboard(), vec!["hello".to_string()]);
    }

    #[test]
    fn update_terminal_view_drag_at_bottom_returns_to_live_follow() {
        let mut m = test_model();
        m.terminal_cols = 40;
        m.terminal_rows = 10;
        m.add_session(test_session("s1"));
        let mut parser = vt100::Parser::new(7, 40, 100);
        parser.process(b"hello world");
        m.vt_parsers.insert("pane-s1".to_string(), parser);

        update(
            &mut m,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 0,
                row: 2,
                modifiers: KeyModifiers::NONE,
            }),
        );
        update(
            &mut m,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: 4,
                row: 2,
                modifiers: KeyModifiers::NONE,
            }),
        );
        update(
            &mut m,
            Message::MouseInput(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 4,
                row: 2,
                modifiers: KeyModifiers::NONE,
            }),
        );

        let viewport = m.terminal_viewport("pane-s1").expect("viewport state");
        assert!(viewport.follow_live);
        assert_eq!(viewport.scrollback, 0);
        assert!(viewport.selection_anchor.is_none());
        assert!(viewport.selection_focus.is_none());
    }

    #[test]
    fn update_terminal_view_preserves_viewport_on_pty_output() {
        let mut m = test_model();
        m.terminal_cols = 20;
        m.terminal_rows = 10;
        seed_session_transcript(&mut m, "s1", 15);

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::PageUp, KeyModifiers::NONE)),
        );
        let before = m.terminal_viewport("pane-s1").unwrap().scrollback;

        update(
            &mut m,
            Message::PtyOutput {
                pane_id: "pane-s1".into(),
                data: b"later line\r\n".to_vec(),
            },
        );

        let after = m.terminal_viewport("pane-s1").unwrap().scrollback;
        assert!(before > 0);
        assert!(after >= before);
        assert_eq!(after, before + 1);
    }

    #[test]
    fn update_terminal_view_renders_old_lines_from_file_backed_scrollback() {
        let mut m = test_model();
        m.terminal_cols = 40;
        m.terminal_rows = 10;

        let _reader = add_cat_session(&mut m, "history");
        m.vt_parsers
            .insert("pane-history".to_string(), vt100::Parser::new(6, 40, 2));

        for index in 0..12 {
            update(
                &mut m,
                Message::PtyOutput {
                    pane_id: "pane-history".into(),
                    data: format!("line-{index}\r\n").into_bytes(),
                },
            );
        }

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::Home, KeyModifiers::NONE)),
        );

        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&m, f)).unwrap();
        let rendered = buffer_text(content_area_rect(40, 10), terminal.backend().buffer());

        assert!(
            rendered.contains("line-0"),
            "expected earliest line to be visible from file-backed history, got: {rendered:?}"
        );
    }

    #[test]
    fn update_terminal_view_preserves_ansi_style_from_file_backed_scrollback() {
        let mut m = test_model();
        m.terminal_cols = 40;
        m.terminal_rows = 10;

        let _reader = add_cat_session(&mut m, "history-ansi");
        m.session_layout_mode = SessionLayoutMode::Maximized;
        m.vt_parsers.insert(
            "pane-history-ansi".to_string(),
            vt100::Parser::new(6, 40, 2),
        );

        update(
            &mut m,
            Message::PtyOutput {
                pane_id: "pane-history-ansi".into(),
                data: b"\x1b[31mred-old\x1b[0m\r\n".to_vec(),
            },
        );
        for index in 1..12 {
            update(
                &mut m,
                Message::PtyOutput {
                    pane_id: "pane-history-ansi".into(),
                    data: format!("line-{index}\r\n").into_bytes(),
                },
            );
        }

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::Home, KeyModifiers::NONE)),
        );

        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&m, f)).unwrap();

        let area = content_area_rect(40, 10);
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(area.x, area.y)].symbol(), "r");
        assert_eq!(buffer[(area.x, area.y)].fg, Color::Indexed(1));
    }

    #[test]
    fn wants_mouse_capture_in_main_with_sessions() {
        let mut m = test_model();
        assert!(wants_mouse_capture(&m));

        m.add_session(test_session("s1"));
        assert!(wants_mouse_capture(&m));
    }

    #[test]
    fn update_mouse_scroll_down_moves_logs_selection() {
        let mut m = test_model();
        m.active_layer = ActiveLayer::Management;
        m.management_tab = ManagementTab::Logs;
        m.overlay_mode = OverlayMode::None;
        m.logs_state.entries = vec![
            test_log_entry("2026-04-01T00:00:01Z", "first"),
            test_log_entry("2026-04-01T00:00:00Z", "second"),
        ];

        update(
            &mut m,
            Message::MouseInput(make_mouse(MouseEventKind::ScrollDown)),
        );

        assert_eq!(m.logs_state.selected, 1);
    }

    #[test]
    fn update_mouse_scroll_up_moves_logs_selection() {
        let mut m = test_model();
        m.active_layer = ActiveLayer::Management;
        m.management_tab = ManagementTab::Logs;
        m.overlay_mode = OverlayMode::None;
        m.logs_state.entries = vec![
            test_log_entry("2026-04-01T00:00:01Z", "first"),
            test_log_entry("2026-04-01T00:00:00Z", "second"),
        ];
        m.logs_state.selected = 1;

        update(
            &mut m,
            Message::MouseInput(make_mouse(MouseEventKind::ScrollUp)),
        );

        assert_eq!(m.logs_state.selected, 0);
    }

    #[test]
    fn update_pty_output_feeds_parser() {
        let mut m = test_model();
        m.vt_parsers
            .insert("pane-1".to_string(), vt100::Parser::new(24, 80, 0));
        update(
            &mut m,
            Message::PtyOutput {
                pane_id: "pane-1".into(),
                data: b"hello".to_vec(),
            },
        );
        let screen = m.vt_parsers["pane-1"].screen();
        let row = screen.contents_between(0, 0, 0, 5);
        assert_eq!(row, "hello");
    }

    #[test]
    fn update_tick_increments() {
        let mut m = test_model();
        update(&mut m, Message::Tick);
        assert_eq!(m.tick_count, 1);
    }

    #[test]
    fn management_tab_tab_key_cycles_in_expected_order() {
        let mut m = test_model();
        m.active_layer = ActiveLayer::Management;
        m.management_tab = ManagementTab::Branches;

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::Tab, KeyModifiers::NONE)),
        );
        assert_eq!(m.management_tab, ManagementTab::Specs);

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::Tab, KeyModifiers::NONE)),
        );
        assert_eq!(m.management_tab, ManagementTab::Issues);

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::Tab, KeyModifiers::NONE)),
        );
        assert_eq!(m.management_tab, ManagementTab::Profiles);

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::Tab, KeyModifiers::NONE)),
        );
        assert_eq!(m.management_tab, ManagementTab::Branches);
    }

    #[test]
    fn specs_detail_reads_spec_file_when_metadata_id_is_numeric() {
        let dir = tempfile::tempdir().unwrap();
        let specs_dir = dir.path().join("specs");
        std::fs::create_dir(&specs_dir).unwrap();

        let spec_dir = specs_dir.join("SPEC-100");
        std::fs::create_dir(&spec_dir).unwrap();
        std::fs::write(
            spec_dir.join("metadata.json"),
            r#"{"id":"100","title":"Numeric id spec","status":"open","phase":"planning","created_at":"2026-04-02T00:00:00Z","updated_at":"2026-04-02T00:00:00Z"}"#,
        )
        .unwrap();
        std::fs::write(spec_dir.join("spec.md"), "# Heading\n\nBody line\n").unwrap();

        let mut m = Model::new(dir.path().to_path_buf());
        m.active_layer = ActiveLayer::Management;
        m.management_tab = ManagementTab::Specs;
        m.specs_state.specs = crate::screens::specs::load_specs(dir.path());

        update(
            &mut m,
            Message::KeyInput(make_key(KeyCode::Enter, KeyModifiers::NONE)),
        );

        assert!(m.specs_state.detail_mode);
        assert!(m.specs_state.detail_content.contains("# Heading"));
        assert!(
            !m.specs_state.detail_content.contains("Could not read"),
            "detail content should load spec.md, got: {}",
            m.specs_state.detail_content
        );
    }

    #[test]
    fn update_error_push_and_dismiss() {
        let mut m = test_model();
        update(
            &mut m,
            Message::PushError(ErrorEntry {
                message: "fail".into(),
                severity: ErrorSeverity::Critical,
            }),
        );
        assert_eq!(m.error_queue.len(), 1);
        update(&mut m, Message::DismissError);
        assert!(m.error_queue.is_empty());
    }

    // -- Key → Message conversion tests ----------------------------------------

    #[test]
    fn action_to_message_maps_correctly() {
        let dummy_key = make_key(KeyCode::Char('x'), KeyModifiers::NONE);

        assert!(action_to_message(KeyAction::None, dummy_key).is_none());
        assert!(matches!(
            action_to_message(KeyAction::Quit, dummy_key),
            Some(Message::Quit)
        ));
        assert!(matches!(
            action_to_message(KeyAction::ToggleLayer, dummy_key),
            Some(Message::ToggleLayer)
        ));
        assert!(matches!(
            action_to_message(KeyAction::NextSession, dummy_key),
            Some(Message::NextSession)
        ));
        assert!(matches!(
            action_to_message(KeyAction::NewShell, dummy_key),
            Some(Message::NewShell)
        ));
        assert!(matches!(
            action_to_message(KeyAction::SwitchSession(3), dummy_key),
            Some(Message::SwitchSession(3))
        ));
    }

    #[test]
    fn action_forward_produces_key_input() {
        let key = make_key(KeyCode::Char('a'), KeyModifiers::NONE);
        let msg = action_to_message(KeyAction::Forward(key), key);
        assert!(matches!(msg, Some(Message::KeyInput(_))));
    }

    // -- View smoke test -------------------------------------------------------

    #[test]
    fn view_renders_without_panic() {
        let model = test_model();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
    }

    #[test]
    fn view_with_sessions_renders_without_panic() {
        let mut model = test_model();
        model.add_session(test_session("shell-1"));
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
    }

    #[test]
    fn view_main_layer_grid_renders_all_session_titles() {
        let mut model = test_model();
        model.add_session(test_session("s1"));
        model.add_session(test_session("s2"));
        model.session_layout_mode = SessionLayoutMode::Grid;
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
        let rendered = buffer_text(Rect::new(0, 0, 80, 24), terminal.backend().buffer());
        assert!(rendered.contains("s1"), "expected s1 title in grid");
        assert!(rendered.contains("s2"), "expected s2 title in grid");
    }

    #[test]
    fn view_with_error_overlay_renders() {
        let mut model = test_model();
        model.push_error(ErrorEntry {
            message: "Something went wrong".into(),
            severity: ErrorSeverity::Critical,
        });
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
    }

    #[test]
    fn view_with_progress_renders() {
        let mut model = test_model();
        model.progress = Some(crate::widgets::progress_modal::ProgressState::simple(
            "Loading...",
            Some("step 1"),
        ));
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
    }

    // -- Quit confirmation tests ------------------------------------------------

    #[test]
    fn quit_with_running_agents_shows_confirm() {
        let mut m = test_model();
        m.add_session(SessionTab {
            pane_id: "p1".into(),
            name: "Agent #1".into(),
            tab_type: SessionTabType::Agent,
            color: AgentColor::Blue,
            status: SessionStatus::Running,
            branch: Some("feature/test".into()),
            spec_id: None,
        });
        update(&mut m, Message::Quit);
        assert!(
            !m.should_quit,
            "Should not quit immediately with running agents"
        );
        assert!(m.confirm.is_some(), "Confirm dialog should appear");
        assert_eq!(m.overlay_mode, OverlayMode::Confirm);
    }

    #[test]
    fn quit_without_agents_exits_immediately() {
        let mut m = test_model();
        // Only shell sessions — no agents
        m.add_session(test_session("shell-1"));
        update(&mut m, Message::Quit);
        assert!(
            m.should_quit,
            "Should quit immediately with no running agents"
        );
    }

    #[test]
    fn confirm_accepted_quits() {
        let mut m = test_model();
        m.confirm = Some(crate::screens::confirm::ConfirmState::exit_with_running_agents(1));
        m.overlay_mode = OverlayMode::Confirm;
        update(&mut m, Message::ConfirmAccepted);
        assert!(m.should_quit);
        assert!(m.confirm.is_none());
    }

    #[test]
    fn confirm_cancelled_does_not_quit() {
        let mut m = test_model();
        m.confirm = Some(crate::screens::confirm::ConfirmState::exit_with_running_agents(1));
        m.overlay_mode = OverlayMode::Confirm;
        update(&mut m, Message::ConfirmCancelled);
        assert!(!m.should_quit);
        assert!(m.confirm.is_none());
    }

    // -- Versions tab view test -------------------------------------------------

    #[test]
    fn view_versions_tab_renders() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Versions;
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&model, f)).unwrap();
    }
}
