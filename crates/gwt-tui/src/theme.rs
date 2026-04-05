//! Centralized theme definitions for gwt-tui.
//!
//! All UI styling (colors, borders, icons, pre-composed styles) lives here.
//! Screen and widget code should reference `theme::` constants instead of
//! inline `Color::*` / `Modifier::*` values.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::BorderType;

// ---------------------------------------------------------------------------
// Colors — semantic names for ANSI 16 palette
// ---------------------------------------------------------------------------

pub mod color {
    use ratatui::style::Color;

    /// Active/selected items, input text.
    pub const ACTIVE: Color = Color::Yellow;
    /// Focused elements, headings, interactive highlights.
    pub const FOCUS: Color = Color::Cyan;
    /// Success states, positive indicators.
    pub const SUCCESS: Color = Color::Green;
    /// Error states, danger.
    pub const ERROR: Color = Color::Red;
    /// Warning states.
    pub const WARNING: Color = Color::Yellow;
    /// Inactive, secondary elements.
    pub const MUTED: Color = Color::Gray;
    /// Background fills, status bar background, subtle chrome.
    pub const SURFACE: Color = Color::DarkGray;
    /// Primary content text.
    pub const TEXT_PRIMARY: Color = Color::White;
    /// Secondary content text.
    pub const TEXT_SECONDARY: Color = Color::Gray;
    /// Disabled/placeholder text.
    pub const TEXT_DISABLED: Color = Color::DarkGray;
    /// Focused pane border.
    pub const BORDER_FOCUSED: Color = Color::Cyan;
    /// Unfocused pane border.
    pub const BORDER_UNFOCUSED: Color = Color::Gray;
    /// Error overlay border.
    pub const BORDER_ERROR: Color = Color::Red;
    /// Metadata, alternative highlights.
    pub const ACCENT: Color = Color::Magenta;
    /// Agent-specific coloring.
    pub const AGENT: Color = Color::Blue;
}

// ---------------------------------------------------------------------------
// Borders
// ---------------------------------------------------------------------------

pub mod border {
    use ratatui::widgets::BorderType;

    /// Default pane border — rounded corners for Minimalist Modern tone.
    pub const fn default() -> BorderType {
        BorderType::Rounded
    }

    /// Focused pane border — thick lines for emphasis.
    pub const fn focused() -> BorderType {
        BorderType::Thick
    }

    /// Modal/overlay border — double lines for importance.
    pub const fn modal() -> BorderType {
        BorderType::Double
    }
}

// ---------------------------------------------------------------------------
// Icons — Unicode constants
// ---------------------------------------------------------------------------

pub mod icon {
    /// Branch has an active worktree.
    pub const WORKTREE_ACTIVE: &str = "\u{25C6}"; // ◆
    /// Branch without worktree.
    pub const WORKTREE_INACTIVE: &str = "\u{25C7}"; // ◇
    /// HEAD branch marker.
    pub const HEAD_INDICATOR: &str = " \u{25B8}"; // ▸ (with leading space)
    /// Shell session tab icon.
    pub const SESSION_SHELL: &str = "\u{203A}"; // ›
    /// Agent session tab icon.
    pub const SESSION_AGENT: &str = "\u{25C8}"; // ◈
    /// Tab layout indicator.
    pub const LAYOUT_TAB: &str = "\u{25A3}"; // ▣
    /// Grid layout indicator.
    pub const LAYOUT_GRID: &str = "\u{25A6}"; // ▦
    /// Success status.
    pub const SUCCESS: &str = "\u{2713}"; // ✓
    /// Failed status.
    pub const FAILED: &str = "\u{2717}"; // ✗
    /// In-progress status.
    pub const IN_PROGRESS: &str = "\u{25D0}"; // ◐
    /// Right-pointing triangle (used in docker, service select).
    pub const ARROW_RIGHT: &str = "\u{25B6}"; // ▶
    /// Empty circle (used in docker progress idle state).
    pub const CIRCLE_EMPTY: &str = "\u{25CB}"; // ○
    /// Block cursor for text input.
    pub const BLOCK_CURSOR: &str = "\u{2588}"; // █
    /// Checkmark (completed/resolved).
    pub const CHECKMARK: &str = "\u{2714}"; // ✔
    /// Warning badge.
    pub const WARNING_BADGE: &str = "\u{26A0}"; // ⚠
    /// Bullet list marker.
    pub const BULLET: &str = "\u{2022}"; // •
    /// Vertical separator pipe.
    pub const SEPARATOR_VERT: &str = "\u{2502}"; // │
    /// Git branch symbol.
    pub const GIT_BRANCH: &str = "\u{2387}"; // ⎇
    /// Left accent bar for selected items.
    pub const LEFT_ACCENT: &str = "\u{258E}"; // ▎
    /// Horizontal rule character.
    pub const HRULE: char = '\u{2500}'; // ─
}

// ---------------------------------------------------------------------------
// Pre-composed styles
// ---------------------------------------------------------------------------

pub mod style {
    use ratatui::style::{Modifier, Style};

    use super::color;

    /// Active/selected item: yellow + bold.
    pub const fn active_item() -> Style {
        Style::new()
            .fg(color::ACTIVE)
            .add_modifier(Modifier::BOLD)
    }

    /// List selection highlight: white on dark-gray + bold.
    pub const fn selected_item() -> Style {
        Style::new()
            .fg(color::TEXT_PRIMARY)
            .bg(color::SURFACE)
            .add_modifier(Modifier::BOLD)
    }

    /// Section header: cyan + bold.
    pub const fn header() -> Style {
        Style::new()
            .fg(color::FOCUS)
            .add_modifier(Modifier::BOLD)
    }

    /// Muted/disabled text.
    pub const fn muted_text() -> Style {
        Style::new().fg(color::TEXT_DISABLED)
    }

    /// Error text: red + bold.
    pub const fn error_text() -> Style {
        Style::new()
            .fg(color::ERROR)
            .add_modifier(Modifier::BOLD)
    }

    /// Active tab label.
    pub const fn tab_active() -> Style {
        Style::new()
            .fg(color::ACTIVE)
            .add_modifier(Modifier::BOLD)
    }

    /// Inactive tab label.
    pub const fn tab_inactive() -> Style {
        Style::new().fg(color::MUTED)
    }

    /// Tab separator (│).
    pub const fn tab_separator() -> Style {
        Style::new().fg(color::SURFACE)
    }

    /// Success text: green + bold.
    pub const fn success_text() -> Style {
        Style::new()
            .fg(color::SUCCESS)
            .add_modifier(Modifier::BOLD)
    }

    /// Warning text: yellow + bold.
    pub const fn warning_text() -> Style {
        Style::new()
            .fg(color::WARNING)
            .add_modifier(Modifier::BOLD)
    }

    /// Primary text.
    pub const fn text() -> Style {
        Style::new().fg(color::TEXT_PRIMARY)
    }

    /// Layer badge: reverse video (SURFACE on FOCUS + bold).
    pub const fn layer_badge() -> Style {
        Style::new()
            .fg(color::SURFACE)
            .bg(color::FOCUS)
            .add_modifier(Modifier::BOLD)
    }

    /// Notification style by severity name.
    pub fn notification(severity: &str) -> Style {
        match severity {
            "DEBUG" => Style::new().fg(color::SURFACE),
            "INFO" => success_text(),
            "WARN" => warning_text(),
            "ERROR" => error_text(),
            _ => Style::new().fg(color::TEXT_PRIMARY),
        }
    }
}

// ---------------------------------------------------------------------------
// Border helpers for Block construction
// ---------------------------------------------------------------------------

/// Default pane border style (unfocused).
pub fn pane_border(is_focused: bool) -> (Style, BorderType) {
    if is_focused {
        (
            Style::default().fg(color::BORDER_FOCUSED),
            border::focused(),
        )
    } else {
        (
            Style::default().fg(color::BORDER_UNFOCUSED),
            border::default(),
        )
    }
}

/// Modal overlay border style.
pub fn modal_border(accent: Color) -> (Style, BorderType) {
    (Style::default().fg(accent), border::modal())
}

/// Status bar section separator: ` │ ` in SURFACE color.
pub fn status_separator() -> Span<'static> {
    Span::styled(
        format!(" {} ", icon::SEPARATOR_VERT),
        Style::default().fg(color::SURFACE),
    )
}

/// Decorative section divider: `─── Label ───` fitting the given width.
pub fn section_divider(label: &str, width: u16) -> Line<'static> {
    let label_with_pad = format!(" {} ", label);
    let label_len = label_with_pad.chars().count();
    let remaining = (width as usize).saturating_sub(label_len);
    let left = remaining / 2;
    let right = remaining.saturating_sub(left);
    let left_rule: String = std::iter::repeat_n(icon::HRULE, left).collect();
    let right_rule: String = std::iter::repeat_n(icon::HRULE, right).collect();
    Line::from(vec![
        Span::styled(left_rule, style::muted_text()),
        Span::styled(label_with_pad, style::header()),
        Span::styled(right_rule, style::muted_text()),
    ])
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;

    use super::*;

    #[test]
    fn color_constants_are_ansi16() {
        // Verify all colors are basic ANSI 16 (not Rgb or Indexed)
        let colors = [
            color::ACTIVE,
            color::FOCUS,
            color::SUCCESS,
            color::ERROR,
            color::WARNING,
            color::MUTED,
            color::SURFACE,
            color::TEXT_PRIMARY,
            color::TEXT_SECONDARY,
            color::TEXT_DISABLED,
            color::BORDER_FOCUSED,
            color::BORDER_UNFOCUSED,
            color::BORDER_ERROR,
            color::ACCENT,
            color::AGENT,
        ];
        for c in colors {
            assert!(
                !matches!(c, Color::Rgb(_, _, _) | Color::Indexed(_)),
                "Theme colors must be ANSI 16 base colors"
            );
        }
    }

    #[test]
    fn border_types_are_correct() {
        assert_eq!(border::default(), BorderType::Rounded);
        assert_eq!(border::focused(), BorderType::Thick);
        assert_eq!(border::modal(), BorderType::Double);
    }

    #[test]
    fn icon_constants_are_nonempty() {
        let icons = [
            icon::WORKTREE_ACTIVE,
            icon::WORKTREE_INACTIVE,
            icon::HEAD_INDICATOR,
            icon::SESSION_SHELL,
            icon::SESSION_AGENT,
            icon::LAYOUT_TAB,
            icon::LAYOUT_GRID,
            icon::SUCCESS,
            icon::FAILED,
            icon::IN_PROGRESS,
        ];
        for i in icons {
            assert!(!i.is_empty(), "Icon constants must be non-empty");
        }
    }

    #[test]
    fn style_helpers_have_expected_modifiers() {
        let active = style::active_item();
        assert!(active.add_modifier.contains(Modifier::BOLD));

        let selected = style::selected_item();
        assert!(selected.add_modifier.contains(Modifier::BOLD));

        let header = style::header();
        assert!(header.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pane_border_focused_vs_unfocused() {
        let (focused_style, focused_type) = pane_border(true);
        let (unfocused_style, unfocused_type) = pane_border(false);
        assert_ne!(focused_type, unfocused_type);
        assert_ne!(focused_style, unfocused_style);
    }
}
