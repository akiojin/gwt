//! Git clone operations (gwt-spec issue)
//!
//! Provides repository cloning functionality.

/// Clone configuration (gwt-spec issue T301)
#[derive(Debug, Clone)]
pub struct CloneConfig {
    /// Repository URL to clone
    pub url: String,
    /// Target directory for the clone
    pub target_dir: std::path::PathBuf,
    /// Shallow clone with depth
    pub depth: Option<u32>,
}

/// Extract repository name from URL (gwt-spec issue)
///
/// Examples:
/// - `https://github.com/user/repo.git` -> `repo.git`
/// - `git@github.com:user/repo.git` -> `repo.git`
/// - `https://github.com/user/repo` -> `repo.git`
pub fn extract_repo_name(url: &str) -> String {
    let url = url.trim_end_matches('/');

    // Extract the last path segment
    let name = url
        .rsplit('/')
        .next()
        .or_else(|| url.rsplit(':').next())
        .unwrap_or("repo");

    // Add .git suffix if not present
    if name.ends_with(".git") {
        name.to_string()
    } else {
        format!("{}.git", name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_repo_name_https_with_git() {
        assert_eq!(
            extract_repo_name("https://github.com/user/repo.git"),
            "repo.git"
        );
    }

    #[test]
    fn test_extract_repo_name_https_without_git() {
        assert_eq!(
            extract_repo_name("https://github.com/user/repo"),
            "repo.git"
        );
    }

    #[test]
    fn test_extract_repo_name_ssh() {
        assert_eq!(
            extract_repo_name("git@github.com:user/repo.git"),
            "repo.git"
        );
    }

    #[test]
    fn test_extract_repo_name_trailing_slash() {
        assert_eq!(
            extract_repo_name("https://github.com/user/repo/"),
            "repo.git"
        );
    }
}
