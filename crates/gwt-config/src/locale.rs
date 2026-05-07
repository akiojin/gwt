//! OS locale detection for narrative output language resolution.
//!
//! See SPEC-1933 (Phase: System Settings — Output Language) FR-009.
//! Used by `AISettings::effective_language()` to resolve `auto` against
//! the user's OS locale.

/// Detect the user's preferred narrative language from common locale
/// environment variables.
///
/// Returns `Some("ja")` for ja_JP / ja-JP / ja variants, `Some("en")`
/// for any other detected non-`C` / non-`POSIX` locale, and `None`
/// when the locale cannot be determined.
pub fn detect_user_locale() -> Option<String> {
    detect_user_locale_from(read_env_locale().as_deref())
}

fn read_env_locale() -> Option<String> {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// Pure: parse a raw locale string into a `"ja"` / `"en"` / `None`
/// decision. Exposed for testing and for callers that already hold a
/// detected locale string.
pub fn detect_user_locale_from(value: Option<&str>) -> Option<String> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    if lower == "c" || lower == "posix" || lower.starts_with("c.") {
        return None;
    }
    let head = lower.split(['_', '-', '.', '@']).next()?;
    if head == "ja" {
        return Some("ja".to_string());
    }
    Some("en".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ja_jp_utf8_resolves_to_ja() {
        assert_eq!(
            detect_user_locale_from(Some("ja_JP.UTF-8")).as_deref(),
            Some("ja")
        );
    }

    #[test]
    fn ja_dash_jp_resolves_to_ja() {
        assert_eq!(
            detect_user_locale_from(Some("ja-JP")).as_deref(),
            Some("ja")
        );
    }

    #[test]
    fn bare_ja_resolves_to_ja() {
        assert_eq!(detect_user_locale_from(Some("ja")).as_deref(), Some("ja"));
    }

    #[test]
    fn en_us_resolves_to_en() {
        assert_eq!(
            detect_user_locale_from(Some("en_US.UTF-8")).as_deref(),
            Some("en")
        );
    }

    #[test]
    fn c_locale_returns_none() {
        assert_eq!(detect_user_locale_from(Some("C")).as_deref(), None);
        assert_eq!(detect_user_locale_from(Some("c")).as_deref(), None);
        assert_eq!(detect_user_locale_from(Some("C.UTF-8")).as_deref(), None);
    }

    #[test]
    fn posix_locale_returns_none() {
        assert_eq!(detect_user_locale_from(Some("POSIX")).as_deref(), None);
        assert_eq!(detect_user_locale_from(Some("posix")).as_deref(), None);
    }

    #[test]
    fn empty_or_none_returns_none() {
        assert_eq!(detect_user_locale_from(Some("")).as_deref(), None);
        assert_eq!(detect_user_locale_from(Some("   ")).as_deref(), None);
        assert_eq!(detect_user_locale_from(None).as_deref(), None);
    }

    #[test]
    fn other_languages_resolve_to_en() {
        assert_eq!(
            detect_user_locale_from(Some("zh_CN.UTF-8")).as_deref(),
            Some("en")
        );
        assert_eq!(
            detect_user_locale_from(Some("fr_FR")).as_deref(),
            Some("en")
        );
        assert_eq!(
            detect_user_locale_from(Some("de-DE")).as_deref(),
            Some("en")
        );
    }

    #[test]
    fn ja_with_modifier_resolves_to_ja() {
        assert_eq!(
            detect_user_locale_from(Some("ja_JP@cjknarrow")).as_deref(),
            Some("ja")
        );
    }
}
