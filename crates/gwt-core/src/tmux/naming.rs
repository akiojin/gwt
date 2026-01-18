//! tmux session naming conventions
//!
//! Provides session name generation following the gwt-{repo} format.

/// Generate a unique session name for a repository
///
/// Format: gwt-{repo_name} or gwt-{repo_name}-{n} if already exists
///
/// # Arguments
/// * `repo_name` - The repository name
/// * `existing_sessions` - List of existing tmux session names
///
/// # Returns
/// A unique session name
pub fn generate_session_name(repo_name: &str, existing_sessions: &[String]) -> String {
    let sanitized = sanitize_session_name(repo_name);
    let base_name = format!("gwt-{}", sanitized);

    if !existing_sessions.contains(&base_name) {
        return base_name;
    }

    // Find the next available number
    let mut n = 2;
    loop {
        let candidate = format!("{}-{}", base_name, n);
        if !existing_sessions.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Sanitize repository name for use in tmux session name
///
/// Replaces special characters with hyphens
fn sanitize_session_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        // Remove consecutive hyphens
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Check if a session name belongs to gwt
pub fn is_gwt_session(session_name: &str) -> bool {
    session_name.starts_with("gwt-")
}

/// Extract repository name from gwt session name
pub fn extract_repo_name(session_name: &str) -> Option<&str> {
    if !is_gwt_session(session_name) {
        return None;
    }

    let without_prefix = session_name.strip_prefix("gwt-")?;

    // Remove any numeric suffix (e.g., "-2", "-3")
    if let Some(pos) = without_prefix.rfind('-') {
        let suffix = &without_prefix[pos + 1..];
        if suffix.chars().all(|c| c.is_ascii_digit()) {
            return Some(&without_prefix[..pos]);
        }
    }

    Some(without_prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_name_from_repo() {
        let name = generate_session_name("my-awesome-repo", &[]);
        assert_eq!(name, "gwt-my-awesome-repo");
    }

    #[test]
    fn test_session_name_with_existing_session() {
        let existing = vec!["gwt-my-repo".to_string()];
        let name = generate_session_name("my-repo", &existing);
        assert_eq!(name, "gwt-my-repo-2");
    }

    #[test]
    fn test_session_name_with_multiple_existing() {
        let existing = vec![
            "gwt-my-repo".to_string(),
            "gwt-my-repo-2".to_string(),
            "gwt-my-repo-3".to_string(),
        ];
        let name = generate_session_name("my-repo", &existing);
        assert_eq!(name, "gwt-my-repo-4");
    }

    #[test]
    fn test_session_name_sanitization() {
        let name = generate_session_name("my/repo@v1", &[]);
        assert_eq!(name, "gwt-my-repo-v1");
    }

    #[test]
    fn test_sanitize_special_characters() {
        assert_eq!(sanitize_session_name("foo/bar"), "foo-bar");
        assert_eq!(sanitize_session_name("foo@bar"), "foo-bar");
        assert_eq!(sanitize_session_name("foo:bar"), "foo-bar");
        assert_eq!(sanitize_session_name("foo.bar"), "foo-bar");
    }

    #[test]
    fn test_sanitize_consecutive_hyphens() {
        assert_eq!(sanitize_session_name("foo//bar"), "foo-bar");
        assert_eq!(sanitize_session_name("foo---bar"), "foo-bar");
    }

    #[test]
    fn test_is_gwt_session() {
        assert!(is_gwt_session("gwt-myrepo"));
        assert!(is_gwt_session("gwt-myrepo-2"));
        assert!(!is_gwt_session("myrepo"));
        assert!(!is_gwt_session("other-session"));
    }

    #[test]
    fn test_extract_repo_name() {
        assert_eq!(extract_repo_name("gwt-myrepo"), Some("myrepo"));
        assert_eq!(extract_repo_name("gwt-myrepo-2"), Some("myrepo"));
        assert_eq!(extract_repo_name("gwt-my-repo"), Some("my-repo"));
        assert_eq!(extract_repo_name("gwt-my-repo-3"), Some("my-repo"));
        assert_eq!(extract_repo_name("other-session"), None);
    }
}
