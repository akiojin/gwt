//! Shared Bash command segmentation used by every block hook.
//!
//! Translated 1:1 from the legacy Node `splitCommandSegments` helper shared
//! by the retired block hooks. The goal is **not** to
//! be a general-purpose shell parser — only to approximate shell control
//! operators well enough that an adversarial command like
//! `echo hello && git rebase -i origin/main` is not allowed to hide a
//! blocked segment after an innocuous prefix.
//!
//! The sequence of transformations is order-sensitive; do not reorder
//! without re-running `hook_block_*_test` suites.

/// Split a raw command string on unquoted shell control operators and
/// strip simple redirections (`> file`, `<< EOF`, ...). Heredoc bodies are
/// masked out first — they are data, not command structure, so operators
/// inside them must not produce segments (issue #3265). Comment segments and
/// empty segments are dropped.
pub fn split_command_segments(command: &str) -> Vec<String> {
    let masked = mask_heredoc_bodies(command);
    split_unquoted_control_operators(&masked)
        .into_iter()
        .map(normalize_segment)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect()
}

/// Remove heredoc bodies — from the line after an unquoted `<<DELIM` up to
/// and including the terminator line — so later lexical passes never treat
/// heredoc payloads as command structure (issue #3265). Here-strings (`<<<`)
/// are not heredocs. Feeding an interpreter via heredoc (`bash <<EOF`) hides
/// the script body, but that is equivalent to `bash -c '<script>'`, which
/// segmentation never classified either.
///
/// An unterminated heredoc ends the masking pass: bash would swallow the rest
/// of the input as that body, so nothing after it can be classified anyway,
/// and leaving the raw text in place keeps it visible to fail-closed checks.
/// Stopping there also keeps this a single forward pass — hooks run on every
/// Bash tool call and must stay linear in command length.
pub fn mask_heredoc_bodies(command: &str) -> String {
    let bytes = command.as_bytes();
    let mut masked = String::with_capacity(command.len());
    let mut pending: Vec<HeredocMarker> = Vec::new();
    // Everything before `copied` has been written to `masked`.
    let mut copied = 0;
    let mut i = 0;
    let mut quote = Quote::None;
    let mut escaped = false;

    while i < bytes.len() {
        let b = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        match quote {
            Quote::Single => {
                if b == b'\'' {
                    quote = Quote::None;
                }
            }
            Quote::Double => match b {
                b'\\' => escaped = true,
                b'"' => quote = Quote::None,
                _ => {}
            },
            Quote::None => match b {
                b'\\' => escaped = true,
                b'\'' => quote = Quote::Single,
                b'"' => quote = Quote::Double,
                b'<' if bytes.get(i + 1) == Some(&b'<')
                    && bytes.get(i + 2) != Some(&b'<')
                    && (i == 0 || bytes[i - 1] != b'<') =>
                {
                    match parse_heredoc_delimiter(command, i + 2) {
                        Some(marker) => {
                            i = marker.delimiter_end;
                            pending.push(marker);
                        }
                        None => i += 2,
                    }
                    continue;
                }
                // Bodies for every marker collected on this line follow the
                // newline, in marker order.
                b'\n' if !pending.is_empty() => {
                    masked.push_str(&command[copied..=i]);
                    let mut body_start = i + 1;
                    let mut unterminated = false;
                    for marker in pending.drain(..) {
                        match heredoc_terminator_end(
                            command,
                            body_start,
                            &marker.delimiter,
                            marker.strip_tabs,
                        ) {
                            Some(end) => body_start = end,
                            None => {
                                unterminated = true;
                                break;
                            }
                        }
                    }
                    copied = body_start;
                    i = body_start;
                    if unterminated {
                        break;
                    }
                    continue;
                }
                _ => {}
            },
        }
        i += 1;
    }

    masked.push_str(&command[copied..]);
    masked
}

struct HeredocMarker {
    delimiter_end: usize,
    delimiter: String,
    strip_tabs: bool,
}

fn parse_heredoc_delimiter(command: &str, start: usize) -> Option<HeredocMarker> {
    let bytes = command.as_bytes();
    let mut j = start;
    let strip_tabs = bytes.get(j) == Some(&b'-');
    if strip_tabs {
        j += 1;
    }
    while matches!(bytes.get(j), Some(b' ' | b'\t')) {
        j += 1;
    }
    match bytes.get(j) {
        Some(&quote @ (b'\'' | b'"')) => {
            j += 1;
            let word_start = j;
            while j < bytes.len() && bytes[j] != quote && bytes[j] != b'\n' {
                j += 1;
            }
            (bytes.get(j) == Some(&quote) && j > word_start).then(|| HeredocMarker {
                delimiter_end: j + 1,
                delimiter: command[word_start..j].to_string(),
                strip_tabs,
            })
        }
        _ => {
            if bytes.get(j) == Some(&b'\\') {
                j += 1;
            }
            let word_start = j;
            while j < bytes.len()
                && !matches!(
                    bytes[j],
                    b' ' | b'\t'
                        | b'\n'
                        | b';'
                        | b'|'
                        | b'&'
                        | b'<'
                        | b'>'
                        | b'('
                        | b')'
                        | b'\''
                        | b'"'
                )
            {
                j += 1;
            }
            (j > word_start).then(|| HeredocMarker {
                delimiter_end: j,
                delimiter: command[word_start..j].to_string(),
                strip_tabs,
            })
        }
    }
}

/// Byte index just past the heredoc terminator line (including its trailing
/// newline when present), or `None` when the body never terminates.
fn heredoc_terminator_end(
    command: &str,
    body_start: usize,
    delimiter: &str,
    strip_tabs: bool,
) -> Option<usize> {
    let mut line_start = body_start;
    while line_start <= command.len() {
        let line_end = command[line_start..]
            .find('\n')
            .map_or(command.len(), |offset| line_start + offset);
        let line = &command[line_start..line_end];
        let line = if strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line
        };
        if line == delimiter {
            return Some((line_end + 1).min(command.len()));
        }
        if line_end == command.len() {
            return None;
        }
        line_start = line_end + 1;
    }
    None
}

/// Stand-in target recorded when the command cannot be lexed with confidence
/// (an unterminated quote desynchronizes the scanner, so a later `>` may be
/// invisible). It never names a real path, so consumers that ask "does this
/// command write a file" and "is every target bookkeeping" both fail closed.
pub const UNRESOLVED_REDIRECT_TARGET: &str = "<unresolved redirect>";

/// Unquoted output-redirect file targets outside heredoc bodies. Harmless
/// sinks — fd duplication (`2>&1`, `>&2`, `>&-`) and `/dev/null` — are not
/// returned. A `>` inside a string literal or heredoc payload is data, not
/// a redirection (issue #3265).
pub fn output_redirect_file_targets(command: &str) -> Vec<String> {
    let masked = mask_heredoc_bodies(command);
    let bytes = masked.as_bytes();
    let mut targets = Vec::new();
    let mut i = 0;
    let mut quote = Quote::None;
    let mut escaped = false;

    while i < bytes.len() {
        let b = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        match quote {
            Quote::Single => {
                if b == b'\'' {
                    quote = Quote::None;
                }
            }
            Quote::Double => match b {
                b'\\' => escaped = true,
                b'"' => quote = Quote::None,
                _ => {}
            },
            Quote::None => match b {
                b'\\' => escaped = true,
                b'\'' => quote = Quote::Single,
                b'"' => quote = Quote::Double,
                b'>' => {
                    let mut j = i + 1;
                    if bytes.get(j) == Some(&b'>') {
                        j += 1;
                    }
                    if bytes.get(j) == Some(&b'|') {
                        j += 1;
                    }
                    if let Some(after_dup) = fd_duplication_end(bytes, j) {
                        i = after_dup;
                        continue;
                    }
                    if bytes.get(j) == Some(&b'&') {
                        // Not a descriptor duplication, so `>&word` is the
                        // stdout+stderr form: the file target follows the `&`.
                        j += 1;
                    }
                    while matches!(bytes.get(j), Some(b' ' | b'\t')) {
                        j += 1;
                    }
                    match read_redirect_target(&masked, j) {
                        Some((target, next)) => {
                            if !target.is_empty() && target != "/dev/null" {
                                targets.push(target);
                            }
                            i = next.max(i + 1);
                            continue;
                        }
                        None => {
                            targets.push(UNRESOLVED_REDIRECT_TARGET.to_string());
                            return targets;
                        }
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    if quote != Quote::None {
        targets.push(UNRESOLVED_REDIRECT_TARGET.to_string());
    }
    targets
}

/// Index just past an `>&N` / `>&-` fd duplication starting at `start`, or
/// `None` when this is an ordinary redirect. Bash duplicates only when the
/// word after `&` is entirely digits (or exactly `-`); `>&2foo` writes the
/// file `2foo`, so the whole word must be checked, not just its prefix.
fn fd_duplication_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'&') {
        return None;
    }
    let word_start = start + 1;
    let mut j = word_start;
    if bytes.get(j) == Some(&b'-') {
        j += 1;
    } else {
        while bytes.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
    }
    let ends_word = bytes.get(j).is_none_or(|b| {
        matches!(
            b,
            b' ' | b'\t' | b'\n' | b';' | b'|' | b'&' | b'<' | b'>' | b'(' | b')'
        )
    });
    (j > word_start && ends_word).then_some(j)
}

/// The redirect target word starting at `start`, plus the index just past it.
/// `None` when a quoted target never closes — the command cannot be lexed.
fn read_redirect_target(command: &str, start: usize) -> Option<(String, usize)> {
    let bytes = command.as_bytes();
    match bytes.get(start) {
        Some(&quote @ (b'\'' | b'"')) => {
            let mut j = start + 1;
            let mut escaped = false;
            while j < bytes.len() {
                match bytes[j] {
                    _ if escaped => escaped = false,
                    // Inside single quotes bash takes a backslash literally.
                    b'\\' if quote == b'"' => escaped = true,
                    b if b == quote => break,
                    _ => {}
                }
                j += 1;
            }
            (j < bytes.len()).then(|| (command[start + 1..j].to_string(), j + 1))
        }
        _ => {
            let mut j = start;
            while j < bytes.len()
                && !matches!(
                    bytes[j],
                    b' ' | b'\t' | b'\n' | b';' | b'|' | b'&' | b'<' | b'>'
                )
            {
                j += 1;
            }
            Some((command[start..j].to_string(), j))
        }
    }
}

fn split_unquoted_control_operators(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut quote = Quote::None;
    let mut escaped = false;

    while i < bytes.len() {
        let b = bytes[i];

        if escaped {
            escaped = false;
            i += 1;
            continue;
        }

        match quote {
            Quote::Single => {
                if b == b'\'' {
                    quote = Quote::None;
                }
                i += 1;
                continue;
            }
            Quote::Double => {
                match b {
                    b'\\' => escaped = true,
                    b'"' => quote = Quote::None,
                    _ => {}
                }
                i += 1;
                continue;
            }
            Quote::None => match b {
                b'\\' => {
                    escaped = true;
                    i += 1;
                    continue;
                }
                b'\'' => {
                    quote = Quote::Single;
                    i += 1;
                    continue;
                }
                b'"' => {
                    quote = Quote::Double;
                    i += 1;
                    continue;
                }
                b'&' if bytes.get(i + 1) == Some(&b'&') => {
                    segments.push(&command[start..i]);
                    i += 2;
                    start = i;
                    continue;
                }
                b'|' if matches!(bytes.get(i + 1), Some(b'|' | b'&')) => {
                    segments.push(&command[start..i]);
                    i += 2;
                    start = i;
                    continue;
                }
                // `>&` is a redirection (`2>&1`), not a control operator; keep
                // it inside the segment so the redirection pass strips it.
                b'&' if i > 0 && bytes[i - 1] == b'>' => {}
                // A newline separates commands like `;`. This is only safe
                // because heredoc bodies are masked before splitting; an
                // escaped newline (line continuation) stays in the segment.
                b';' | b'|' | b'&' | b'\n' => {
                    segments.push(&command[start..i]);
                    i += 1;
                    start = i;
                    continue;
                }
                _ => {}
            },
        }

        i += 1;
    }

    segments.push(&command[start..]);
    segments
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

/// Drop everything from the first redirection operator onward, then trim.
///
/// The Node helper uses two passes (`[<>].*` and `<<.*`). The heredoc
/// pattern is covered by the first pass too, but we keep the two passes
/// separate to match the original behaviour on edge cases like `cat <<EOF`.
fn normalize_segment(s: &str) -> String {
    let s = match first_unquoted_redirection(s) {
        Some(idx) => &s[..idx],
        None => s,
    };
    // Heredoc already stripped above, but run a defensive second pass
    // against `<<` just like the Node code so that any future refactor
    // does not silently change behaviour.
    let s = match s.find("<<") {
        Some(idx) => &s[..idx],
        None => s,
    };
    s.trim().to_string()
}

fn first_unquoted_redirection(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut quote = Quote::None;
    let mut escaped = false;

    while i < bytes.len() {
        let b = bytes[i];

        if escaped {
            escaped = false;
            i += 1;
            continue;
        }

        match quote {
            Quote::Single => {
                if b == b'\'' {
                    quote = Quote::None;
                }
            }
            Quote::Double => match b {
                b'\\' => escaped = true,
                b'"' => quote = Quote::None,
                _ => {}
            },
            Quote::None => match b {
                b'\\' => escaped = true,
                b'\'' => quote = Quote::Single,
                b'"' => quote = Quote::Double,
                b'<' | b'>' => return Some(i),
                _ => {}
            },
        }

        i += 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_and_operator() {
        let segments = split_command_segments("echo hi && git status");
        assert_eq!(segments, vec!["echo hi", "git status"]);
    }

    #[test]
    fn splits_on_multiple_operators() {
        let segments = split_command_segments("a ; b || c && d | e");
        assert_eq!(segments, vec!["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn does_not_split_control_operators_inside_quotes() {
        let segments =
            split_command_segments(r#"rg -n "gh pr checks|gh run view|gh api graphql" .codex"#);
        assert_eq!(
            segments,
            vec![r#"rg -n "gh pr checks|gh run view|gh api graphql" .codex"#]
        );
    }

    #[test]
    fn strips_redirection_tail() {
        let segments = split_command_segments("echo hi > out.log");
        assert_eq!(segments, vec!["echo hi"]);
    }

    #[test]
    fn keeps_redirection_like_text_inside_quotes() {
        let segments = split_command_segments(r#"grep "a>b" file.txt > out.log"#);
        assert_eq!(segments, vec![r#"grep "a>b" file.txt"#]);
    }

    #[test]
    fn strips_heredoc_tail() {
        let segments = split_command_segments("cat <<EOF\nhello\nEOF");
        assert!(segments.iter().any(|s| s == "cat"));
    }

    #[test]
    fn empty_input_yields_empty_vec() {
        assert!(split_command_segments("").is_empty());
    }

    #[test]
    fn adversarial_prefix_does_not_hide_blocked_segment() {
        // Regression guard: `echo hello && git rebase -i origin/main`
        // must surface the rebase segment so the block hook can see it.
        let segments = split_command_segments("echo hello && git rebase -i origin/main");
        assert!(segments.iter().any(|s| s == "git rebase -i origin/main"));
    }

    // ---- issue #3265: heredoc bodies are data, newlines are separators ----

    #[test]
    fn masks_terminated_heredoc_body() {
        let masked = mask_heredoc_bodies("gwtd <<'JSON'\n{\"body\":\"a; b | c > d << e\"}\nJSON");
        assert_eq!(masked, "gwtd <<'JSON'\n");
    }

    #[test]
    fn masks_unquoted_and_tab_stripped_delimiters() {
        assert_eq!(
            mask_heredoc_bodies("cat <<EOF\nhello\nEOF\necho done"),
            "cat <<EOF\necho done"
        );
        assert_eq!(
            mask_heredoc_bodies("cat <<-EOF\n\thello\n\tEOF"),
            "cat <<-EOF\n"
        );
    }

    #[test]
    fn leaves_unterminated_heredoc_untouched() {
        let command = "gwtd <<'JSON'\n{\"a\":1}\nJSON > out.json";
        assert_eq!(mask_heredoc_bodies(command), command);
    }

    #[test]
    fn masks_multiple_heredocs_in_sequence() {
        let masked = mask_heredoc_bodies("cat <<A <<B\nbody a\nA\nbody b\nB");
        assert_eq!(masked, "cat <<A <<B\n");
    }

    #[test]
    fn here_strings_and_quoted_markers_are_not_heredocs() {
        assert_eq!(
            mask_heredoc_bodies("grep x <<< 'a; b'"),
            "grep x <<< 'a; b'"
        );
        assert_eq!(
            mask_heredoc_bodies("echo \"<<EOF\"\nEOF"),
            "echo \"<<EOF\"\nEOF"
        );
    }

    #[test]
    fn does_not_split_on_operators_inside_heredoc_body() {
        let segments = split_command_segments(
            "gwtd <<'EOF'\nline one; rm -rf / && echo hidden\nEOF\n&& cargo test",
        );
        assert_eq!(segments, vec!["gwtd", "cargo test"]);
    }

    #[test]
    fn splits_on_newline_between_commands() {
        let segments = split_command_segments("echo hi\ngit status");
        assert_eq!(segments, vec!["echo hi", "git status"]);
    }

    #[test]
    fn does_not_split_on_escaped_or_quoted_newlines() {
        assert_eq!(
            split_command_segments("git log \\\n--oneline").len(),
            1,
            "line continuation stays one segment"
        );
        assert_eq!(
            split_command_segments("git commit -m \"line1\nline2\"").len(),
            1,
            "quoted newline stays one segment"
        );
    }

    #[test]
    fn drops_comment_segments() {
        assert_eq!(
            split_command_segments("# stage the envelope\njq -n '{}' | gwtd"),
            vec!["jq -n '{}'", "gwtd"]
        );
        assert!(
            split_command_segments("echo x # && git rebase -i main")
                .iter()
                .any(|s| s == "git rebase -i main"),
            "a segment after a control operator is classified, not skipped as comment text"
        );
    }

    #[test]
    fn output_redirect_targets_are_quote_and_heredoc_aware() {
        assert_eq!(
            output_redirect_file_targets("git log --oneline > log.txt 2>&1"),
            vec!["log.txt"]
        );
        assert_eq!(output_redirect_file_targets("echo a>b"), vec!["b"]);
        assert_eq!(
            output_redirect_file_targets("cmd >> out.log"),
            vec!["out.log"]
        );
        assert_eq!(
            output_redirect_file_targets("cmd > 'out file.txt'"),
            vec!["out file.txt"]
        );
        assert_eq!(
            output_redirect_file_targets("cmd 2> err.log"),
            vec!["err.log"]
        );
        assert_eq!(
            output_redirect_file_targets("cmd &> all.log"),
            vec!["all.log"]
        );
        assert!(output_redirect_file_targets("ls 2>/dev/null").is_empty());
        assert!(output_redirect_file_targets("git log 2>&1 | head -3").is_empty());
        assert!(output_redirect_file_targets("cmd >&2").is_empty());
        assert!(output_redirect_file_targets("cmd >&-").is_empty());
        assert!(output_redirect_file_targets("grep \"a>b\" file.txt").is_empty());
        assert!(
            output_redirect_file_targets("gwtd <<'J'\n{\"snippet\":\"cat x > y\"}\nJ").is_empty()
        );
    }

    // `>&word` duplicates a descriptor only when the word is all digits or
    // exactly `-`; anything else is a stdout+stderr redirect to that file.
    #[test]
    fn fd_duplication_does_not_swallow_digit_prefixed_filenames() {
        assert_eq!(
            output_redirect_file_targets("echo pwn >&2src.txt"),
            vec!["2src.txt"]
        );
        assert_eq!(
            output_redirect_file_targets("echo pwn >&-src"),
            vec!["-src"]
        );
        assert_eq!(
            output_redirect_file_targets("echo pwn >&out.txt"),
            vec!["out.txt"]
        );
    }

    // Lexer desync must fail closed: an unterminated quote can hide a later
    // `>` entirely, so the command is reported as writing an unknown file.
    #[test]
    fn unresolvable_quoting_reports_an_unresolved_redirect() {
        for command in [
            r#"echo $'a\'b' > src/generated.rs"#,
            r#"echo x > "unterminated"#,
        ] {
            assert_eq!(
                output_redirect_file_targets(command),
                vec![UNRESOLVED_REDIRECT_TARGET.to_string()],
                "{command}"
            );
        }
    }

    #[test]
    fn escaped_quote_inside_a_double_quoted_target_does_not_hide_later_redirects() {
        assert_eq!(
            output_redirect_file_targets(r#"echo x > ".gwt/a\"b" ; echo evil > src/main.rs"#),
            vec![".gwt/a\\\"b".to_string(), "src/main.rs".to_string()]
        );
    }

    // Hooks run on every Bash tool call: masking must stay linear. Before the
    // single-pass rewrite this input took seconds.
    #[test]
    fn masking_stays_linear_on_large_heredoc_marker_spam() {
        let command = "x <<a\n".repeat(20_000);
        let start = std::time::Instant::now();
        let masked = mask_heredoc_bodies(&command);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "masking 120KB took {:?}",
            start.elapsed()
        );
        assert!(!masked.is_empty());
    }
}
