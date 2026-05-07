//! AI provider settings.

use serde::{Deserialize, Serialize};

use crate::locale::detect_user_locale;

fn default_endpoint() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_language() -> Option<String> {
    Some("auto".to_string())
}

fn default_summary_enabled() -> bool {
    true
}

/// AI provider configuration for OpenAI-compatible APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AISettings {
    /// API endpoint URL.
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    /// API key (optional for local LLMs).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Model name.
    #[serde(default)]
    pub model: String,
    /// Output language ("en" | "ja" | "auto").
    #[serde(default = "default_language")]
    pub language: Option<String>,
    /// Enable session summary generation.
    #[serde(default = "default_summary_enabled")]
    pub summary_enabled: bool,
}

impl Default for AISettings {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            api_key: None,
            model: String::new(),
            language: default_language(),
            summary_enabled: default_summary_enabled(),
        }
    }
}

impl AISettings {
    /// Check if the settings are valid for use (endpoint and model required).
    pub fn is_enabled(&self) -> bool {
        !self.endpoint.trim().is_empty() && !self.model.trim().is_empty()
    }

    /// Normalize language to a known value ("en", "ja", "auto").
    pub fn normalized_language(&self) -> String {
        match self
            .language
            .as_deref()
            .unwrap_or("en")
            .trim()
            .to_lowercase()
            .as_str()
        {
            "ja" => "ja".to_string(),
            "auto" => "auto".to_string(),
            _ => "en".to_string(),
        }
    }

    /// Resolve the language used for narrative outputs (Workspace summaries
    /// and Board posts). `auto` resolves against the OS locale; `ja` / `en`
    /// pass through; unknown values fall back to `en`.
    ///
    /// SPEC-1933 FR-009 / SC-004.
    pub fn effective_language(&self) -> &'static str {
        let detected = detect_user_locale();
        self.effective_language_with_locale(detected.as_deref())
    }

    /// Pure variant of [`Self::effective_language`] that takes an
    /// already-detected locale, for tests and reuse from contexts that hold
    /// the value.
    pub fn effective_language_with_locale(&self, detected: Option<&str>) -> &'static str {
        match self
            .language
            .as_deref()
            .unwrap_or("auto")
            .trim()
            .to_lowercase()
            .as_str()
        {
            "ja" => "ja",
            "en" => "en",
            "auto" => match detected {
                Some("ja") => "ja",
                Some("en") => "en",
                _ => "en",
            },
            _ => "en",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_endpoint_but_no_model() {
        let s = AISettings::default();
        assert!(!s.endpoint.is_empty());
        assert!(s.model.is_empty());
        assert!(!s.is_enabled());
    }

    #[test]
    fn enabled_when_endpoint_and_model_set() {
        let s = AISettings {
            endpoint: "http://localhost:11434/v1".to_string(),
            model: "llama3".to_string(),
            ..Default::default()
        };
        assert!(s.is_enabled());
    }

    #[test]
    fn disabled_when_model_empty() {
        let s = AISettings {
            endpoint: "http://localhost:11434/v1".to_string(),
            model: "  ".to_string(),
            ..Default::default()
        };
        assert!(!s.is_enabled());
    }

    #[test]
    fn language_normalizes_known_values() {
        let s = AISettings {
            language: Some("JA".to_string()),
            ..Default::default()
        };
        assert_eq!(s.normalized_language(), "ja");

        let s = AISettings {
            language: Some(" auto ".to_string()),
            ..Default::default()
        };
        assert_eq!(s.normalized_language(), "auto");

        let s = AISettings {
            language: Some("fr".to_string()),
            ..Default::default()
        };
        assert_eq!(s.normalized_language(), "en");

        let s = AISettings {
            language: None,
            ..Default::default()
        };
        assert_eq!(s.normalized_language(), "en");
    }

    #[test]
    fn default_language_is_auto() {
        let s = AISettings::default();
        assert_eq!(s.language.as_deref(), Some("auto"));
    }

    #[test]
    fn effective_language_auto_with_ja_locale_returns_ja() {
        let s = AISettings {
            language: Some("auto".to_string()),
            ..Default::default()
        };
        assert_eq!(s.effective_language_with_locale(Some("ja")), "ja");
    }

    #[test]
    fn effective_language_auto_with_en_locale_returns_en() {
        let s = AISettings {
            language: Some("auto".to_string()),
            ..Default::default()
        };
        assert_eq!(s.effective_language_with_locale(Some("en")), "en");
    }

    #[test]
    fn effective_language_auto_with_no_locale_falls_back_to_en() {
        let s = AISettings {
            language: Some("auto".to_string()),
            ..Default::default()
        };
        assert_eq!(s.effective_language_with_locale(None), "en");
    }

    #[test]
    fn effective_language_explicit_ja_returns_ja() {
        let s = AISettings {
            language: Some("ja".to_string()),
            ..Default::default()
        };
        assert_eq!(s.effective_language_with_locale(None), "ja");
        assert_eq!(s.effective_language_with_locale(Some("en")), "ja");
    }

    #[test]
    fn effective_language_explicit_en_returns_en() {
        let s = AISettings {
            language: Some("en".to_string()),
            ..Default::default()
        };
        assert_eq!(s.effective_language_with_locale(None), "en");
        assert_eq!(s.effective_language_with_locale(Some("ja")), "en");
    }

    #[test]
    fn effective_language_unknown_falls_back_to_en() {
        let s = AISettings {
            language: Some("zh".to_string()),
            ..Default::default()
        };
        assert_eq!(s.effective_language_with_locale(Some("ja")), "en");
    }

    #[test]
    fn effective_language_none_falls_back_to_en() {
        let s = AISettings {
            language: None,
            ..Default::default()
        };
        assert_eq!(s.effective_language_with_locale(None), "en");
    }

    #[test]
    fn roundtrip_toml() {
        let s = AISettings {
            endpoint: "https://api.example.com/v1".to_string(),
            api_key: Some("sk-test".to_string()),
            model: "gpt-4".to_string(),
            language: Some("ja".to_string()),
            summary_enabled: false,
        };
        let toml_str = toml::to_string_pretty(&s).unwrap();
        let loaded: AISettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.endpoint, s.endpoint);
        assert_eq!(loaded.api_key, s.api_key);
        assert_eq!(loaded.model, s.model);
        assert_eq!(loaded.language, s.language);
        assert_eq!(loaded.summary_enabled, s.summary_enabled);
    }
}
