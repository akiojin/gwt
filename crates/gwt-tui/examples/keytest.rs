use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gwt_tui::input_trace::append_probe_event_with_path;
use std::{
    env,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

const DEFAULT_LOG_PATH: &str = "/tmp/gwt-crossterm-events.jsonl";

fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
}

fn output_path() -> PathBuf {
    env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_LOG_PATH))
}

fn reset_output_file(path: &Path) {
    let _ = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path);
}

fn main() {
    let output_path = output_path();
    reset_output_file(&output_path);

    enable_raw_mode().unwrap();
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).unwrap();
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
    );

    print!("\x1b[2J\x1b[H");
    print!("gwt-tui raw crossterm probe\r\n");
    print!("JSONL output: {}\r\n", output_path.display());
    print!("Try your IME candidate keys in this screen.\r\n");
    print!("Ctrl+C twice to exit\r\n");
    print!("---\r\n");
    stdout.flush().unwrap();

    let mut last_ctrl_c = false;
    let mut count = 0;

    loop {
        if event::poll(std::time::Duration::from_millis(200)).unwrap() {
            let event = event::read().unwrap();
            count += 1;
            append_probe_event_with_path(&output_path, &event).unwrap();

            let line = format!("{count:3}: {event:?}");
            print!("{}\r\n", line);
            stdout.flush().unwrap();

            if let Event::Key(key) = event {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    if last_ctrl_c {
                        break;
                    }
                    last_ctrl_c = true;
                } else {
                    last_ctrl_c = false;
                }
            }
        }
    }

    let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    disable_raw_mode().unwrap();
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture).unwrap();
    println!("JSONL saved to {}", output_path.display());
}
