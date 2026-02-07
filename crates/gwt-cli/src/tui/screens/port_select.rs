//! Port selection screen for resolving docker compose published port conflicts (SPEC-f5f5657e)
//!
//! This screen is shown when gwt detects that a default host port is already in use.

use ratatui::{prelude::*, widgets::*};
use std::collections::{HashMap, HashSet};

const DEFAULT_CANDIDATE_LIMIT: usize = 12;
const CANDIDATE_PREVIEW_LIMIT: usize = 7;

#[derive(Debug, Clone)]
pub struct PortSelectItem {
    pub env_name: String,
    pub default_port: u16,
    pub conflicting_port: u16,
    pub selected_port: u16,
    pub suggested_port: u16,
    pub candidates: Vec<u16>,
    pub selected_candidate: usize,
}

#[derive(Debug, Clone, Default)]
pub struct CustomPortInput {
    pub value: String,
}

/// Port selection screen state
#[derive(Debug, Default)]
pub struct PortSelectState {
    pub items: Vec<PortSelectItem>,
    pub selected: usize,
    pub custom_input: Option<CustomPortInput>,
    pub error: Option<String>,
    pub container_name: Option<String>,
    pub worktree_name: Option<String>,
    pub service: Option<String>,
}

impl PortSelectState {
    #[cfg(test)]
    pub fn new(items: Vec<(String, u16)>) -> Self {
        let items = items
            .into_iter()
            .map(|(env_name, default_port)| PortSelectItem {
                env_name,
                default_port,
                conflicting_port: default_port,
                selected_port: default_port,
                suggested_port: default_port,
                candidates: if default_port == 0 {
                    Vec::new()
                } else {
                    vec![default_port]
                },
                selected_candidate: 0,
            })
            .collect();
        Self {
            items,
            selected: 0,
            custom_input: None,
            error: None,
            container_name: None,
            worktree_name: None,
            service: None,
        }
    }

    pub fn from_conflicts<F>(
        conflicts: Vec<(String, u16, u16)>,
        docker_ports: &HashSet<u16>,
        is_taken: F,
    ) -> Self
    where
        F: Fn(u16) -> bool,
    {
        let is_taken = |port: u16| docker_ports.contains(&port) || is_taken(port);

        let mut already_selected = HashSet::new();
        let mut items = Vec::with_capacity(conflicts.len());

        for (env_name, default_port, conflicting_port) in conflicts {
            let base_port = if conflicting_port == 0 {
                default_port.max(1)
            } else {
                conflicting_port
            };
            let mut candidates = build_conflict_candidates(
                base_port,
                default_port,
                &is_taken,
                DEFAULT_CANDIDATE_LIMIT,
            );
            if candidates.is_empty() {
                candidates = Vec::new();
            }

            let (selected_candidate, selected_port) = candidates
                .iter()
                .enumerate()
                .find(|(_, port)| !already_selected.contains(*port))
                .map(|(idx, port)| (idx, *port))
                .or_else(|| candidates.first().copied().map(|port| (0, port)))
                .unwrap_or((0, default_port.max(1)));

            already_selected.insert(selected_port);

            items.push(PortSelectItem {
                env_name,
                default_port,
                conflicting_port,
                selected_port,
                suggested_port: selected_port,
                candidates,
                selected_candidate,
            });
        }

        items.sort_by(|a, b| a.env_name.cmp(&b.env_name));

        Self {
            items,
            selected: 0,
            custom_input: None,
            error: None,
            container_name: None,
            worktree_name: None,
            service: None,
        }
    }

    pub fn set_context(
        &mut self,
        container_name: &str,
        worktree_name: &str,
        service: Option<&str>,
    ) {
        self.container_name = Some(container_name.to_string());
        self.worktree_name = Some(worktree_name.to_string());
        self.service = service.map(|s| s.to_string());
    }

    pub fn select_next(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
        }
        self.error = None;
    }

    pub fn select_previous(&mut self) {
        if !self.items.is_empty() {
            self.selected = self.selected.saturating_sub(1);
        }
        self.error = None;
    }

    pub fn cycle_candidate_next(&mut self) {
        let Some(item) = self.items.get_mut(self.selected) else {
            return;
        };
        if item.candidates.is_empty() {
            return;
        }
        item.selected_candidate = (item.selected_candidate + 1) % item.candidates.len();
        item.selected_port = item.candidates[item.selected_candidate];
        self.error = None;
    }

    pub fn cycle_candidate_prev(&mut self) {
        let Some(item) = self.items.get_mut(self.selected) else {
            return;
        };
        if item.candidates.is_empty() {
            return;
        }
        if item.selected_candidate == 0 {
            item.selected_candidate = item.candidates.len().saturating_sub(1);
        } else {
            item.selected_candidate = item.selected_candidate.saturating_sub(1);
        }
        item.selected_port = item.candidates[item.selected_candidate];
        self.error = None;
    }

    pub fn open_custom_input(&mut self) {
        self.custom_input = Some(CustomPortInput::default());
        self.error = None;
    }

    pub fn insert_custom_char(&mut self, c: char) {
        let Some(input) = self.custom_input.as_mut() else {
            return;
        };
        input.value.push(c);
        self.error = None;
    }

    pub fn backspace_custom(&mut self) {
        if let Some(input) = self.custom_input.as_mut() {
            input.value.pop();
        }
        self.error = None;
    }

    pub fn cancel_custom_input(&mut self) {
        self.custom_input = None;
        self.error = None;
    }

    pub fn validate_unique_ports(&self) -> bool {
        let mut seen = HashSet::new();
        for item in &self.items {
            if !seen.insert(item.selected_port) {
                return false;
            }
        }
        true
    }

    pub fn reset_selected_to_suggested(&mut self) {
        let Some(item) = self.items.get_mut(self.selected) else {
            return;
        };
        item.selected_port = item.suggested_port;
        if let Some(idx) = item
            .candidates
            .iter()
            .position(|port| *port == item.suggested_port)
        {
            item.selected_candidate = idx;
        } else {
            item.candidates.insert(0, item.suggested_port);
            item.selected_candidate = 0;
        }
        self.error = None;
    }

    pub fn is_port_selected_elsewhere(&self, port: u16) -> bool {
        self.items
            .iter()
            .enumerate()
            .any(|(idx, item)| idx != self.selected && item.selected_port == port)
    }

    pub fn validate_selected_ports<F>(&self, is_taken: F) -> Result<(), String>
    where
        F: Fn(u16) -> bool,
    {
        if self.items.is_empty() {
            return Ok(());
        }

        // Ensure ports are valid before checking collisions.
        for item in &self.items {
            if item.selected_port == 0 {
                return Err(format!("Invalid port for {}", item.env_name));
            }
        }

        // Ensure uniqueness inside this selection.
        let mut seen = HashSet::new();
        for item in &self.items {
            if !seen.insert(item.selected_port) {
                return Err("Ports must be unique".to_string());
            }
        }

        // Ensure ports are not already in use.
        for item in &self.items {
            if is_taken(item.selected_port) {
                return Err(format!(
                    "Port {} for {} is already in use",
                    item.selected_port, item.env_name
                ));
            }
        }

        Ok(())
    }

    pub fn build_env_overrides(&self) -> HashMap<String, String> {
        self.items
            .iter()
            .map(|item| (item.env_name.clone(), item.selected_port.to_string()))
            .collect()
    }

    pub fn apply_custom_port<F>(&mut self, is_taken: F) -> Result<(), String>
    where
        F: Fn(u16) -> bool,
    {
        let Some(input) = self.custom_input.as_mut() else {
            return Ok(());
        };
        let parsed = input
            .value
            .parse::<u16>()
            .map_err(|_| "Invalid port".to_string())?;
        if parsed == 0 {
            return Err("Invalid port".to_string());
        }
        if is_taken(parsed) {
            return Err(format!("Port {} is already in use", parsed));
        }
        let selected = self.selected;
        let Some(prev) = self
            .items
            .get(selected)
            .map(|i| (i.selected_port, i.selected_candidate))
        else {
            return Ok(());
        };

        let mut inserted = false;
        {
            let Some(item) = self.items.get_mut(selected) else {
                return Ok(());
            };
            item.selected_port = parsed;
            if !item.candidates.contains(&parsed) {
                item.candidates.insert(0, parsed);
                inserted = true;
            }
            item.selected_candidate = item
                .candidates
                .iter()
                .position(|port| *port == parsed)
                .unwrap_or(0);
        }

        if !self.validate_unique_ports() {
            if let Some(item) = self.items.get_mut(selected) {
                // Revert and show a clear message so users can pick a different port.
                item.selected_port = prev.0;
                item.selected_candidate = prev.1;
                if inserted && item.candidates.first() == Some(&parsed) {
                    item.candidates.remove(0);
                }
            }
            return Err("Ports must be unique".to_string());
        }
        self.custom_input = None;
        self.error = None;
        Ok(())
    }
}

#[cfg(test)]
pub fn build_candidate_ports<F>(base_port: u16, is_taken: F, limit: usize) -> Vec<u16>
where
    F: Fn(u16) -> bool,
{
    build_conflict_candidates(base_port, base_port, &is_taken, limit)
}

fn build_conflict_candidates<F>(
    base_port: u16,
    default_port: u16,
    is_taken: &F,
    limit: usize,
) -> Vec<u16>
where
    F: Fn(u16) -> bool,
{
    if limit == 0 {
        return Vec::new();
    }

    let base_port = if base_port == 0 { 1 } else { base_port };
    let mut ports = Vec::with_capacity(limit);
    let mut current = base_port;

    // Scan all ports at most once (wrap-around), but stop early when we have enough.
    for _ in 0..=u16::MAX {
        if current != 0 && !is_taken(current) && !ports.contains(&current) {
            ports.push(current);
            if ports.len() >= limit {
                break;
            }
        }
        current = current.wrapping_add(1);
        if current == base_port {
            break;
        }
    }

    // Make sure default port is visible as an option (but keep the first suggestion stable).
    if default_port != 0 && !is_taken(default_port) && !ports.contains(&default_port) {
        if ports.is_empty() {
            ports.push(default_port);
        } else {
            ports.insert(1.min(ports.len()), default_port);
            ports.truncate(limit);
        }
    }

    ports
}

pub fn render_port_select(state: &mut PortSelectState, frame: &mut Frame, area: Rect) {
    // Calculate popup size based on content
    let popup_width = 80.min(area.width.saturating_sub(4));
    let min_height = 12u16;
    let popup_height = (state.items.len() as u16 + 10)
        .max(min_height)
        .min(area.height.saturating_sub(4));

    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    // Clear background
    frame.render_widget(Clear, popup_area);

    // Outer block
    let block = Block::default()
        .title(" Port Conflict ")
        .title_style(Style::default().fg(Color::Cyan).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .border_type(BorderType::Rounded);
    frame.render_widget(block, popup_area);

    // Inner area for content
    let inner = Rect {
        x: popup_area.x + 2,
        y: popup_area.y + 1,
        width: popup_area.width.saturating_sub(4),
        height: popup_area.height.saturating_sub(2),
    };

    let header_height = 4u16;
    let footer_height = 2u16;
    let error_height = if state.error.is_some() { 2u16 } else { 1u16 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(1),
            Constraint::Length(error_height),
            Constraint::Length(footer_height),
        ])
        .split(inner);

    // Header with context
    let worktree = state
        .worktree_name
        .as_deref()
        .unwrap_or("(unknown worktree)");
    let container = state
        .container_name
        .as_deref()
        .unwrap_or("(unknown container)");
    let service = state.service.as_deref().unwrap_or("(default)");

    let header_lines = vec![
        Line::from(vec![
            Span::styled("Worktree: ", Style::default().fg(Color::DarkGray)),
            Span::styled(worktree, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Container: ", Style::default().fg(Color::DarkGray)),
            Span::styled(container, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("Service: ", Style::default().fg(Color::DarkGray)),
            Span::styled(service, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![Span::styled(
            "Select replacement ports to continue.",
            Style::default(),
        )]),
    ];
    frame.render_widget(Paragraph::new(header_lines), chunks[0]);

    // List items
    let items: Vec<ListItem> = state
        .items
        .iter()
        .map(|item| {
            let env_width = 18usize.min(chunks[1].width as usize);
            let mut env = item.env_name.clone();
            if env.chars().count() > env_width && env_width > 3 {
                env = env.chars().take(env_width - 3).collect::<String>() + "...";
            }
            let env = format!("{: <width$}", env, width = env_width);

            let mut spans = Vec::new();
            spans.push(Span::styled(env, Style::default().fg(Color::White)));
            spans.push(Span::raw(" "));

            spans.push(Span::styled(
                format!("{}", item.conflicting_port),
                Style::default().fg(Color::Red),
            ));
            spans.push(Span::styled(" -> ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!("{}", item.selected_port),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));

            if item.default_port != item.conflicting_port {
                spans.push(Span::styled(
                    format!(" (default {})", item.default_port),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
            spans.extend(candidate_preview_spans(item, CANDIDATE_PREVIEW_LIMIT));

            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(Style::default().bg(Color::Cyan))
        .highlight_symbol("> ");

    let mut list_state = ListState::default();
    if !state.items.is_empty() {
        list_state.select(Some(state.selected));
    }
    frame.render_stateful_widget(list, chunks[1], &mut list_state);

    // Error / hint line
    let hint = if let Some(msg) = state.error.as_deref() {
        Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )))
    } else {
        Paragraph::new(Line::from(Span::styled(
            "If you need a specific value, press 'c' to enter a custom port.",
            Style::default().fg(Color::DarkGray),
        )))
    };
    frame.render_widget(hint, chunks[2]);

    // Footer with instructions
    let footer = if state.custom_input.is_some() {
        Paragraph::new(Line::from(vec![
            Span::styled("[0-9]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Type  "),
            Span::styled("[Backspace]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Delete  "),
            Span::styled("[Enter]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Apply  "),
            Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Cancel"),
        ]))
        .alignment(Alignment::Center)
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("[Up/Down]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Select  "),
            Span::styled("[Left/Right]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Change  "),
            Span::styled("[C]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Custom  "),
            Span::styled("[A]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Auto  "),
            Span::styled("[Enter]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Continue  "),
            Span::styled("[Esc]", Style::default().fg(Color::DarkGray)),
            Span::raw(" Cancel"),
        ]))
        .alignment(Alignment::Center)
    };
    frame.render_widget(footer, chunks[3]);

    if let Some(input) = state.custom_input.as_ref() {
        render_custom_input(input, state.error.as_deref(), frame, area);
    }
}

fn candidate_preview_spans(item: &PortSelectItem, max: usize) -> Vec<Span<'static>> {
    if item.candidates.is_empty() || max == 0 {
        return vec![Span::styled(
            "(no candidates)",
            Style::default().fg(Color::DarkGray),
        )];
    }

    let len = item.candidates.len();
    let max = max.min(len);
    let half = max / 2;
    let mut start = item.selected_candidate.saturating_sub(half);
    if start + max > len {
        start = len.saturating_sub(max);
    }
    let end = (start + max).min(len);

    let mut spans: Vec<Span<'static>> = Vec::new();
    if start > 0 {
        spans.push(Span::styled("... ", Style::default().fg(Color::DarkGray)));
    }

    for (idx, port) in item.candidates[start..end].iter().enumerate() {
        if idx > 0 {
            spans.push(Span::raw(" "));
        }
        if *port == item.selected_port {
            spans.push(Span::styled(
                format!("[{}]", port),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                port.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    if end < len {
        spans.push(Span::styled(" ...", Style::default().fg(Color::DarkGray)));
    }

    spans
}

fn render_custom_input(
    input: &CustomPortInput,
    error: Option<&str>,
    frame: &mut Frame,
    area: Rect,
) {
    let popup_width = 52.min(area.width.saturating_sub(6));
    let popup_height = 9.min(area.height.saturating_sub(6));
    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Custom Port ")
        .title_style(Style::default().fg(Color::Cyan).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .border_type(BorderType::Rounded);
    frame.render_widget(block, popup_area);

    let inner = Rect {
        x: popup_area.x + 2,
        y: popup_area.y + 1,
        width: popup_area.width.saturating_sub(4),
        height: popup_area.height.saturating_sub(2),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(Paragraph::new("Enter a port number (1-65535)."), chunks[0]);

    let input_value = if input.value.is_empty() {
        Span::styled("(empty)", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(input.value.clone(), Style::default().fg(Color::White))
    };

    let input_box = Paragraph::new(Line::from(vec![
        Span::styled("Port: ", Style::default().fg(Color::DarkGray)),
        input_value,
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if error.is_some() {
                Color::Red
            } else {
                Color::DarkGray
            })),
    );
    frame.render_widget(input_box, chunks[1]);

    if let Some(msg) = error {
        frame.render_widget(
            Paragraph::new(Span::styled(
                msg,
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            chunks[2],
        );
    } else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "Press Enter to apply, Esc to cancel.",
                Style::default().fg(Color::DarkGray),
            )),
            chunks[2],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_build_candidate_ports_skips_taken() {
        let mut used = HashSet::new();
        used.insert(3000u16);
        used.insert(3001u16);

        let ports = build_candidate_ports(3000, |p| used.contains(&p), 3);
        assert_eq!(ports, vec![3002u16, 3003u16, 3004u16]);
    }

    #[test]
    fn test_validate_unique_ports_detects_duplicates() {
        let mut state = PortSelectState::new(vec![
            ("PORT".to_string(), 3000),
            ("OTHER_PORT".to_string(), 4000),
        ]);
        // Force duplicate selection
        state.items[0].selected_port = 10000;
        state.items[1].selected_port = 10000;

        assert!(!state.validate_unique_ports());
    }

    #[test]
    fn test_apply_custom_port_rejects_taken_port() {
        let mut state = PortSelectState::new(vec![("PORT".to_string(), 3000)]);
        state.open_custom_input();
        for c in "3000".chars() {
            state.insert_custom_char(c);
        }

        let res = state.apply_custom_port(|port| port == 3000);
        assert!(res.is_err());
    }

    #[test]
    fn test_from_conflicts_suggests_free_ports_and_unique() {
        let docker_ports: HashSet<u16> = [3000u16, 3001u16].into_iter().collect();
        let state = PortSelectState::from_conflicts(
            vec![
                ("PORT".to_string(), 3000, 3000),
                ("OTHER".to_string(), 3000, 3000),
            ],
            &docker_ports,
            |_p| false,
        );

        assert_eq!(state.items.len(), 2);
        assert_ne!(state.items[0].selected_port, 3000);
        assert_ne!(state.items[1].selected_port, 3000);
        assert!(state.validate_unique_ports());
    }

    #[test]
    fn test_reset_selected_to_suggested() {
        let docker_ports: HashSet<u16> = [3000u16].into_iter().collect();
        let mut state = PortSelectState::from_conflicts(
            vec![("PORT".to_string(), 3000, 3000)],
            &docker_ports,
            |_p| false,
        );
        let suggested = state.items[0].suggested_port;
        state.items[0].selected_port = suggested.saturating_add(10);
        state.reset_selected_to_suggested();
        assert_eq!(state.items[0].selected_port, suggested);
    }

    #[test]
    fn test_validate_selected_ports_rejects_duplicates() {
        let mut state =
            PortSelectState::new(vec![("A".to_string(), 3000), ("B".to_string(), 4000)]);
        state.items[0].selected_port = 1234;
        state.items[1].selected_port = 1234;
        let res = state.validate_selected_ports(|_p| false);
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "Ports must be unique");
    }
}
