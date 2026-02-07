//! Plan generation via LLM

use std::collections::HashMap;

use crate::ai::{AIClient, AIError, ChatMessage};

use super::templates::{render_template, PLAN_TEMPLATE};

/// Run the plan workflow: generate plan.md content from spec
pub fn run_plan(
    client: &AIClient,
    spec_content: &str,
    repository_context: &str,
    claude_md: &str,
    directory_tree: &str,
) -> Result<String, AIError> {
    let mut vars = HashMap::new();
    vars.insert("spec_content".to_string(), spec_content.to_string());
    vars.insert(
        "repository_context".to_string(),
        repository_context.to_string(),
    );
    vars.insert("claude_md".to_string(), claude_md.to_string());
    vars.insert("directory_tree".to_string(), directory_tree.to_string());

    let prompt = render_template(PLAN_TEMPLATE, &vars);

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
    fn test_plan_template_renders() {
        let mut vars = HashMap::new();
        vars.insert("spec_content".to_string(), "# Spec".to_string());
        vars.insert("repository_context".to_string(), "Rust".to_string());
        vars.insert("claude_md".to_string(), "# Rules".to_string());
        vars.insert("directory_tree".to_string(), "src/".to_string());

        let result = render_template(PLAN_TEMPLATE, &vars);
        assert!(result.contains("# Spec"));
        assert!(result.contains("src/"));
    }
}
