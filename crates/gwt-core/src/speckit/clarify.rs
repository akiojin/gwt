//! Clarification question generation via LLM

use std::collections::HashMap;

use crate::ai::{AIClient, AIError, ChatMessage};

use super::templates::{render_template, CLARIFY_TEMPLATE};

/// Run the clarify workflow: generate clarifying questions from spec
pub fn run_clarify(
    client: &AIClient,
    spec_content: &str,
    repository_context: &str,
) -> Result<String, AIError> {
    let mut vars = HashMap::new();
    vars.insert("spec_content".to_string(), spec_content.to_string());
    vars.insert(
        "repository_context".to_string(),
        repository_context.to_string(),
    );

    let prompt = render_template(CLARIFY_TEMPLATE, &vars);

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    client.create_response(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clarify_template_renders() {
        let mut vars = HashMap::new();
        vars.insert("spec_content".to_string(), "# Spec".to_string());
        vars.insert("repository_context".to_string(), "Rust".to_string());

        let result = render_template(CLARIFY_TEMPLATE, &vars);
        assert!(result.contains("# Spec"));
    }
}
