//! Antigravity CLI (`agy`) model selection (Issue #7778).
//!
//! `agy --model` takes the label `agy models` prints, not a slug: a slug such
//! as `gemini-3-pro` is silently replaced by the CLI's default model. The
//! reasoning strength is part of that label (`Gemini 3.1 Pro (High)`), so gwt
//! stores the model's base label and the reasoning step separately and joins
//! them only when building the launch argument.

/// One model row of the Antigravity picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AntigravityModel {
    /// The `agy models` label without its reasoning suffix.
    pub label: &'static str,
    pub description: &'static str,
    /// Reasoning steps the model's labels carry, lowest first, as the common
    /// stored values (`low` / `medium` / `high`). Empty when the label has no
    /// reasoning suffix.
    pub efforts: &'static [&'static str],
    /// The step used when none is selected.
    pub default_effort: Option<&'static str>,
}

/// Snapshot of the labels `agy models` prints (agy 1.0.5+).
pub const ANTIGRAVITY_MODELS: &[AntigravityModel] = &[
    AntigravityModel {
        label: "Gemini 3.1 Pro",
        description: "Google's most capable Gemini model",
        efforts: &["low", "high"],
        default_effort: Some("high"),
    },
    AntigravityModel {
        label: "Gemini 3.5 Flash",
        description: "Fast Gemini model for everyday tasks",
        efforts: &["low", "medium", "high"],
        default_effort: Some("medium"),
    },
    AntigravityModel {
        label: "Claude Sonnet 4.6",
        description: "Anthropic Claude Sonnet",
        efforts: &[],
        default_effort: None,
    },
    AntigravityModel {
        label: "Claude Opus 4.6",
        description: "Anthropic Claude Opus",
        efforts: &[],
        default_effort: None,
    },
    AntigravityModel {
        label: "GPT-OSS 120B",
        description: "OpenAI open-weight model",
        efforts: &[],
        default_effort: None,
    },
];

/// The first agy release with `agy models` and `--model`.
pub const ANTIGRAVITY_MODEL_FLAG_MIN_VERSION: (u64, u64, u64) = (1, 0, 5);

pub fn antigravity_model(label: &str) -> Option<&'static AntigravityModel> {
    ANTIGRAVITY_MODELS.iter().find(|model| model.label == label)
}

fn effort_label(effort: &str) -> Option<&'static str> {
    match effort {
        "low" => Some("Low"),
        "medium" => Some("Medium"),
        "high" => Some("High"),
        _ => None,
    }
}

/// The value passed to `agy --model`: the catalog label joined with the
/// selected reasoning step (or the model's default step when the selection is
/// not one the model offers). Labels outside the catalog pass unchanged.
pub fn antigravity_model_arg(model: &str, reasoning: Option<&str>) -> String {
    let model = model.trim();
    let Some(row) = antigravity_model(model) else {
        return model.to_string();
    };
    let effort = reasoning
        .filter(|effort| row.efforts.contains(effort))
        .or(row.default_effort);
    match effort.and_then(effort_label) {
        Some(label) => format!("{model} ({label})"),
        None => model.to_string(),
    }
}

/// Why an Antigravity launch with `model` must not start, if it must not.
///
/// A slug-shaped value (no whitespace) is rejected because agy silently
/// replaces it with its default model. A known runner version older than
/// 1.0.5 is rejected because it has no `--model` flag. An unknown version is
/// left to the CLI, which fails loudly on an unknown flag.
pub fn antigravity_model_launch_error(model: &str, version: Option<&str>) -> Option<String> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    if !model.contains(char::is_whitespace) {
        return Some(format!(
            "Antigravity CLI expects a model label from `agy models` (for example \"Gemini 3.1 Pro (High)\"), not \"{model}\". Pick a model from the list and relaunch."
        ));
    }
    let (major, minor, patch) = ANTIGRAVITY_MODEL_FLAG_MIN_VERSION;
    let minimum = semver::Version::new(major, minor, patch);
    let installed = version.and_then(|raw| {
        raw.split_whitespace()
            .find_map(|token| semver::Version::parse(token.strip_prefix('v').unwrap_or(token)).ok())
    });
    match installed {
        Some(installed) if installed < minimum => Some(format!(
            "Antigravity CLI {installed} cannot select a model (`--model` needs {minimum} or later). Run `agy update`, or clear the model selection to use the model set in agy's settings."
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_rows_are_agy_labels_not_slugs() {
        assert!(!ANTIGRAVITY_MODELS.is_empty());
        for model in ANTIGRAVITY_MODELS {
            assert!(
                model.label.contains(' '),
                "{} looks like a slug",
                model.label
            );
            assert_eq!(
                model.default_effort.is_some(),
                !model.efforts.is_empty(),
                "{}",
                model.label
            );
            if let Some(default) = model.default_effort {
                assert!(model.efforts.contains(&default));
            }
        }
    }

    #[test]
    fn model_arg_joins_the_label_with_the_reasoning_step() {
        assert_eq!(
            antigravity_model_arg("Gemini 3.1 Pro", Some("high")),
            "Gemini 3.1 Pro (High)"
        );
        assert_eq!(
            antigravity_model_arg("Gemini 3.5 Flash", Some("low")),
            "Gemini 3.5 Flash (Low)"
        );
        // No step, or a step the model lacks: the model's default step.
        assert_eq!(
            antigravity_model_arg("Gemini 3.5 Flash", None),
            "Gemini 3.5 Flash (Medium)"
        );
        assert_eq!(
            antigravity_model_arg("Gemini 3.1 Pro", Some("medium")),
            "Gemini 3.1 Pro (High)"
        );
        // Models without reasoning suffixes and labels outside the catalog.
        assert_eq!(
            antigravity_model_arg("Claude Opus 4.6", Some("high")),
            "Claude Opus 4.6"
        );
        assert_eq!(
            antigravity_model_arg("Gemini 3.1 Pro (Low)", Some("high")),
            "Gemini 3.1 Pro (Low)"
        );
    }

    #[test]
    fn launch_error_rejects_slugs_and_agy_without_model_flag() {
        assert!(
            antigravity_model_launch_error("gemini-3.5-pro", Some("1.2.0"))
                .is_some_and(|error| error.contains("agy models"))
        );
        assert!(
            antigravity_model_launch_error("Gemini 3.1 Pro (High)", Some("1.0.4"))
                .is_some_and(|error| error.contains("1.0.5"))
        );
        assert!(antigravity_model_launch_error("Gemini 3.1 Pro (High)", Some("v1.0.0")).is_some());
        assert_eq!(
            antigravity_model_launch_error("Gemini 3.1 Pro (High)", Some("1.0.5")),
            None
        );
        assert_eq!(
            antigravity_model_launch_error("Gemini 3.1 Pro (High)", None),
            None
        );
        // No model: agy uses its settings.json model on every version.
        assert_eq!(antigravity_model_launch_error("", Some("1.0.0")), None);
    }
}
