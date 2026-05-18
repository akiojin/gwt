//! TOML persistence for Backend Override profiles (SPEC-1921 FR-098).
//!
//! Backend profiles live under `[builtinAgents.<agent>.backends.<id>]` in the
//! gwt global config TOML (`~/.gwt/config.toml` by default). This module
//! preserves unrelated root-level tables (notably
//! `[tools.customCodingAgents.*]` reserved for External Agents) on every
//! load → save round-trip.

use std::path::Path;

use gwt_config::atomic::write_atomic as write_atomic_shared;
use toml::{Table, Value};
use tracing::warn;

use crate::backend::{AgentBackendProfile, BuiltinAgentId};

/// Load every saved backend profile for the given built-in agent. Returns an
/// empty vector when the config file does not exist or has no matching
/// section.
pub fn load_backends_for_agent(
    path: &Path,
    agent: BuiltinAgentId,
) -> Result<Vec<AgentBackendProfile>, String> {
    let root = load_root_document(path)?;
    let Some(backends) = backends_table(&root, agent) else {
        return Ok(Vec::new());
    };

    let mut profiles = Vec::with_capacity(backends.len());
    for (key, raw_value) in backends {
        let Some(_table) = raw_value.as_table() else {
            warn!(agent = agent.as_str(), backend = %key, "skipping non-table backend entry");
            continue;
        };
        let parsed: AgentBackendProfile = match raw_value.clone().try_into() {
            Ok(parsed) => parsed,
            Err(err) => {
                warn!(
                    agent = agent.as_str(),
                    backend = %key,
                    error = %err,
                    "skipping unparsable backend entry"
                );
                continue;
            }
        };
        let mut profile = parsed;
        if profile.id.trim().is_empty() {
            profile.id = key.clone();
        }
        if let Err(err) = profile.validate(agent) {
            warn!(
                agent = agent.as_str(),
                backend = %key,
                error = err,
                "skipping invalid backend entry"
            );
            continue;
        }
        profiles.push(profile);
    }
    profiles.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(profiles)
}

/// Save every backend profile for the given built-in agent, replacing any
/// previous entries. Preserves the other agent's backends, root-level
/// sibling tables (`debug = true`, etc.), and the External Agent section
/// (`[tools.customCodingAgents.*]`).
pub fn save_backends_for_agent(
    path: &Path,
    agent: BuiltinAgentId,
    profiles: &[AgentBackendProfile],
) -> Result<(), String> {
    validate_unique_ids(profiles, agent)?;

    let mut root = load_root_document(path)?;
    let root_table = root
        .as_table_mut()
        .ok_or_else(|| format!("config {} must contain a TOML table root", path.display()))?;

    let builtin_entry = root_table
        .entry("builtinAgents".to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    let builtin_table = builtin_entry.as_table_mut().ok_or_else(|| {
        format!(
            "config {} has a non-table [builtinAgents] section",
            path.display()
        )
    })?;

    let agent_entry = builtin_table
        .entry(agent.as_str().to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    let agent_table = agent_entry.as_table_mut().ok_or_else(|| {
        format!(
            "config {} has a non-table [builtinAgents.{}] section",
            path.display(),
            agent.as_str()
        )
    })?;

    if profiles.is_empty() {
        agent_table.remove("backends");
    } else {
        let mut backends_table = Table::new();
        for profile in profiles {
            backends_table.insert(
                profile.id.clone(),
                Value::try_from(profile).map_err(|err| {
                    format!(
                        "failed to serialize backend {} for {}: {err}",
                        profile.id,
                        agent.as_str()
                    )
                })?,
            );
        }
        agent_table.insert("backends".to_string(), Value::Table(backends_table));
    }

    if agent_table.is_empty() {
        builtin_table.remove(agent.as_str());
    }
    if builtin_table.is_empty() {
        root_table.remove("builtinAgents");
    }

    let content = toml::to_string_pretty(&root)
        .map_err(|err| format!("failed to serialize config {}: {err}", path.display()))?;
    write_atomic_shared(path, &content)
        .map_err(|err| format!("failed to write config {}: {err}", path.display()))
}

/// Insert a single backend profile, rejecting duplicate ids within the agent.
pub fn add_backend(
    path: &Path,
    agent: BuiltinAgentId,
    profile: AgentBackendProfile,
) -> Result<(), String> {
    profile.validate(agent).map_err(|err| err.to_string())?;
    let mut profiles = load_backends_for_agent(path, agent)?;
    if profiles.iter().any(|p| p.id == profile.id) {
        return Err(format!(
            "backend id `{}` already exists for {}",
            profile.id,
            agent.as_str()
        ));
    }
    profiles.push(profile);
    save_backends_for_agent(path, agent, &profiles)
}

/// Replace an existing backend profile by id.
pub fn update_backend(
    path: &Path,
    agent: BuiltinAgentId,
    id: &str,
    patch: AgentBackendProfile,
) -> Result<(), String> {
    if patch.id != id {
        return Err(format!(
            "patch id `{}` does not match target id `{}`",
            patch.id, id
        ));
    }
    patch.validate(agent).map_err(|err| err.to_string())?;
    let mut profiles = load_backends_for_agent(path, agent)?;
    let mut found = false;
    for entry in profiles.iter_mut() {
        if entry.id == id {
            *entry = patch.clone();
            found = true;
            break;
        }
    }
    if !found {
        return Err(format!(
            "backend id `{}` not found for {}",
            id,
            agent.as_str()
        ));
    }
    save_backends_for_agent(path, agent, &profiles)
}

/// Remove a backend profile by id. Returns `Ok(false)` when the id was not
/// present, so callers can treat repeat deletions as no-ops.
pub fn delete_backend(path: &Path, agent: BuiltinAgentId, id: &str) -> Result<bool, String> {
    let mut profiles = load_backends_for_agent(path, agent)?;
    let before = profiles.len();
    profiles.retain(|p| p.id != id);
    let removed = profiles.len() != before;
    if removed {
        save_backends_for_agent(path, agent, &profiles)?;
    }
    Ok(removed)
}

fn validate_unique_ids(
    profiles: &[AgentBackendProfile],
    agent: BuiltinAgentId,
) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for profile in profiles {
        profile.validate(agent).map_err(|err| {
            format!(
                "invalid backend profile {} for {}: {err}",
                profile.id,
                agent.as_str()
            )
        })?;
        if !seen.insert(profile.id.clone()) {
            return Err(format!(
                "duplicate backend id `{}` for {}",
                profile.id,
                agent.as_str()
            ));
        }
    }
    Ok(())
}

fn backends_table(root: &Value, agent: BuiltinAgentId) -> Option<&Table> {
    let builtin = root.get("builtinAgents")?.as_table()?;
    let agent_section = builtin.get(agent.as_str())?.as_table()?;
    agent_section.get("backends")?.as_table()
}

fn load_root_document(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Table(Table::new()));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read config {}: {err}", path.display()))?;
    toml::from_str(&content)
        .map_err(|err| format!("failed to parse config {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cc_profile(id: &str) -> AgentBackendProfile {
        AgentBackendProfile {
            id: id.into(),
            display_name: "LM Studio".into(),
            base_url: "http://127.0.0.1:1234".into(),
            api_key: "sk-test".into(),
            model: "openai/gpt-oss-20b".into(),
            ..Default::default()
        }
    }

    fn codex_profile(id: &str) -> AgentBackendProfile {
        AgentBackendProfile {
            id: id.into(),
            display_name: "llmlb".into(),
            base_url: "http://127.0.0.1:8080".into(),
            api_key: "sk-codex".into(),
            model: "local/qwen3-coder".into(),
            ..Default::default()
        }
    }

    #[test]
    fn load_returns_empty_when_file_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing.toml");
        let claude = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load");
        let codex = load_backends_for_agent(&path, BuiltinAgentId::Codex).expect("load");
        assert!(claude.is_empty());
        assert!(codex.is_empty());
    }

    #[test]
    fn load_returns_empty_when_section_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "debug = true\n").expect("write");
        let claude = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load");
        assert!(claude.is_empty());
    }

    #[test]
    fn save_and_load_round_trip_claude_code() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        let mut profile = cc_profile("lmstudio");
        profile.opus_model = Some("openai/gpt-oss-120b".into());

        save_backends_for_agent(&path, BuiltinAgentId::ClaudeCode, &[profile.clone()])
            .expect("save");

        let content = std::fs::read_to_string(&path).expect("read");
        // SPEC-1921 FR-098: canonical camelCase section path.
        assert!(content.contains("[builtinAgents.claudeCode.backends.lmstudio]"));
        assert!(content.contains("displayName = \"LM Studio\""));
        assert!(content.contains("opusModel = \"openai/gpt-oss-120b\""));

        let loaded = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load");
        assert_eq!(loaded, vec![profile]);
        // Codex section is untouched.
        let codex = load_backends_for_agent(&path, BuiltinAgentId::Codex).expect("load codex");
        assert!(codex.is_empty());
    }

    #[test]
    fn save_and_load_round_trip_codex_with_headers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        let mut profile = codex_profile("llmlb");
        profile.wire_api = Some("responses".into());
        let mut headers = HashMap::new();
        headers.insert("X-Bearer".into(), "v".into());
        profile.http_headers = headers;

        save_backends_for_agent(&path, BuiltinAgentId::Codex, &[profile.clone()]).expect("save");

        let content = std::fs::read_to_string(&path).expect("read");
        assert!(content.contains("[builtinAgents.codex.backends.llmlb]"));
        assert!(content.contains("wireApi = \"responses\""));
        assert!(content.contains("[builtinAgents.codex.backends.llmlb.httpHeaders]"));

        let loaded = load_backends_for_agent(&path, BuiltinAgentId::Codex).expect("load");
        assert_eq!(loaded, vec![profile]);
    }

    #[test]
    fn save_preserves_sibling_root_tables_and_custom_agents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
debug = true

[tools.customCodingAgents.aider]
id = "aider"
displayName = "Aider"
agentType = "command"
command = "aider"
"#,
        )
        .expect("write fixture");

        save_backends_for_agent(&path, BuiltinAgentId::ClaudeCode, &[cc_profile("lmstudio")])
            .expect("save");

        let content = std::fs::read_to_string(&path).expect("read");
        assert!(content.contains("debug = true"));
        // External Agent section MUST survive (FR-088 reword).
        assert!(content.contains("[tools.customCodingAgents.aider]"));
        assert!(content.contains("command = \"aider\""));
        // New section was added.
        assert!(content.contains("[builtinAgents.claudeCode.backends.lmstudio]"));
    }

    #[test]
    fn save_preserves_other_agent_backends() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        save_backends_for_agent(&path, BuiltinAgentId::Codex, &[codex_profile("llmlb")])
            .expect("seed codex");
        save_backends_for_agent(&path, BuiltinAgentId::ClaudeCode, &[cc_profile("lmstudio")])
            .expect("seed claude");

        let codex = load_backends_for_agent(&path, BuiltinAgentId::Codex).expect("load codex");
        let claude = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load cc");
        assert_eq!(codex.len(), 1);
        assert_eq!(claude.len(), 1);
        assert_eq!(codex[0].id, "llmlb");
        assert_eq!(claude[0].id, "lmstudio");
    }

    #[test]
    fn save_empty_removes_section_but_keeps_other_root_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
debug = true

[builtinAgents.claudeCode.backends.to-remove]
id = "to-remove"
displayName = "Old"
baseUrl = "http://x"
apiKey = "k"
model = "m"
"#,
        )
        .expect("write");

        save_backends_for_agent(&path, BuiltinAgentId::ClaudeCode, &[]).expect("save empty");

        let content = std::fs::read_to_string(&path).expect("read");
        assert!(content.contains("debug = true"));
        assert!(!content.contains("to-remove"));
        assert!(!content.contains("builtinAgents"));
    }

    #[test]
    fn save_rejects_duplicate_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let err = save_backends_for_agent(
            &path,
            BuiltinAgentId::ClaudeCode,
            &[cc_profile("dup"), cc_profile("dup")],
        )
        .unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn save_rejects_invalid_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let mut bad = cc_profile("x");
        bad.id = "BAD ID".into();
        let err = save_backends_for_agent(&path, BuiltinAgentId::ClaudeCode, &[bad]).unwrap_err();
        assert!(err.contains("invalid"));
    }

    #[test]
    fn add_backend_rejects_duplicate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        add_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("a")).expect("first add");
        let err = add_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("a")).unwrap_err();
        assert!(err.contains("already exists"));
    }

    #[test]
    fn update_backend_replaces_existing_profile() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        add_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("lmstudio")).expect("seed");

        let mut patch = cc_profile("lmstudio");
        patch.display_name = "Updated Studio".into();
        patch.opus_model = Some("openai/gpt-oss-120b".into());
        update_backend(&path, BuiltinAgentId::ClaudeCode, "lmstudio", patch).expect("update");

        let loaded = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].display_name, "Updated Studio");
        assert_eq!(loaded[0].opus_model.as_deref(), Some("openai/gpt-oss-120b"));
    }

    #[test]
    fn update_backend_rejects_id_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        add_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("lmstudio")).expect("seed");

        let patch = cc_profile("renamed");
        let err = update_backend(&path, BuiltinAgentId::ClaudeCode, "lmstudio", patch).unwrap_err();
        assert!(err.contains("does not match"));
    }

    #[test]
    fn update_backend_rejects_missing_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let err = update_backend(
            &path,
            BuiltinAgentId::ClaudeCode,
            "ghost",
            cc_profile("ghost"),
        )
        .unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn delete_backend_returns_false_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let removed =
            delete_backend(&path, BuiltinAgentId::ClaudeCode, "ghost").expect("delete missing");
        assert!(!removed);
    }

    #[test]
    fn delete_backend_removes_existing_profile() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        add_backend(&path, BuiltinAgentId::ClaudeCode, cc_profile("lmstudio")).expect("seed");
        let removed =
            delete_backend(&path, BuiltinAgentId::ClaudeCode, "lmstudio").expect("delete existing");
        assert!(removed);
        let loaded = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load");
        assert!(loaded.is_empty());
    }

    #[test]
    fn load_skips_invalid_entries_silently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[builtinAgents.claudeCode.backends.ok]
id = "ok"
displayName = "OK"
baseUrl = "http://127.0.0.1:1234"
apiKey = "k"
model = "m"

[builtinAgents.claudeCode.backends.bad]
id = "bad"
displayName = ""
baseUrl = "ftp://nope"
apiKey = "k"
model = "m"
"#,
        )
        .expect("write");
        let loaded = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "ok");
    }

    #[test]
    fn load_uses_section_key_when_id_field_is_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[builtinAgents.claudeCode.backends.from-key]
displayName = "From Key"
baseUrl = "http://127.0.0.1:1234"
apiKey = "k"
model = "m"
"#,
        )
        .expect("write");
        let loaded = load_backends_for_agent(&path, BuiltinAgentId::ClaudeCode).expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "from-key");
    }
}
