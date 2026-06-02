//! Model → context-window-size lookup (SPEC-2970 FR-018).
//!
//! Used to estimate per-session "context left" when the source does not embed
//! the window size. Codex rollouts carry `info.model_context_window` directly,
//! so this table is primarily for Claude transcripts. Unknown models return
//! `None`, which the UI renders as "tokens only, no remaining %".

/// Return the approximate context window size (in tokens) for a model name,
/// or `None` when the model is unknown. Matching is case-insensitive and
/// substring-based so versioned names (`claude-opus-4-7`) resolve correctly.
pub fn context_limit(model: &str) -> Option<u64> {
    let m = model.to_ascii_lowercase();

    // Explicit 1M-context beta variants take priority.
    if m.contains("[1m]") || m.contains("-1m") || m.contains("1m-context") {
        return Some(1_000_000);
    }

    // Anthropic Claude family: standard 200k context.
    if m.contains("claude") || m.contains("opus") || m.contains("sonnet") || m.contains("haiku") {
        return Some(200_000);
    }

    // OpenAI gpt-5 / codex family fallback. Codex normally provides the real
    // window via the rollout payload; this is only a defensive default.
    if m.contains("gpt-5") || m.contains("codex") {
        return Some(400_000);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn gpt5_codex_fallback() {
        assert_eq!(context_limit("gpt-5.5"), Some(400_000));
        assert_eq!(context_limit("gpt-5-codex"), Some(400_000));
    }

    #[test]
    fn unknown_model_is_none() {
        assert_eq!(context_limit("some-future-model"), None);
        assert_eq!(context_limit(""), None);
    }
}
