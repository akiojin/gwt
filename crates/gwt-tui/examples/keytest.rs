use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::fs::OpenOptions;
use std::io::Write;

fn main() {
    let mut log = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("/tmp/keytest.log")
        .unwrap();

    enable_raw_mode().unwrap();
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).unwrap();

    writeln!(log, "=== gwt-tui key test ===").unwrap();
    writeln!(log, "Ctrl+C twice to exit").unwrap();
    writeln!(log, "---").unwrap();

    print!("\x1b[2J\x1b[H");
    print!("gwt-tui key test\r\n");
    print!("Results logged to /tmp/keytest.log\r\n");
    print!("Try: Ctrl+G, then ], [, c, Tab, Ctrl+G\r\n");
    print!("Ctrl+C twice to exit\r\n");
    print!("---\r\n");
    stdout.flush().unwrap();

    let mut last_ctrl_c = false;
    let mut count = 0;

    loop {
        if event::poll(std::time::Duration::from_millis(200)).unwrap() {
            if let Event::Key(key) = event::read().unwrap() {
                count += 1;
                let line = format!(
                    "{:3}: code={:?}  mod={:?}  kind={:?}",
                    count, key.code, key.modifiers, key.kind
                );
                writeln!(log, "{}", line).unwrap();
                log.flush().unwrap();
                print!("{}\r\n", line);
                stdout.flush().unwrap();

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

    writeln!(log, "--- exit ---").unwrap();
    disable_raw_mode().unwrap();
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture).unwrap();
    println!("Log saved to /tmp/keytest.log");
    println!("Run: cat /tmp/keytest.log");
}
