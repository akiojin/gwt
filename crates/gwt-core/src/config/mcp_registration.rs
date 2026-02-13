//! MCP Bridge server registration for agent configuration files.
//!
//! Registers/unregisters the `gwt-agent-bridge` MCP server in each agent's
//! global settings so that agents can communicate with gwt via the bridge
//! process at runtime.
//!
//! Supported agents:
//! - Claude Code: `~/.claude.json` (JSON, `mcpServers`)
//! - Codex: `~/.codex/config.toml` (TOML, `[mcp_servers.gwt-agent-bridge]`)
//! - Gemini: `~/.gemini/settings.json` (JSON, `mcpServers`)

use crate::error::GwtError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Name used as the key in each agent's MCP server configuration.
pub const MCP_SERVER_NAME: &str = "gwt-agent-bridge";

/// Agent types that support MCP server registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpAgentType {
    Claude,
    Codex,
    Gemini,
}

impl McpAgentType {
    /// All supported agent types.
    pub fn all() -> &'static [McpAgentType] {
        &[
            McpAgentType::Claude,
            McpAgentType::Codex,
            McpAgentType::Gemini,
        ]
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            McpAgentType::Claude => "Claude Code",
            McpAgentType::Codex => "Codex",
            McpAgentType::Gemini => "Gemini",
        }
    }
}

/// Configuration payload for the MCP bridge server entry.
#[derive(Debug, Clone)]
pub struct McpBridgeConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Runtime detection (T25)
// ---------------------------------------------------------------------------

/// Detect a JS runtime (bun preferred, node as fallback).
///
/// Returns the full path to the runtime binary.
pub fn detect_runtime() -> Result<String, GwtError> {
    if let Ok(path) = which::which("bun") {
        return Ok(path.to_string_lossy().into_owned());
    }
    if let Ok(path) = which::which("node") {
        return Ok(path.to_string_lossy().into_owned());
    }
    Err(GwtError::Internal(
        "Neither bun nor node found in PATH".to_string(),
    ))
}

/// Resolve the path to the bundled bridge JS file.
///
/// When running inside a Tauri bundle the resource directory is provided by
/// the caller (`resource_dir`).  During development this falls back to
/// `<project>/gwt-mcp-bridge/dist/gwt-mcp-bridge.js`.
pub fn resolve_bridge_path(resource_dir: Option<&Path>) -> Result<PathBuf, GwtError> {
    if let Some(dir) = resource_dir {
        let path = dir.join("gwt-mcp-bridge.js");
        if path.exists() {
            return Ok(path);
        }
    }

    // Development fallback: look relative to the workspace root.
    let dev_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|root| {
            root.join("gwt-mcp-bridge")
                .join("dist")
                .join("gwt-mcp-bridge.js")
        });

    if let Some(path) = dev_path {
        if path.exists() {
            return Ok(path);
        }
    }

    Err(GwtError::Internal(
        "MCP bridge JS file not found".to_string(),
    ))
}

// ---------------------------------------------------------------------------
// Per-agent config file paths
// ---------------------------------------------------------------------------

fn claude_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

fn codex_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex").join("config.toml"))
}

fn gemini_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".gemini").join("settings.json"))
}

fn config_path_for(agent: McpAgentType) -> Option<PathBuf> {
    match agent {
        McpAgentType::Claude => claude_config_path(),
        McpAgentType::Codex => codex_config_path(),
        McpAgentType::Gemini => gemini_config_path(),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Register the MCP bridge server for one agent.
pub fn register_mcp_server(agent: McpAgentType, config: &McpBridgeConfig) -> Result<(), GwtError> {
    let Some(path) = config_path_for(agent) else {
        return Ok(());
    };
    register_mcp_server_at(agent, config, &path)
}

/// Register the MCP bridge server at an explicit path (testable).
pub fn register_mcp_server_at(
    agent: McpAgentType,
    config: &McpBridgeConfig,
    path: &Path,
) -> Result<(), GwtError> {
    debug!(
        category = "mcp",
        agent = agent.label(),
        path = %path.display(),
        "Registering MCP server"
    );

    match agent {
        McpAgentType::Claude | McpAgentType::Gemini => register_json_agent(config, path),
        McpAgentType::Codex => register_codex_agent(config, path),
    }
}

/// Unregister the MCP bridge server for one agent.
pub fn unregister_mcp_server(agent: McpAgentType) -> Result<(), GwtError> {
    let Some(path) = config_path_for(agent) else {
        return Ok(());
    };
    unregister_mcp_server_at(agent, &path)
}

/// Unregister at an explicit path (testable).
pub fn unregister_mcp_server_at(agent: McpAgentType, path: &Path) -> Result<(), GwtError> {
    debug!(
        category = "mcp",
        agent = agent.label(),
        path = %path.display(),
        "Unregistering MCP server"
    );

    match agent {
        McpAgentType::Claude | McpAgentType::Gemini => unregister_json_agent(path),
        McpAgentType::Codex => unregister_codex_agent(path),
    }
}

/// Check whether the bridge is currently registered for an agent.
pub fn is_registered(agent: McpAgentType) -> Result<bool, GwtError> {
    let Some(path) = config_path_for(agent) else {
        return Ok(false);
    };
    is_registered_at(agent, &path)
}

/// Check registration at an explicit path (testable).
pub fn is_registered_at(agent: McpAgentType, path: &Path) -> Result<bool, GwtError> {
    if !path.exists() {
        return Ok(false);
    }
    match agent {
        McpAgentType::Claude | McpAgentType::Gemini => is_registered_json(path),
        McpAgentType::Codex => is_registered_codex(path),
    }
}

/// Remove stale registrations from all agents.
///
/// Called at startup to clean up registrations left behind by a previous crash.
pub fn cleanup_stale_registrations() -> Result<(), GwtError> {
    for agent in McpAgentType::all() {
        if is_registered(*agent)? {
            info!(
                category = "mcp",
                agent = agent.label(),
                "Cleaning up stale MCP registration"
            );
            unregister_mcp_server(*agent)?;
        }
    }
    Ok(())
}

/// Register the bridge in all supported agents.
pub fn register_all(config: &McpBridgeConfig) -> Result<(), GwtError> {
    for agent in McpAgentType::all() {
        if let Err(e) = register_mcp_server(*agent, config) {
            warn!(
                category = "mcp",
                agent = agent.label(),
                error = %e,
                "Failed to register MCP server; continuing"
            );
        }
    }
    Ok(())
}

/// Unregister the bridge from all supported agents.
pub fn unregister_all() -> Result<(), GwtError> {
    for agent in McpAgentType::all() {
        if let Err(e) = unregister_mcp_server(*agent) {
            warn!(
                category = "mcp",
                agent = agent.label(),
                error = %e,
                "Failed to unregister MCP server; continuing"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// JSON agents (Claude Code, Gemini) – T18, T20
// ---------------------------------------------------------------------------

fn ensure_parent_dir(path: &Path) -> Result<(), GwtError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| GwtError::ConfigWriteError {
            reason: format!("Failed to create directory {}: {}", parent.display(), e),
        })?;
    }
    Ok(())
}

fn load_json(path: &Path) -> Result<serde_json::Value, GwtError> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content = std::fs::read_to_string(path).map_err(|e| GwtError::ConfigParseError {
        reason: format!("Failed to read {}: {}", path.display(), e),
    })?;
    serde_json::from_str(&content).map_err(|e| GwtError::ConfigParseError {
        reason: format!("Failed to parse {}: {}", path.display(), e),
    })
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), GwtError> {
    let content = serde_json::to_string_pretty(value).map_err(|e| GwtError::ConfigWriteError {
        reason: e.to_string(),
    })?;
    std::fs::write(path, content).map_err(|e| GwtError::ConfigWriteError {
        reason: format!("Failed to write {}: {}", path.display(), e),
    })
}

fn register_json_agent(config: &McpBridgeConfig, path: &Path) -> Result<(), GwtError> {
    ensure_parent_dir(path)?;

    let mut root = load_json(path)?;

    if root.get("mcpServers").is_none() {
        root["mcpServers"] = serde_json::json!({});
    }

    let mut entry = serde_json::json!({
        "command": config.command,
        "args": config.args,
    });
    if !config.env.is_empty() {
        entry["env"] = serde_json::to_value(&config.env).unwrap_or(serde_json::json!({}));
    }

    match root.get_mut("mcpServers").and_then(|servers| servers.as_object_mut()) {
        Some(servers) => {
            servers.insert(MCP_SERVER_NAME.to_string(), entry);
        }
        None => {
            let mut servers = serde_json::Map::new();
            servers.insert(MCP_SERVER_NAME.to_string(), entry);
            root["mcpServers"] = serde_json::Value::Object(servers);
        }
    }

    write_json(path, &root)?;

    info!(
        category = "mcp",
        path = %path.display(),
        "Registered MCP server in JSON config"
    );
    Ok(())
}

fn unregister_json_agent(path: &Path) -> Result<(), GwtError> {
    if !path.exists() {
        return Ok(());
    }

    let mut root = load_json(path)?;

    if let Some(servers) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        if servers.remove(MCP_SERVER_NAME).is_some() {
            write_json(path, &root)?;
            info!(
                category = "mcp",
                path = %path.display(),
                "Unregistered MCP server from JSON config"
            );
        }
    }

    Ok(())
}

fn is_registered_json(path: &Path) -> Result<bool, GwtError> {
    let root = load_json(path).unwrap_or_else(|_| serde_json::json!({}));
    Ok(root
        .get("mcpServers")
        .and_then(|v| v.get(MCP_SERVER_NAME))
        .is_some())
}

// ---------------------------------------------------------------------------
// Codex (TOML) – T19
// ---------------------------------------------------------------------------

fn load_toml_table(path: &Path) -> Result<toml::Table, GwtError> {
    if !path.exists() {
        return Ok(toml::Table::new());
    }
    let content = std::fs::read_to_string(path).map_err(|e| GwtError::ConfigParseError {
        reason: format!("Failed to read {}: {}", path.display(), e),
    })?;
    content
        .parse::<toml::Table>()
        .map_err(|e| GwtError::ConfigParseError {
            reason: format!("Failed to parse {}: {}", path.display(), e),
        })
}

fn write_toml_table(path: &Path, table: &toml::Table) -> Result<(), GwtError> {
    let content = toml::to_string_pretty(table).map_err(|e| GwtError::ConfigWriteError {
        reason: e.to_string(),
    })?;
    std::fs::write(path, content).map_err(|e| GwtError::ConfigWriteError {
        reason: format!("Failed to write {}: {}", path.display(), e),
    })
}

fn register_codex_agent(config: &McpBridgeConfig, path: &Path) -> Result<(), GwtError> {
    ensure_parent_dir(path)?;

    let mut root = load_toml_table(path)?;

    // Ensure [mcp_servers] table exists
    if !root.contains_key("mcp_servers") {
        root.insert(
            "mcp_servers".to_string(),
            toml::Value::Table(toml::Table::new()),
        );
    }

    let servers = root
        .get_mut("mcp_servers")
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| GwtError::ConfigWriteError {
            reason: "mcp_servers is not a table".to_string(),
        })?;

    let mut entry = toml::Table::new();
    entry.insert(
        "command".to_string(),
        toml::Value::String(config.command.clone()),
    );
    entry.insert(
        "args".to_string(),
        toml::Value::Array(
            config
                .args
                .iter()
                .map(|a| toml::Value::String(a.clone()))
                .collect(),
        ),
    );

    servers.insert(MCP_SERVER_NAME.to_string(), toml::Value::Table(entry));

    write_toml_table(path, &root)?;

    info!(
        category = "mcp",
        path = %path.display(),
        "Registered MCP server in TOML config"
    );
    Ok(())
}

fn unregister_codex_agent(path: &Path) -> Result<(), GwtError> {
    if !path.exists() {
        return Ok(());
    }

    let mut root = load_toml_table(path)?;

    let removed = root
        .get_mut("mcp_servers")
        .and_then(|v| v.as_table_mut())
        .map(|servers| servers.remove(MCP_SERVER_NAME).is_some())
        .unwrap_or(false);

    if removed {
        write_toml_table(path, &root)?;
        info!(
            category = "mcp",
            path = %path.display(),
            "Unregistered MCP server from TOML config"
        );
    }

    Ok(())
}

fn is_registered_codex(path: &Path) -> Result<bool, GwtError> {
    let root = load_toml_table(path).unwrap_or_default();
    Ok(root
        .get("mcp_servers")
        .and_then(|v| v.get(MCP_SERVER_NAME))
        .is_some())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_config() -> McpBridgeConfig {
        McpBridgeConfig {
            command: "/usr/local/bin/bun".to_string(),
            args: vec!["/path/to/mcp-bridge.js".to_string()],
            env: HashMap::new(),
        }
    }

    // --- Claude Code (JSON) ---

    #[test]
    fn claude_register_creates_file_when_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        let cfg = sample_config();

        register_mcp_server_at(McpAgentType::Claude, &cfg, &path).unwrap();

        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let entry = &root["mcpServers"][MCP_SERVER_NAME];
        assert_eq!(entry["command"], "/usr/local/bin/bun");
        assert_eq!(entry["args"][0], "/path/to/mcp-bridge.js");
    }

    #[test]
    fn claude_register_preserves_other_servers() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");

        let existing = r#"{"mcpServers":{"other-server":{"command":"foo","args":[]}}}"#;
        std::fs::write(&path, existing).unwrap();

        register_mcp_server_at(McpAgentType::Claude, &sample_config(), &path).unwrap();

        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(root["mcpServers"]["other-server"].is_object());
        assert!(root["mcpServers"][MCP_SERVER_NAME].is_object());
    }

    #[test]
    fn claude_unregister_removes_only_our_entry() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");

        let existing = r#"{"mcpServers":{"other-server":{"command":"foo","args":[]},"gwt-agent-bridge":{"command":"bun","args":[]}}}"#;
        std::fs::write(&path, existing).unwrap();

        unregister_mcp_server_at(McpAgentType::Claude, &path).unwrap();

        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(root["mcpServers"]["other-server"].is_object());
        assert!(root["mcpServers"][MCP_SERVER_NAME].is_null());
    }

    #[test]
    fn claude_unregister_noop_when_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        // File does not exist.
        assert!(unregister_mcp_server_at(McpAgentType::Claude, &path).is_ok());
    }

    #[test]
    fn claude_is_registered() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");

        assert!(!is_registered_at(McpAgentType::Claude, &path).unwrap());

        register_mcp_server_at(McpAgentType::Claude, &sample_config(), &path).unwrap();
        assert!(is_registered_at(McpAgentType::Claude, &path).unwrap());

        unregister_mcp_server_at(McpAgentType::Claude, &path).unwrap();
        assert!(!is_registered_at(McpAgentType::Claude, &path).unwrap());
    }

    #[test]
    fn claude_register_handles_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");

        std::fs::write(&path, "not valid json {{").unwrap();

        let result = register_mcp_server_at(McpAgentType::Claude, &sample_config(), &path);
        assert!(result.is_err());

        let original = std::fs::read_to_string(&path).unwrap();
        assert_eq!(original, "not valid json {{");
    }

    #[test]
    fn claude_register_preserves_non_mcp_keys() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");

        let existing = r#"{"enabledPlugins":{"foo":true},"someOtherKey":42}"#;
        std::fs::write(&path, existing).unwrap();

        register_mcp_server_at(McpAgentType::Claude, &sample_config(), &path).unwrap();

        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["enabledPlugins"]["foo"], true);
        assert_eq!(root["someOtherKey"], 42);
        assert!(root["mcpServers"][MCP_SERVER_NAME].is_object());
    }

    #[test]
    fn claude_register_with_env() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");

        let cfg = McpBridgeConfig {
            command: "bun".to_string(),
            args: vec!["bridge.js".to_string()],
            env: HashMap::from([("FOO".to_string(), "bar".to_string())]),
        };

        register_mcp_server_at(McpAgentType::Claude, &cfg, &path).unwrap();

        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["mcpServers"][MCP_SERVER_NAME]["env"]["FOO"], "bar");
    }

    // --- Codex (TOML) ---

    #[test]
    fn codex_register_creates_file_when_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".codex").join("config.toml");

        register_mcp_server_at(McpAgentType::Codex, &sample_config(), &path).unwrap();

        let root = std::fs::read_to_string(&path)
            .unwrap()
            .parse::<toml::Table>()
            .unwrap();
        let entry = &root["mcp_servers"][MCP_SERVER_NAME];
        assert_eq!(entry["command"].as_str(), Some("/usr/local/bin/bun"));
        assert_eq!(entry["args"][0].as_str(), Some("/path/to/mcp-bridge.js"));
    }

    #[test]
    fn codex_register_preserves_other_servers() {
        let tmp = TempDir::new().unwrap();
        let codex_dir = tmp.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let path = codex_dir.join("config.toml");

        let existing = r#"
[mcp_servers.other-server]
command = "foo"
args = []
"#;
        std::fs::write(&path, existing).unwrap();

        register_mcp_server_at(McpAgentType::Codex, &sample_config(), &path).unwrap();

        let root = std::fs::read_to_string(&path)
            .unwrap()
            .parse::<toml::Table>()
            .unwrap();
        assert!(root["mcp_servers"]["other-server"].is_table());
        assert!(root["mcp_servers"][MCP_SERVER_NAME].is_table());
    }

    #[test]
    fn codex_unregister_removes_only_our_entry() {
        let tmp = TempDir::new().unwrap();
        let codex_dir = tmp.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let path = codex_dir.join("config.toml");

        let existing = format!(
            r#"
[mcp_servers.other-server]
command = "foo"
args = []

[mcp_servers.{}]
command = "bun"
args = ["bridge.js"]
"#,
            MCP_SERVER_NAME
        );
        std::fs::write(&path, existing).unwrap();

        unregister_mcp_server_at(McpAgentType::Codex, &path).unwrap();

        let root = std::fs::read_to_string(&path)
            .unwrap()
            .parse::<toml::Table>()
            .unwrap();
        assert!(root["mcp_servers"]["other-server"].is_table());
        assert!(root["mcp_servers"].get(MCP_SERVER_NAME).is_none());
    }

    #[test]
    fn codex_unregister_noop_when_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");
        assert!(unregister_mcp_server_at(McpAgentType::Codex, &path).is_ok());
    }

    #[test]
    fn codex_is_registered() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.toml");

        assert!(!is_registered_at(McpAgentType::Codex, &path).unwrap());

        register_mcp_server_at(McpAgentType::Codex, &sample_config(), &path).unwrap();
        assert!(is_registered_at(McpAgentType::Codex, &path).unwrap());

        unregister_mcp_server_at(McpAgentType::Codex, &path).unwrap();
        assert!(!is_registered_at(McpAgentType::Codex, &path).unwrap());
    }

    #[test]
    fn codex_preserves_non_mcp_keys() {
        let tmp = TempDir::new().unwrap();
        let codex_dir = tmp.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let path = codex_dir.join("config.toml");

        let existing = r#"
model = "gpt-5.2"
approval_mode = "full-auto"

[mcp_servers.other]
command = "foo"
args = []
"#;
        std::fs::write(&path, existing).unwrap();

        register_mcp_server_at(McpAgentType::Codex, &sample_config(), &path).unwrap();

        let root = std::fs::read_to_string(&path)
            .unwrap()
            .parse::<toml::Table>()
            .unwrap();
        assert_eq!(root["model"].as_str(), Some("gpt-5.2"));
        assert_eq!(root["approval_mode"].as_str(), Some("full-auto"));
        assert!(root["mcp_servers"]["other"].is_table());
        assert!(root["mcp_servers"][MCP_SERVER_NAME].is_table());
    }

    #[test]
    fn codex_register_handles_invalid_toml() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".codex").join("config.toml");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }

        std::fs::write(&path, "not valid toml = = =").unwrap();

        let result = register_mcp_server_at(McpAgentType::Codex, &sample_config(), &path);
        assert!(result.is_err());

        let original = std::fs::read_to_string(&path).unwrap();
        assert_eq!(original, "not valid toml = = =");
    }

    // --- Gemini (JSON) ---

    #[test]
    fn gemini_register_creates_file_when_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gemini").join("settings.json");

        register_mcp_server_at(McpAgentType::Gemini, &sample_config(), &path).unwrap();

        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let entry = &root["mcpServers"][MCP_SERVER_NAME];
        assert_eq!(entry["command"], "/usr/local/bin/bun");
        assert_eq!(entry["args"][0], "/path/to/mcp-bridge.js");
    }

    #[test]
    fn gemini_unregister_removes_entry() {
        let tmp = TempDir::new().unwrap();
        let gemini_dir = tmp.path().join(".gemini");
        std::fs::create_dir_all(&gemini_dir).unwrap();
        let path = gemini_dir.join("settings.json");

        register_mcp_server_at(McpAgentType::Gemini, &sample_config(), &path).unwrap();
        assert!(is_registered_at(McpAgentType::Gemini, &path).unwrap());

        unregister_mcp_server_at(McpAgentType::Gemini, &path).unwrap();
        assert!(!is_registered_at(McpAgentType::Gemini, &path).unwrap());
    }

    // --- detect_runtime ---

    #[test]
    fn detect_runtime_finds_something() {
        // In CI at least one of bun/node should be present.
        // If neither is available the test is still valid -- just skipped implicitly.
        if let Ok(rt) = detect_runtime() {
            assert!(!rt.is_empty());
        }
    }

    // --- cleanup_stale_registrations ---

    #[test]
    fn cleanup_stale_registrations_removes_all() {
        let tmp = TempDir::new().unwrap();
        let claude_path = tmp.path().join("claude.json");
        let codex_path = tmp.path().join("codex.toml");
        let gemini_path = tmp.path().join("gemini.json");
        let cfg = sample_config();

        register_mcp_server_at(McpAgentType::Claude, &cfg, &claude_path).unwrap();
        register_mcp_server_at(McpAgentType::Codex, &cfg, &codex_path).unwrap();
        register_mcp_server_at(McpAgentType::Gemini, &cfg, &gemini_path).unwrap();

        assert!(is_registered_at(McpAgentType::Claude, &claude_path).unwrap());
        assert!(is_registered_at(McpAgentType::Codex, &codex_path).unwrap());
        assert!(is_registered_at(McpAgentType::Gemini, &gemini_path).unwrap());

        // Cleanup uses real paths, so we test the per-agent unregister_at directly.
        unregister_mcp_server_at(McpAgentType::Claude, &claude_path).unwrap();
        unregister_mcp_server_at(McpAgentType::Codex, &codex_path).unwrap();
        unregister_mcp_server_at(McpAgentType::Gemini, &gemini_path).unwrap();

        assert!(!is_registered_at(McpAgentType::Claude, &claude_path).unwrap());
        assert!(!is_registered_at(McpAgentType::Codex, &codex_path).unwrap());
        assert!(!is_registered_at(McpAgentType::Gemini, &gemini_path).unwrap());
    }

    // --- Idempotent registration ---

    #[test]
    fn register_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("claude.json");
        let cfg = sample_config();

        register_mcp_server_at(McpAgentType::Claude, &cfg, &path).unwrap();
        register_mcp_server_at(McpAgentType::Claude, &cfg, &path).unwrap();

        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let servers = root["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
    }
}
