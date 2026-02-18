//! devcontainer.json parsing (SPEC-f5f5657e)
//!
//! Parses .devcontainer/devcontainer.json files and converts them
//! to docker-compose compatible configurations.

use serde::Deserialize;
use std::fmt;
use std::path::Path;
use tracing::debug;

use crate::{GwtError, Result};

/// devcontainer.json configuration
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DevContainerConfig {
    /// Container name
    #[serde(default)]
    pub name: Option<String>,

    /// Reference to a docker-compose file
    #[serde(default)]
    pub docker_compose_file: Option<StringOrArray>,

    /// Service to use from docker-compose
    #[serde(default)]
    pub service: Option<String>,

    /// Dockerfile path (relative to .devcontainer)
    #[serde(default)]
    pub dockerfile: Option<String>,

    /// Docker image to use
    #[serde(default)]
    pub image: Option<String>,

    /// Build configuration
    #[serde(default)]
    pub build: Option<BuildConfig>,

    /// Ports to forward
    #[serde(default)]
    pub forward_ports: Option<Vec<u16>>,

    /// Working directory inside container
    #[serde(default)]
    pub workspace_folder: Option<String>,

    /// Run arguments for docker
    #[serde(default)]
    pub run_args: Option<Vec<String>>,

    /// Features to install
    #[serde(default)]
    pub features: Option<serde_json::Value>,

    /// Post-create command
    #[serde(default)]
    pub post_create_command: Option<StringOrArray>,

    /// Post-start command
    #[serde(default)]
    pub post_start_command: Option<StringOrArray>,
}

/// String or array of strings (common pattern in devcontainer.json)
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StringOrArray {
    String(String),
    Array(Vec<String>),
}

impl StringOrArray {
    /// Convert to vector of strings
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            StringOrArray::String(s) => vec![s.clone()],
            StringOrArray::Array(arr) => arr.clone(),
        }
    }
}

impl fmt::Display for StringOrArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StringOrArray::String(s) => write!(f, "{}", s),
            StringOrArray::Array(arr) => write!(f, "{}", arr.join(" ")),
        }
    }
}

/// Build configuration for devcontainer
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BuildConfig {
    /// Dockerfile path
    #[serde(default)]
    pub dockerfile: Option<String>,

    /// Build context path
    #[serde(default)]
    pub context: Option<String>,

    /// Build arguments
    #[serde(default)]
    pub args: Option<std::collections::HashMap<String, String>>,

    /// Target stage in multi-stage build
    #[serde(default)]
    pub target: Option<String>,
}

impl DevContainerConfig {
    /// Load devcontainer.json from a path
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| GwtError::Docker(format!("Failed to read devcontainer.json: {}", e)))?;

        // Remove comments (JSON with comments support)
        let content = remove_json_comments(&content);

        let config: DevContainerConfig = serde_json::from_str(&content)
            .map_err(|e| GwtError::Docker(format!("Failed to parse devcontainer.json: {}", e)))?;

        debug!(
            category = "docker",
            name = ?config.name,
            has_compose = config.docker_compose_file.is_some(),
            has_dockerfile = config.dockerfile.is_some() || config.build.as_ref().map(|b| b.dockerfile.is_some()).unwrap_or(false),
            has_image = config.image.is_some(),
            "Loaded devcontainer.json"
        );

        Ok(config)
    }

    /// Check if this config uses docker-compose
    pub fn uses_compose(&self) -> bool {
        self.docker_compose_file.is_some()
    }

    /// Check if this config uses a Dockerfile
    pub fn uses_dockerfile(&self) -> bool {
        self.dockerfile.is_some()
            || self
                .build
                .as_ref()
                .map(|b| b.dockerfile.is_some())
                .unwrap_or(false)
    }

    /// Check if this config uses a pre-built image
    pub fn uses_image(&self) -> bool {
        self.image.is_some() && !self.uses_dockerfile()
    }

    /// Get the docker-compose file paths
    pub fn get_compose_files(&self) -> Vec<String> {
        match &self.docker_compose_file {
            Some(files) => files.to_vec(),
            None => Vec::new(),
        }
    }

    /// Get the Dockerfile path (relative to .devcontainer)
    pub fn get_dockerfile(&self) -> Option<String> {
        self.dockerfile
            .clone()
            .or_else(|| self.build.as_ref().and_then(|b| b.dockerfile.clone()))
    }

    /// Convert to docker compose arguments
    ///
    /// Returns arguments that can be used with docker compose commands.
    pub fn to_compose_args(&self, devcontainer_dir: &Path) -> Vec<String> {
        let mut args = Vec::new();

        // If using compose file, reference it
        if let Some(files) = &self.docker_compose_file {
            for file in files.to_vec() {
                let compose_path = devcontainer_dir.join(&file);
                args.push("-f".to_string());
                args.push(compose_path.to_string_lossy().to_string());
            }
        }

        args
    }

    /// Get the service name to use
    pub fn get_service(&self) -> Option<&str> {
        self.service.as_deref()
    }

    /// Get forwarded ports
    pub fn get_forward_ports(&self) -> Vec<u16> {
        self.forward_ports.clone().unwrap_or_default()
    }
}

/// Remove single-line and multi-line comments from JSON
fn remove_json_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;

    while let Some(c) = chars.next() {
        if escape_next {
            result.push(c);
            escape_next = false;
            continue;
        }

        if c == '\\' && in_string {
            result.push(c);
            escape_next = true;
            continue;
        }

        if c == '"' {
            in_string = !in_string;
            result.push(c);
            continue;
        }

        if in_string {
            result.push(c);
            continue;
        }

        if c == '/' {
            if let Some(&next) = chars.peek() {
                if next == '/' {
                    // Single-line comment - skip until newline
                    chars.next();
                    while let Some(&ch) = chars.peek() {
                        if ch == '\n' {
                            break;
                        }
                        chars.next();
                    }
                    continue;
                } else if next == '*' {
                    // Multi-line comment - skip until */
                    chars.next();
                    while let Some(ch) = chars.next() {
                        if ch == '*' {
                            if let Some(&'/') = chars.peek() {
                                chars.next();
                                break;
                            }
                        }
                    }
                    continue;
                }
            }
        }

        result.push(c);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // T-501: devcontainer.json parsing test
    #[test]
    fn test_load_basic_devcontainer() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("devcontainer.json");

        std::fs::write(
            &config_path,
            r#"{
                "name": "Test Container",
                "image": "mcr.microsoft.com/devcontainers/base:ubuntu"
            }"#,
        )
        .unwrap();

        let config = DevContainerConfig::load(&config_path).unwrap();
        assert_eq!(config.name, Some("Test Container".to_string()));
        assert_eq!(
            config.image,
            Some("mcr.microsoft.com/devcontainers/base:ubuntu".to_string())
        );
        assert!(config.uses_image());
        assert!(!config.uses_compose());
        assert!(!config.uses_dockerfile());
    }

    // T-502: dockerComposeFile test
    #[test]
    fn test_load_devcontainer_with_compose() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("devcontainer.json");

        std::fs::write(
            &config_path,
            r#"{
                "name": "Compose Container",
                "dockerComposeFile": "docker-compose.yml",
                "service": "app",
                "workspaceFolder": "/workspace"
            }"#,
        )
        .unwrap();

        let config = DevContainerConfig::load(&config_path).unwrap();
        assert!(config.uses_compose());
        assert_eq!(config.get_service(), Some("app"));
        assert_eq!(config.get_compose_files(), vec!["docker-compose.yml"]);
    }

    // T-502: multiple compose files
    #[test]
    fn test_load_devcontainer_with_multiple_compose_files() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("devcontainer.json");

        std::fs::write(
            &config_path,
            r#"{
                "dockerComposeFile": ["docker-compose.yml", "docker-compose.override.yml"],
                "service": "web"
            }"#,
        )
        .unwrap();

        let config = DevContainerConfig::load(&config_path).unwrap();
        assert!(config.uses_compose());
        assert_eq!(
            config.get_compose_files(),
            vec!["docker-compose.yml", "docker-compose.override.yml"]
        );
    }

    // T-503: Dockerfile test
    #[test]
    fn test_load_devcontainer_with_dockerfile() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("devcontainer.json");

        std::fs::write(
            &config_path,
            r#"{
                "name": "Dockerfile Container",
                "build": {
                    "dockerfile": "Dockerfile",
                    "context": ".."
                }
            }"#,
        )
        .unwrap();

        let config = DevContainerConfig::load(&config_path).unwrap();
        assert!(config.uses_dockerfile());
        assert!(!config.uses_compose());
        assert_eq!(config.get_dockerfile(), Some("Dockerfile".to_string()));
    }

    #[test]
    fn test_load_devcontainer_with_forward_ports() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("devcontainer.json");

        std::fs::write(
            &config_path,
            r#"{
                "image": "node:18",
                "forwardPorts": [3000, 8080, 5432]
            }"#,
        )
        .unwrap();

        let config = DevContainerConfig::load(&config_path).unwrap();
        assert_eq!(config.get_forward_ports(), vec![3000, 8080, 5432]);
    }

    #[test]
    fn test_remove_json_comments_single_line() {
        let input = r#"{
            // This is a comment
            "key": "value"
        }"#;
        let result = remove_json_comments(input);
        assert!(!result.contains("This is a comment"));
        assert!(result.contains("\"key\": \"value\""));
    }

    #[test]
    fn test_remove_json_comments_multi_line() {
        let input = r#"{
            /* Multi-line
               comment */
            "key": "value"
        }"#;
        let result = remove_json_comments(input);
        assert!(!result.contains("Multi-line"));
        assert!(result.contains("\"key\": \"value\""));
    }

    #[test]
    fn test_remove_json_comments_in_string() {
        let input = r#"{"url": "http://example.com/path"}"#;
        let result = remove_json_comments(input);
        // URL should not be treated as comment
        assert!(result.contains("http://example.com/path"));
    }

    #[test]
    fn test_string_or_array_string() {
        let s = StringOrArray::String("hello".to_string());
        assert_eq!(s.to_string(), "hello");
        assert_eq!(s.to_vec(), vec!["hello"]);
    }

    #[test]
    fn test_string_or_array_array() {
        let s = StringOrArray::Array(vec!["a".to_string(), "b".to_string()]);
        assert_eq!(s.to_string(), "a b");
        assert_eq!(s.to_vec(), vec!["a", "b"]);
    }

    #[test]
    fn test_to_compose_args() {
        let temp_dir = TempDir::new().unwrap();
        let config = DevContainerConfig {
            docker_compose_file: Some(StringOrArray::String("docker-compose.yml".to_string())),
            ..Default::default()
        };

        let args = config.to_compose_args(temp_dir.path());
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-f");
        assert!(args[1].ends_with("docker-compose.yml"));
    }
}
