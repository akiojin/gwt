use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
    time::Instant,
};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute, queue,
    style::Print,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use gwt_tui::{
    ime_probe::{help_text, ProbeMode, ProbeOptions, ProbeState, INPUT_PROMPT},
    input_trace::append_probe_event_with_path,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
}

fn reset_output_file(path: &Path) {
    let _ = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path);
}

fn main() -> io::Result<()> {
    let options = match ProbeOptions::parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            println!("{message}");
            if message != help_text() {
                std::process::exit(1);
            }
            return Ok(());
        }
    };
    reset_output_file(&options.output_path);

    enter_probe_terminal()?;
    let result = match options.mode {
        ProbeMode::Ratatui => run_ratatui_probe(&options),
        ProbeMode::Raw | ProbeMode::Redraw => run_plain_probe(&options),
    };
    leave_probe_terminal()?;
    println!("JSONL saved to {}", options.output_path.display());
    result
}

fn enter_probe_terminal() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
    );
    Ok(())
}

fn leave_probe_terminal() -> io::Result<()> {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    disable_raw_mode()?;
    execute!(stdout, Show, LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

fn run_plain_probe(options: &ProbeOptions) -> io::Result<()> {
    let mut stdout = io::stdout();
    let mut state = ProbeState::new();
    render_plain(&mut stdout, options, &state)?;
    let mut next_frame = Instant::now() + options.tick_rate;

    loop {
        let wait = if options.needs_periodic_redraw() {
            next_frame.saturating_duration_since(Instant::now())
        } else {
            options.tick_rate
        };

        if event::poll(wait)? {
            let event = event::read()?;
            append_probe_event_with_path(&options.output_path, &event)?;
            state.record_event(&event);
            if let Event::Key(key) = event {
                if state.handle_key_event(key) {
                    return Ok(());
                }
            }
            render_plain(&mut stdout, options, &state)?;
        } else if options.needs_periodic_redraw() {
            state.tick_frame();
            next_frame = Instant::now() + options.tick_rate;
            render_plain(&mut stdout, options, &state)?;
        }
    }
}

fn render_plain(
    stdout: &mut io::Stdout,
    options: &ProbeOptions,
    state: &ProbeState,
) -> io::Result<()> {
    queue!(stdout, Hide, MoveTo(0, 0), Clear(ClearType::All))?;
    queue!(stdout, Print("gwt-tui layered IME probe\r\n"))?;
    queue!(
        stdout,
        Print(format!(
            "Mode: {}  Tick: {}ms  Frames: {}  Events: {}\r\n",
            options.mode.as_str(),
            options.tick_rate.as_millis(),
            state.frame_count(),
            state.event_count()
        ))
    )?;
    queue!(
        stdout,
        Print(format!(
            "JSONL output: {}\r\n",
            options.output_path.display()
        ))
    )?;
    queue!(stdout, Print("Type with your IME. Ctrl+C twice exits.\r\n"))?;
    queue!(
        stdout,
        MoveTo(0, 5),
        Print(format!("{INPUT_PROMPT}{}", state.input_buffer()))
    )?;

    let mut row = 7u16;
    queue!(stdout, MoveTo(0, row), Print("Submitted:"))?;
    row = row.saturating_add(1);
    for line in state.submitted_lines().iter().rev().take(4).rev() {
        queue!(stdout, MoveTo(0, row), Print(line))?;
        row = row.saturating_add(1);
    }

    row = row.saturating_add(1);
    queue!(stdout, MoveTo(0, row), Print("Recent events:"))?;
    row = row.saturating_add(1);
    for line in state.recent_events() {
        queue!(stdout, MoveTo(0, row), Print(line))?;
        row = row.saturating_add(1);
    }

    queue!(stdout, MoveTo(state.cursor_column(), 5), Show)?;
    stdout.flush()?;
    Ok(())
}

fn run_ratatui_probe(options: &ProbeOptions) -> io::Result<()> {
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut state = ProbeState::new();
    render_ratatui(&mut terminal, options, &state)?;
    let mut next_frame = Instant::now() + options.tick_rate;

    loop {
        let wait = if options.needs_periodic_redraw() {
            next_frame.saturating_duration_since(Instant::now())
        } else {
            options.tick_rate
        };

        if event::poll(wait)? {
            let event = event::read()?;
            append_probe_event_with_path(&options.output_path, &event)?;
            state.record_event(&event);
            if let Event::Key(key) = event {
                if state.handle_key_event(key) {
                    return Ok(());
                }
            }
            render_ratatui(&mut terminal, options, &state)?;
        } else if options.needs_periodic_redraw() {
            state.tick_frame();
            next_frame = Instant::now() + options.tick_rate;
            render_ratatui(&mut terminal, options, &state)?;
        }
    }
}

fn render_ratatui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    options: &ProbeOptions,
    state: &ProbeState,
) -> io::Result<()> {
    terminal.draw(|frame| {
        let area = frame.area();
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Length(3),
                Constraint::Length(6),
                Constraint::Min(6),
            ])
            .split(area);

        let header = Paragraph::new(vec![
            Line::from("gwt-tui layered IME probe"),
            Line::from(format!(
                "Mode: {}  Tick: {}ms  Frames: {}  Events: {}",
                options.mode.as_str(),
                options.tick_rate.as_millis(),
                state.frame_count(),
                state.event_count()
            )),
            Line::from(format!("JSONL output: {}", options.output_path.display())),
            Line::from("Type with your IME. Ctrl+C twice exits."),
        ])
        .block(Block::default().borders(Borders::ALL).title("Probe"));
        frame.render_widget(header, sections[0]);

        let input = Paragraph::new(format!("{INPUT_PROMPT}{}", state.input_buffer()))
            .block(Block::default().borders(Borders::ALL).title("Committed"));
        frame.render_widget(input, sections[1]);

        let submitted = Paragraph::new(
            state
                .submitted_lines()
                .iter()
                .rev()
                .take(4)
                .rev()
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>(),
        )
        .block(Block::default().borders(Borders::ALL).title("Submitted"));
        frame.render_widget(submitted, sections[2]);

        let recent = Paragraph::new(
            state
                .recent_events()
                .iter()
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>(),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Recent events"),
        );
        frame.render_widget(recent, sections[3]);

        let max_x = sections[1].x + sections[1].width.saturating_sub(2);
        let cursor_x = (sections[1].x + 1 + state.cursor_column()).min(max_x);
        let cursor_y = sections[1].y + 1;
        frame.set_cursor_position((cursor_x, cursor_y));
    })?;
    Ok(())
}
