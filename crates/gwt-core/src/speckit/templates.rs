//! LLM prompt templates for Spec Kit
//!
//! Templates are embedded at compile time via `include_str!`.

use std::collections::HashMap;

pub const SPECIFY_TEMPLATE: &str = include_str!("templates/specify.md");
pub const PLAN_TEMPLATE: &str = include_str!("templates/plan.md");
pub const TASKS_TEMPLATE: &str = include_str!("templates/tasks.md");
pub const CLARIFY_TEMPLATE: &str = include_str!("templates/clarify.md");
pub const ANALYZE_TEMPLATE: &str = include_str!("templates/analyze.md");

/// Render a template by replacing `{{variable}}` placeholders with values.
pub fn render_template(template: &str, vars: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::const_is_empty)]
    fn test_templates_are_non_empty() {
        assert!(!SPECIFY_TEMPLATE.is_empty());
        assert!(!PLAN_TEMPLATE.is_empty());
        assert!(!TASKS_TEMPLATE.is_empty());
        assert!(!CLARIFY_TEMPLATE.is_empty());
        assert!(!ANALYZE_TEMPLATE.is_empty());
    }

    #[test]
    fn test_render_template_basic() {
        let template = "Hello {{name}}, welcome to {{project}}!";
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Alice".to_string());
        vars.insert("project".to_string(), "gwt".to_string());
        let result = render_template(template, &vars);
        assert_eq!(result, "Hello Alice, welcome to gwt!");
    }

    #[test]
    fn test_render_template_no_vars() {
        let template = "No variables here.";
        let vars = HashMap::new();
        let result = render_template(template, &vars);
        assert_eq!(result, "No variables here.");
    }

    #[test]
    fn test_render_template_missing_var() {
        let template = "Hello {{name}}, {{missing}} value.";
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Bob".to_string());
        let result = render_template(template, &vars);
        assert_eq!(result, "Hello Bob, {{missing}} value.");
    }

    #[test]
    fn test_render_template_multiple_occurrences() {
        let template = "{{x}} and {{x}} again";
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), "val".to_string());
        let result = render_template(template, &vars);
        assert_eq!(result, "val and val again");
    }
}
