//! Model → context-window-size lookup (SPEC-2970 FR-018, Issue #3986).
//!
//! Used to estimate per-session "context left" when the source does not embed
//! the window size. Codex rollouts normally carry `info.model_context_window`
//! directly, so this table serves Claude transcripts first and acts as the
//! fallback for Codex rollouts that omit the window. Unknown models return
//! `None`, which the UI renders as "tokens only, no remaining %".

/// Known model families and their approximate context window (tokens).
///
/// Each entry is matched on a name boundary — the model id equals the family
/// or starts with it followed by a separator — never as a free substring, so a
/// new generation (`gpt-6`, `gpt-7`, ...) cannot be swallowed by an older
/// entry. The launch wizard catalog test
/// (`launch_wizard::options::tests::every_catalog_model_resolves_a_context_limit`)
/// walks every selectable model against this table, so adding a catalog row
/// without a family entry fails that test instead of silently returning
/// `None`.
const KNOWN_FAMILIES: &[(&str, u64)] = &[
    // Specific model windows precede the standard Claude family estimate.
    ("claude-fable-5", 1_000_000),
    ("claude-mythos-5", 1_000_000),
    // Bare tier aliases are what the launch wizard passes as `--model`.
    ("claude", 200_000),
    ("fable", 1_000_000),
    ("opus", 200_000),
    ("sonnet", 200_000),
    ("haiku", 200_000),
    // OpenAI generations from the Codex picker catalog. Codex reports the
    // real window in its rollout; these are defensive estimates only.
    ("gpt-6", 400_000),
    ("gpt-5", 400_000),
    ("codex", 400_000),
];

/// Return the approximate context window size (in tokens) for a model name,
/// or `None` when the model is unknown. Matching is case-insensitive and
/// family-based (see `KNOWN_FAMILIES`) so versioned names such as
/// `claude-opus-4-7` or `gpt-6-astra` resolve to their family's window.
pub fn context_limit(model: &str) -> Option<u64> {
    let m = model.to_ascii_lowercase();

    // Explicit 1M-context beta variants take priority over the family table.
    if m.contains("[1m]") || m.contains("-1m") || m.contains("1m-context") {
        return Some(1_000_000);
    }

    let resolved = KNOWN_FAMILIES
        .iter()
        .find(|(family, _)| matches_family(&m, family))
        .map(|(_, window)| *window);

    if resolved.is_none() && !m.is_empty() {
        tracing::debug!(
            model,
            "context window unknown for model; context-left estimate unavailable"
        );
    }
    resolved
}

/// `model` names `family` exactly, or starts with it followed by a separator.
fn matches_family(model: &str, family: &str) -> bool {
    match model.strip_prefix(family) {
        Some("") => true,
        Some(rest) => rest.starts_with(['-', '.', '_', ':', '@', '[', ' ']),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fable_and_mythos_use_one_million_context() {
        assert_eq!(context_limit("claude-fable-5"), Some(1_000_000));
        assert_eq!(context_limit("CLAUDE-MYTHOS-5"), Some(1_000_000));
    }

    #[test]
    fn known_claude_models_resolve_to_200k() {
        assert_eq!(context_limit("claude-opus-4-7"), Some(200_000));
        assert_eq!(context_limit("claude-sonnet-4-6"), Some(200_000));
        assert_eq!(context_limit("Claude-Haiku-4-5"), Some(200_000));
    }

    #[test]
    fn one_million_variant_detected() {
        assert_eq!(context_limit("claude-opus-4-8[1m]"), Some(1_000_000));
        assert_eq!(context_limit("claude-sonnet-1m-context"), Some(1_000_000));
    }

    // Issue #3986 AC-3: the pre-existing gpt-5 / codex answers are unchanged.
    #[test]
    fn gpt5_codex_fallback() {
        assert_eq!(context_limit("gpt-5.5"), Some(400_000));
        assert_eq!(context_limit("gpt-5-codex"), Some(400_000));
        assert_eq!(context_limit("gpt-5.3-codex-spark"), Some(400_000));
        assert_eq!(context_limit("codex-mini-latest"), Some(400_000));
    }

    // Issue #3986 AC-1: the gpt-6 generation heads the Codex catalog and must
    // resolve instead of falling through the old `gpt-5` substring check.
    #[test]
    fn gpt6_generation_resolves() {
        assert_eq!(context_limit("gpt-6-astra"), Some(400_000));
        assert_eq!(context_limit("GPT-6-Astra"), Some(400_000));
    }

    // Issue #3986 AC-2: a family entry matches on a name boundary (exact, or
    // followed by a separator), never as a free substring of another name.
    #[test]
    fn family_match_requires_name_boundary() {
        assert_eq!(context_limit("gpt-5"), Some(400_000));
        assert_eq!(context_limit("gpt-50"), None);
        assert_eq!(context_limit("mygpt-5"), None);
        assert_eq!(context_limit("claudette-1"), None);
    }

    // Issue #3986 AC-4: unknown names are an explicit `None` (rendered as
    // "tokens only"), never a guessed window.
    #[test]
    fn unknown_model_is_none() {
        assert_eq!(context_limit("some-future-model"), None);
        assert_eq!(context_limit(""), None);
    }
}
