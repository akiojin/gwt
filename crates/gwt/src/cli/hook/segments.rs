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
/// strip simple redirections (`> file`, `<< EOF`, ...). Empty segments
/// are dropped.
pub fn split_command_segments(command: &str) -> Vec<String> {
    split_unquoted_control_operators(command)
        .into_iter()
        .map(normalize_segment)
        .filter(|s| !s.is_empty())
        .collect()
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
                b';' | b'|' | b'&' => {
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
}
