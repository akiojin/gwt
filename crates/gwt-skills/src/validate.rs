//! YAML frontmatter validation for SKILL.md files.
//!
//! This module provides the same validation logic used by `build.rs` at
//! compile time, exposed as a testable public API.

/// Extract the YAML frontmatter block (between `---` delimiters) from a
/// markdown file. Returns `None` if no frontmatter is found.
pub fn extract_frontmatter(content: &str) -> Option<&str> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let after_first = &content[3..];
    let end = after_first.find("\n---")?;
    Some(&after_first[..end])
}

/// Validate that the YAML frontmatter of a SKILL.md string is syntactically
/// correct. Returns `Ok(())` if valid or no frontmatter present, `Err` with
/// the parse error otherwise.
pub fn validate_frontmatter(content: &str) -> Result<(), String> {
    if let Some(fm) = extract_frontmatter(content) {
        serde_yaml::from_str::<serde_yaml::Value>(fm)
            .map(|_| ())
            .map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_valid_frontmatter() {
        let content = "---\nname: test\ndescription: hello\n---\n# Body";
        let fm = extract_frontmatter(content).unwrap();
        assert!(fm.contains("name: test"));
    }

    #[test]
    fn extract_no_frontmatter() {
        assert!(extract_frontmatter("# Just markdown").is_none());
    }

    #[test]
    fn extract_unclosed_frontmatter() {
        assert!(extract_frontmatter("---\nname: test\n# no closing").is_none());
    }

    #[test]
    fn validate_valid_yaml() {
        let content = "---\nname: test\ndescription: \"hello world\"\n---\n# Body";
        assert!(validate_frontmatter(content).is_ok());
    }

    #[test]
    fn validate_malformed_yaml_returns_error() {
        let content = "---\nname: test\n  bad indent: [\n---\n# Body";
        let result = validate_frontmatter(content);
        assert!(result.is_err(), "expected error for malformed YAML");
    }

    #[test]
    fn validate_no_frontmatter_is_ok() {
        assert!(validate_frontmatter("# Just markdown").is_ok());
    }

    #[test]
    fn validate_empty_frontmatter_is_ok() {
        assert!(validate_frontmatter("---\n---\n# Body").is_ok());
    }

    #[test]
    fn validate_real_skill_frontmatter() {
        let content = r#"---
name: gwt-pr
description: "This skill should be used when the user asks to open a PR."
allowed-tools: Bash, Read, Glob, Grep
argument-hint: "[optional context]"
---
# Body"#;
        assert!(validate_frontmatter(content).is_ok());
    }
}
