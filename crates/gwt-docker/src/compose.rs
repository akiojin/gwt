//! Docker Compose file parsing.
//!
//! Extracts service definitions from docker-compose.yml / compose.yml files.

use std::path::Path;

use gwt_core::{GwtError, Result};
use serde_yaml::Value;
use tracing::debug;

/// A service defined in a Docker Compose file.
#[derive(Debug, Clone)]
pub struct ComposeService {
    /// Service name.
    pub name: String,
    /// Image name (if specified).
    pub image: Option<String>,
    /// Published ports (raw strings, e.g. "8080:80").
    pub ports: Vec<String>,
    /// Services this service depends on.
    pub depends_on: Vec<String>,
    /// Working directory inside the container.
    pub working_dir: Option<String>,
    /// Volume mounts declared for the service.
    pub volumes: Vec<ComposeVolumeMount>,
}

/// A parsed Compose volume mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeVolumeMount {
    /// Source path or named volume.
    pub source: String,
    /// Target path inside the container.
    pub target: String,
    /// Optional mode suffix such as `ro`.
    pub mode: Option<String>,
}

/// Parse a Docker Compose file and return its service definitions.
pub fn parse_compose_file(path: &Path) -> Result<Vec<ComposeService>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| GwtError::Docker(format!("failed to read compose file: {e}")))?;
    parse_compose_content(&content)
}

fn parse_compose_content(content: &str) -> Result<Vec<ComposeService>> {
    let root: Value = serde_yaml::from_str(content)
        .map_err(|e| GwtError::Docker(format!("failed to parse compose YAML: {e}")))?;

    let services = root
        .get("services")
        .and_then(|v| v.as_mapping())
        .ok_or_else(|| GwtError::Docker("no 'services' key in compose file".to_string()))?;

    let mut result = Vec::new();

    for (key, value) in services {
        let name = key.as_str().unwrap_or_default().to_string();

        let image = value
            .get("image")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let ports = value
            .get("ports")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let depends_on = extract_depends_on(value);
        let working_dir = value
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let volumes = extract_volumes(value);

        result.push(ComposeService {
            name,
            image,
            ports,
            depends_on,
            working_dir,
            volumes,
        });
    }

    debug!(
        category = "docker",
        count = result.len(),
        "parsed compose services"
    );
    Ok(result)
}

/// Extract depends_on which can be either a list of strings or a mapping.
fn extract_depends_on(service: &Value) -> Vec<String> {
    match service.get("depends_on") {
        Some(Value::Sequence(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        Some(Value::Mapping(map)) => map
            .keys()
            .filter_map(|k| k.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

fn extract_volumes(service: &Value) -> Vec<ComposeVolumeMount> {
    service
        .get("volumes")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|entry| match entry {
                    Value::String(raw) => parse_volume_mount(raw),
                    Value::Mapping(map) => {
                        let source = map
                            .get(Value::String("source".to_string()))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let target = map
                            .get(Value::String("target".to_string()))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        if source.is_empty() || target.is_empty() {
                            None
                        } else {
                            Some(ComposeVolumeMount {
                                source: source.to_string(),
                                target: target.to_string(),
                                mode: map
                                    .get(Value::String("read_only".to_string()))
                                    .and_then(|v| v.as_bool())
                                    .filter(|read_only| *read_only)
                                    .map(|_| "ro".to_string()),
                            })
                        }
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_volume_mount(raw: &str) -> Option<ComposeVolumeMount> {
    let raw = raw.trim();
    let (mount_spec, mode) = match raw.rsplit_once(':') {
        Some((spec, suffix)) if looks_like_volume_mode(suffix) => {
            (spec, Some(suffix.trim().to_string()))
        }
        _ => (raw, None),
    };
    let (source, target) = split_volume_source_and_target(mount_spec)?;
    if source.is_empty() || target.is_empty() {
        return None;
    }
    Some(ComposeVolumeMount {
        source: source.to_string(),
        target: target.to_string(),
        mode,
    })
}

fn looks_like_volume_mode(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && !trimmed.contains(['/', '\\'])
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ',' | '=' | '-' | '_'))
}

fn split_volume_source_and_target(raw: &str) -> Option<(&str, &str)> {
    for (idx, ch) in raw.char_indices().rev() {
        if ch != ':' {
            continue;
        }

        let source = raw[..idx].trim();
        let target = raw[idx + 1..].trim();
        if source.is_empty() || target.is_empty() {
            continue;
        }
        if idx == 1 && source.chars().all(|part| part.is_ascii_alphabetic()) {
            continue;
        }

        return Some((source, target));
    }

    None
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn parse_simple_compose() {
        let yaml = r#"
services:
  web:
    image: nginx:latest
    ports:
      - "8080:80"
  db:
    image: postgres:15
    ports:
      - "5432:5432"
"#;
        let services = parse_compose_content(yaml).unwrap();
        assert_eq!(services.len(), 2);

        let web = services.iter().find(|s| s.name == "web").unwrap();
        assert_eq!(web.image.as_deref(), Some("nginx:latest"));
        assert_eq!(web.ports, vec!["8080:80"]);

        let db = services.iter().find(|s| s.name == "db").unwrap();
        assert_eq!(db.image.as_deref(), Some("postgres:15"));
    }

    #[test]
    fn parse_depends_on_list() {
        let yaml = r#"
services:
  web:
    image: node:18
    depends_on:
      - db
      - redis
  db:
    image: postgres:15
  redis:
    image: redis:7
"#;
        let services = parse_compose_content(yaml).unwrap();
        let web = services.iter().find(|s| s.name == "web").unwrap();
        assert_eq!(web.depends_on, vec!["db", "redis"]);
    }

    #[test]
    fn parse_depends_on_mapping() {
        let yaml = r#"
services:
  web:
    image: node:18
    depends_on:
      db:
        condition: service_healthy
  db:
    image: postgres:15
"#;
        let services = parse_compose_content(yaml).unwrap();
        let web = services.iter().find(|s| s.name == "web").unwrap();
        assert_eq!(web.depends_on, vec!["db"]);
    }

    #[test]
    fn parse_service_without_image() {
        let yaml = r#"
services:
  app:
    build: .
    ports:
      - "3000:3000"
"#;
        let services = parse_compose_content(yaml).unwrap();
        assert_eq!(services.len(), 1);
        assert!(services[0].image.is_none());
    }

    #[test]
    fn parse_working_dir_and_volumes() {
        let yaml = r#"
services:
  app:
    image: node:18
    working_dir: /workspace
    volumes:
      - .:/workspace
      - cache:/cache:ro
"#;
        let services = parse_compose_content(yaml).unwrap();
        assert_eq!(services[0].working_dir.as_deref(), Some("/workspace"));
        assert_eq!(services[0].volumes.len(), 2);
        assert_eq!(services[0].volumes[0].source, ".");
        assert_eq!(services[0].volumes[0].target, "/workspace");
        assert_eq!(services[0].volumes[1].mode.as_deref(), Some("ro"));
    }

    #[test]
    fn parse_windows_drive_letter_volume_source() {
        let mount = parse_volume_mount(r"C:\repo:/workspace:ro").unwrap();
        assert_eq!(mount.source, r"C:\repo");
        assert_eq!(mount.target, "/workspace");
        assert_eq!(mount.mode.as_deref(), Some("ro"));

        let mount = parse_volume_mount(r"C:\repo:/workspace").unwrap();
        assert_eq!(mount.source, r"C:\repo");
        assert_eq!(mount.target, "/workspace");
        assert_eq!(mount.mode, None);
    }

    #[test]
    fn parse_compose_file_from_disk() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("docker-compose.yml");
        std::fs::write(&path, "services:\n  app:\n    image: alpine:3.18\n").unwrap();
        let services = parse_compose_file(&path).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "app");
    }

    #[test]
    fn missing_services_key_returns_error() {
        let yaml = "version: '3'\n";
        let result = parse_compose_content(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn empty_services_returns_empty_vec() {
        let yaml = "services: {}\n";
        let services = parse_compose_content(yaml).unwrap();
        assert!(services.is_empty());
    }
}
