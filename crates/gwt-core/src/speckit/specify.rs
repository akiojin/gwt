//! Specification generation via LLM

use std::collections::HashMap;

use crate::ai::{AIClient, AIError, ChatMessage};

use super::templates::{render_template, SPECIFY_TEMPLATE};

/// Run the specify workflow: generate spec.md content from user request
pub fn run_specify(
    client: &AIClient,
    user_request: &str,
    repository_context: &str,
    claude_md: &str,
    existing_specs: &str,
) -> Result<String, AIError> {
    let mut vars = HashMap::new();
    vars.insert("user_request".to_string(), user_request.to_string());
    vars.insert(
        "repository_context".to_string(),
        repository_context.to_string(),
    );
    vars.insert("claude_md".to_string(), claude_md.to_string());
    vars.insert("existing_specs".to_string(), existing_specs.to_string());

    let prompt = render_template(SPECIFY_TEMPLATE, &vars);

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
    fn test_specify_template_renders() {
        let mut vars = HashMap::new();
        vars.insert("user_request".to_string(), "Add login".to_string());
        vars.insert("repository_context".to_string(), "Rust project".to_string());
        vars.insert("claude_md".to_string(), "# Rules".to_string());
        vars.insert("existing_specs".to_string(), "SPEC-001".to_string());

        let result = render_template(SPECIFY_TEMPLATE, &vars);
        assert!(result.contains("Add login"));
        assert!(result.contains("Rust project"));
    }
}
