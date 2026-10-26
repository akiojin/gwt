//! Emit a backend-derived terminal fixture for Issue #6420 browser comparisons.
//! Compile with the checkout's built vt100 and serde_json rlibs via rustc.

use serde_json::json;

fn main() {
    let raw = concat!(
        "\x1b[?25l\x1b[2J\x1b[HPM transcript\r\n",
        "\x1b[20;1HOLD STATUS 97 percent\r\n> /modelmodel\r\nPress Enter",
        "\x1b[20;1H\x1b[2KNEW STATUS 7 percent\r\n",
        "\x1b[2K> /model\r\n\x1b[2KPress Enter"
    );
    let continuation = "\x1b[2A\r\x1b[2KSTATUS complete\x1b[1B\r\x1b[2K> ";
    let (rows, cols) = (24, 80);
    let mut source = vt100::Parser::new(rows, cols, 5_000);
    source.process(raw.as_bytes());
    let snapshot = source.screen().snapshot_formatted(5_000);
    let expected_before = source.screen().contents();
    let cursor_before = source.screen().cursor_position();
    let mut replay = vt100::Parser::new(rows, cols, 5_000);
    replay.process(&snapshot);
    assert_eq!(replay.screen().contents(), expected_before);
    assert_eq!(replay.screen().cursor_position(), cursor_before);

    source.process(continuation.as_bytes());
    replay.process(continuation.as_bytes());
    assert_eq!(replay.screen().contents(), source.screen().contents());
    assert_eq!(
        replay.screen().cursor_position(),
        source.screen().cursor_position()
    );
    assert!(!expected_before.contains("OLD STATUS"));
    assert!(!expected_before.contains("/modelmodel"));
    assert!(!source.screen().contents().contains("NEW STATUS"));

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "fixture": "Issue #6420 cursor, erase, status and model redraw",
            "cols": cols,
            "rows": rows,
            "raw": raw,
            "snapshot": String::from_utf8(snapshot).expect("ASCII snapshot fixture"),
            "continuation": continuation,
            "expected_before": {
                "text": expected_before,
                "cursor": [cursor_before.0, cursor_before.1],
            },
            "expected_after": {
                "text": source.screen().contents(),
                "cursor": [source.screen().cursor_position().0, source.screen().cursor_position().1],
            },
        }))
        .expect("serialize fixture")
    );
}
