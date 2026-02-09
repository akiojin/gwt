//! Consistency analysis via LLM

use std::collections::HashMap;

use crate::ai::{AIClient, AIError, ChatMessage};

use super::templates::{render_template, ANALYZE_TEMPLATE};

/// Run the analyze workflow: check consistency of spec/plan/tasks
pub fn run_analyze(
    client: &AIClient,
    spec_content: &str,
    plan_content: &str,
    tasks_content: &str,
) -> Result<String, AIError> {
    let mut vars = HashMap::new();
    vars.insert("spec_content".to_string(), spec_content.to_string());
    vars.insert("plan_content".to_string(), plan_content.to_string());
    vars.insert("tasks_content".to_string(), tasks_content.to_string());

    let prompt = render_template(ANALYZE_TEMPLATE, &vars);

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
    fn test_analyze_template_renders() {
        let mut vars = HashMap::new();
        vars.insert("spec_content".to_string(), "# Spec".to_string());
        vars.insert("plan_content".to_string(), "# Plan".to_string());
        vars.insert("tasks_content".to_string(), "# Tasks".to_string());

        let result = render_template(ANALYZE_TEMPLATE, &vars);
        assert!(result.contains("# Spec"));
        assert!(result.contains("# Plan"));
        assert!(result.contains("# Tasks"));
    }
}
