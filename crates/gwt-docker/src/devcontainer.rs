//! DevContainer configuration parsing.
//!
//! Parses `.devcontainer/devcontainer.json` files, supporting JSON with
//! comments (JSONC) as used by VS Code DevContainers.

use std::path::Path;

use gwt_core::{GwtError, Result};
use serde::Deserialize;
use tracing::debug;

/// Parsed devcontainer.json configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DevContainerConfig {
    /// Container name.
    #[serde(default)]
    pub name: Option<String>,
    /// Docker image to use.
    #[serde(default)]
    pub image: Option<String>,
    /// Build configuration.
    #[serde(default)]
    pub build: Option<BuildConfig>,
    /// Ports to forward from container to host.
    #[serde(default)]
    pub forward_ports: Option<Vec<u16>>,
    /// Post-create command.
    #[serde(default)]
    pub post_create_command: Option<StringOrArray>,
    /// Reference to a docker-compose file.
    #[serde(default)]
    pub docker_compose_file: Option<StringOrArray>,
    /// Service to use from docker-compose.
    #[serde(default)]
    pub service: Option<String>,
    /// Working directory inside the container.
    #[serde(default)]
    pub workspace_folder: Option<String>,
}

/// Build configuration inside devcontainer.json.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BuildConfig {
    /// Dockerfile path (relative to .devcontainer).
    #[serde(default)]
    pub dockerfile: Option<String>,
    /// Build context path.
    #[serde(default)]
    pub context: Option<String>,
}

/// A string or array of strings (common in devcontainer.json).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StringOrArray {
    String(String),
    Array(Vec<String>),
}

impl StringOrArray {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            Self::String(s) => vec![s.clone()],
            Self::Array(v) => v.clone(),
        }
    }
}

impl std::fmt::Display for StringOrArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(s) => write!(f, "{s}"),
            Self::Array(v) => write!(f, "{}", v.join(" ")),
        }
    }
}

impl DevContainerConfig {
    /// Load and parse a devcontainer.json file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| GwtError::Docker(format!("failed to read devcontainer.json: {e}")))?;
        let cleaned = strip_json_comments(&content);
        let config: Self = serde_json::from_str(&cleaned)
            .map_err(|e| GwtError::Docker(format!("failed to parse devcontainer.json: {e}")))?;
        debug!(
            category = "docker",
            name = ?config.name,
            "loaded devcontainer.json"
        );
        Ok(config)
    }

    /// Get forwarded ports, defaulting to empty.
    pub fn get_forward_ports(&self) -> Vec<u16> {
        self.forward_ports.clone().unwrap_or_default()
    }

    /// Whether this config references a docker-compose file.
    pub fn uses_compose(&self) -> bool {
        self.docker_compose_file.is_some()
    }
}

/// Strip single-line (`//`) and multi-line (`/* */`) comments from JSONC.
fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escape = false;

    while let Some(c) = chars.next() {
        if escape {
            out.push(c);
            escape = false;
            continue;
        }
        if c == '\\' && in_string {
            out.push(c);
            escape = true;
            continue;
        }
        if c == '"' {
            in_string = !in_string;
            out.push(c);
            continue;
        }
        if in_string {
            out.push(c);
            continue;
        }
        if c == '/' {
            match chars.peek() {
                Some(&'/') => {
                    chars.next();
                    // Skip until newline.
                    for ch in chars.by_ref() {
                        if ch == '\n' {
                            break;
                        }
                    }
                    continue;
                }
                Some(&'*') => {
                    chars.next();
                    // Skip until `*/`.
                    while let Some(ch) = chars.next() {
                        if ch == '*' && chars.peek() == Some(&'/') {
                            chars.next();
                            break;
                        }
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn load_basic_devcontainer() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("devcontainer.json");
        std::fs::write(&path, r#"{ "name": "Test", "image": "ubuntu:22.04" }"#).unwrap();
        let cfg = DevContainerConfig::load(&path).unwrap();
        assert_eq!(cfg.name.as_deref(), Some("Test"));
        assert_eq!(cfg.image.as_deref(), Some("ubuntu:22.04"));
    }

    #[test]
    fn load_with_forward_ports() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("devcontainer.json");
        std::fs::write(
            &path,
            r#"{ "image": "node:18", "forwardPorts": [3000, 8080] }"#,
        )
        .unwrap();
        let cfg = DevContainerConfig::load(&path).unwrap();
        assert_eq!(cfg.get_forward_ports(), vec![3000, 8080]);
    }

    #[test]
    fn load_with_build_config() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("devcontainer.json");
        std::fs::write(
            &path,
            r#"{ "build": { "dockerfile": "Dockerfile", "context": ".." } }"#,
        )
        .unwrap();
        let cfg = DevContainerConfig::load(&path).unwrap();
        let build = cfg.build.unwrap();
        assert_eq!(build.dockerfile.as_deref(), Some("Dockerfile"));
        assert_eq!(build.context.as_deref(), Some(".."));
    }

    #[test]
    fn load_with_compose_reference() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("devcontainer.json");
        std::fs::write(
            &path,
            r#"{
                "dockerComposeFile": "docker-compose.yml",
                "service": "app",
                "workspaceFolder": "/workspace"
            }"#,
        )
        .unwrap();
        let cfg = DevContainerConfig::load(&path).unwrap();
        assert!(cfg.uses_compose());
        assert_eq!(cfg.service.as_deref(), Some("app"));
    }

    #[test]
    fn load_with_post_create_command_string() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("devcontainer.json");
        std::fs::write(&path, r#"{ "postCreateCommand": "npm install" }"#).unwrap();
        let cfg = DevContainerConfig::load(&path).unwrap();
        let cmd = cfg.post_create_command.unwrap();
        assert_eq!(cmd.to_vec(), vec!["npm install"]);
    }

    #[test]
    fn load_with_post_create_command_array() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("devcontainer.json");
        std::fs::write(&path, r#"{ "postCreateCommand": ["npm", "install"] }"#).unwrap();
        let cfg = DevContainerConfig::load(&path).unwrap();
        let cmd = cfg.post_create_command.unwrap();
        assert_eq!(cmd.to_vec(), vec!["npm", "install"]);
    }

    #[test]
    fn strip_single_line_comments() {
        let input = "{\n// comment\n\"key\": \"value\"\n}";
        let out = strip_json_comments(input);
        assert!(!out.contains("comment"));
        assert!(out.contains("\"key\""));
    }

    #[test]
    fn strip_multi_line_comments() {
        let input = "{\n/* multi\nline */\n\"key\": \"value\"\n}";
        let out = strip_json_comments(input);
        assert!(!out.contains("multi"));
        assert!(out.contains("\"key\""));
    }

    #[test]
    fn preserve_urls_in_strings() {
        let input = r#"{"url": "http://example.com/path"}"#;
        let out = strip_json_comments(input);
        assert!(out.contains("http://example.com/path"));
    }

    #[test]
    fn string_or_array_display() {
        let s = StringOrArray::String("hello".into());
        assert_eq!(s.to_string(), "hello");
        let a = StringOrArray::Array(vec!["a".into(), "b".into()]);
        assert_eq!(a.to_string(), "a b");
    }

    #[test]
    fn default_forward_ports_is_empty() {
        let cfg = DevContainerConfig::default();
        assert!(cfg.get_forward_ports().is_empty());
    }
}
