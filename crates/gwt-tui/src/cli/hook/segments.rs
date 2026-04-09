//! Shared Bash command segmentation used by every block hook.
//!
//! Translated 1:1 from the Node `splitCommandSegments` helper that
//! `.claude/hooks/scripts/gwt-block-*.mjs` shared. The goal is **not** to
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
    let mut s = command.to_string();

    // `|&` and `||` must be expanded before the generic `[;|&]` pass,
    // otherwise `||` would be split into two empty segments.
    s = s.replace("|&", "\n");
    s = s.replace("||", "\n");
    s = s.replace("&&", "\n");

    // Any of `;` `|` `&` by itself is a control operator in shell.
    s = s
        .chars()
        .map(|c| {
            if matches!(c, ';' | '|' | '&') {
                '\n'
            } else {
                c
            }
        })
        .collect();

    s.split('\n')
        .map(normalize_segment)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Drop everything from the first redirection operator onward, then trim.
///
/// The Node helper uses two passes (`[<>].*` and `<<.*`). The heredoc
/// pattern is covered by the first pass too, but we keep the two passes
/// separate to match the original behaviour on edge cases like `cat <<EOF`.
fn normalize_segment(s: &str) -> String {
    let s = match s.find(['<', '>']) {
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
    fn strips_redirection_tail() {
        let segments = split_command_segments("echo hi > out.log");
        assert_eq!(segments, vec!["echo hi"]);
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
