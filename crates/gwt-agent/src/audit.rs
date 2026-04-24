//! Secret-name classifier for the agent launch audit log (FR-047, FR-063).
//!
//! Agent launch diagnostics are emitted through the canonical structured
//! logging pipeline with resolved command, args, cwd, and environment. To
//! prevent plaintext API keys from ending up in the project-scoped
//! `gwt.log.YYYY-MM-DD`, env entries whose keys match a known secret pattern
//! are masked before the event is emitted.

/// Placeholder string that replaces a secret value in the audit record.
pub const REDACTED_PLACEHOLDER: &str = "***REDACTED***";

/// Exact env var names that are always treated as secrets (case-insensitive).
///
/// Maintained as a list rather than a single regex so the set is trivially
/// auditable and easy to extend when new provider SDKs appear.
const EXACT_SECRET_NAMES: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
];

/// Case-insensitive suffix patterns that classify an env var as a secret.
const SECRET_SUFFIXES: &[&str] = &["_API_KEY", "_TOKEN", "_SECRET"];

/// Returns `true` if the given env var key should be masked in audit logs.
///
/// Matching is case-insensitive. The following keys are considered secret:
///
/// - Exact match against [`EXACT_SECRET_NAMES`].
/// - Any key that ends with `_API_KEY`, `_TOKEN`, or `_SECRET`.
///
/// Non-secret keys such as `ANTHROPIC_BASE_URL`, `ANTHROPIC_DEFAULT_OPUS_MODEL`,
/// `CLAUDE_CODE_SUBAGENT_MODEL`, and `CLAUDE_CODE_ATTRIBUTION_HEADER` are
/// returned as-is.
pub fn is_secret_env_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    if EXACT_SECRET_NAMES.iter().any(|exact| upper == *exact) {
        return true;
    }
    SECRET_SUFFIXES.iter().any(|suffix| upper.ends_with(suffix))
}

/// Returns a redaction-safe representation of the given env value. Non-secret
/// keys return the original value unchanged; secret keys return
/// [`REDACTED_PLACEHOLDER`] regardless of input.
pub fn redact_env_value_for_audit<'a>(key: &str, value: &'a str) -> std::borrow::Cow<'a, str> {
    if is_secret_env_key(key) {
        std::borrow::Cow::Owned(REDACTED_PLACEHOLDER.to_string())
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

/// In-place mask every secret-shaped env entry on the given custom agent so
/// that a copy of it is safe to transmit over the WebSocket protocol without
/// leaking plaintext `ANTHROPIC_API_KEY` (or similar) to the frontend.
/// Callers should clone the `CustomCodingAgent` first when the original must
/// retain secrets for launch.
pub fn redact_secrets_in_agent(agent: &mut crate::custom::CustomCodingAgent) {
    for (key, value) in agent.env.iter_mut() {
        if is_secret_env_key(key) {
            *value = REDACTED_PLACEHOLDER.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_secret_names_are_masked() {
        assert!(is_secret_env_key("ANTHROPIC_API_KEY"));
        assert!(is_secret_env_key("OPENAI_API_KEY"));
        assert!(is_secret_env_key("GEMINI_API_KEY"));
        assert!(is_secret_env_key("GOOGLE_API_KEY"));
        assert!(is_secret_env_key("ANTHROPIC_AUTH_TOKEN"));
    }

    #[test]
    fn case_insensitive_matching() {
        assert!(is_secret_env_key("anthropic_api_key"));
        assert!(is_secret_env_key("Anthropic_Api_Key"));
        assert!(is_secret_env_key("openai_api_KEY"));
    }

    #[test]
    fn api_key_suffix_is_masked() {
        assert!(is_secret_env_key("MY_CUSTOM_API_KEY"));
        assert!(is_secret_env_key("AZURE_OPENAI_API_KEY"));
        assert!(is_secret_env_key("internal_service_api_key"));
    }

    #[test]
    fn token_suffix_is_masked() {
        assert!(is_secret_env_key("GITHUB_TOKEN"));
        assert!(is_secret_env_key("ANTHROPIC_AUTH_TOKEN"));
        assert!(is_secret_env_key("CIRCLE_CI_TOKEN"));
    }

    #[test]
    fn secret_suffix_is_masked() {
        assert!(is_secret_env_key("DATABASE_SECRET"));
        assert!(is_secret_env_key("SESSION_SECRET"));
        assert!(is_secret_env_key("jwt_secret"));
    }

    #[test]
    fn non_secret_preset_env_pass_through() {
        // These are the non-secret entries seeded by the
        // Claude Code (OpenAI-compat backend) preset (SPEC-1921 FR-062).
        let allowed = [
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_CODE_ATTRIBUTION_HEADER",
            "DISABLE_TELEMETRY",
            "CLAUDE_CODE_NO_FLICKER",
            "DISABLE_ERROR_REPORTING",
            "DISABLE_FEEDBACK_COMMAND",
            "CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
        ];
        for key in allowed {
            assert!(
                !is_secret_env_key(key),
                "expected {key} to NOT be classified as secret"
            );
        }
    }

    #[test]
    fn redact_env_value_masks_secrets() {
        assert_eq!(
            redact_env_value_for_audit("ANTHROPIC_API_KEY", "sk_real_secret_value"),
            REDACTED_PLACEHOLDER
        );
        assert_eq!(
            redact_env_value_for_audit("MY_CUSTOM_TOKEN", "ghp_abc123"),
            REDACTED_PLACEHOLDER
        );
    }

    #[test]
    fn redact_env_value_passes_through_non_secrets() {
        assert_eq!(
            redact_env_value_for_audit("ANTHROPIC_BASE_URL", "http://proxy.local:32768"),
            "http://proxy.local:32768"
        );
        assert_eq!(
            redact_env_value_for_audit("ANTHROPIC_DEFAULT_OPUS_MODEL", "openai/gpt-oss-20b"),
            "openai/gpt-oss-20b"
        );
    }

    #[test]
    fn empty_value_still_masked_for_secret_keys() {
        // Even if the value is empty, a secret key must be masked so that
        // upstream log readers cannot tell whether a secret was empty.
        assert_eq!(
            redact_env_value_for_audit("OPENAI_API_KEY", ""),
            REDACTED_PLACEHOLDER
        );
    }

    #[test]
    fn keys_without_secret_suffix_not_masked() {
        assert!(!is_secret_env_key("PATH"));
        assert!(!is_secret_env_key("HOME"));
        assert!(!is_secret_env_key("TERM"));
        assert!(!is_secret_env_key("GWT_PROJECT_ROOT"));
        assert!(!is_secret_env_key("CLAUDE_CODE_EFFORT_LEVEL"));
    }
}
