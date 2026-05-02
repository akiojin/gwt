//! Custom coding agent definitions loaded from user configuration.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Agent execution type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CustomAgentType {
    /// Execute via PATH search
    #[default]
    Command,
    /// Execute via absolute path
    Path,
    /// Execute via bunx
    Bunx,
}

/// Mode-specific arguments for different session modes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeArgs {
    pub normal: Vec<String>,
    #[serde(rename = "continue")]
    pub continue_mode: Vec<String>,
    pub resume: Vec<String>,
}

/// A user-defined coding agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCodingAgent {
    pub id: String,
    pub display_name: String,
    #[serde(rename = "type")]
    pub agent_type: CustomAgentType,
    pub command: String,
    #[serde(default)]
    pub default_args: Vec<String>,
    #[serde(default)]
    pub mode_args: Option<ModeArgs>,
    #[serde(default)]
    pub skip_permissions_args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl CustomCodingAgent {
    /// Validate that required fields are present and well-formed.
    pub fn validate(&self) -> bool {
        if self.id.is_empty() || self.display_name.is_empty() || self.command.is_empty() {
            return false;
        }
        self.id.chars().all(|c| c.is_alphanumeric() || c == '-')
    }

    /// Build the command and args for a given session mode.
    pub fn build_args(&self, mode: crate::types::SessionMode) -> Vec<String> {
        let mut args = self.default_args.clone();
        if let Some(ref ma) = self.mode_args {
            match mode {
                crate::types::SessionMode::Normal => args.extend(ma.normal.clone()),
                crate::types::SessionMode::Continue => args.extend(ma.continue_mode.clone()),
                crate::types::SessionMode::Resume => args.extend(ma.resume.clone()),
            }
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SessionMode;

    fn sample_agent() -> CustomCodingAgent {
        CustomCodingAgent {
            id: "test-agent".to_string(),
            display_name: "Test Agent".to_string(),
            agent_type: CustomAgentType::Command,
            command: "test-cmd".to_string(),
            default_args: vec!["--flag".to_string()],
            mode_args: Some(ModeArgs {
                normal: vec![],
                continue_mode: vec!["--continue".to_string()],
                resume: vec!["--resume".to_string()],
            }),
            skip_permissions_args: vec!["--yolo".to_string()],
            env: HashMap::from([("KEY".to_string(), "VALUE".to_string())]),
        }
    }

    #[test]
    fn validate_valid_agent() {
        assert!(sample_agent().validate());
    }

    #[test]
    fn validate_empty_id() {
        let mut a = sample_agent();
        a.id = String::new();
        assert!(!a.validate());
    }

    #[test]
    fn validate_empty_display_name() {
        let mut a = sample_agent();
        a.display_name = String::new();
        assert!(!a.validate());
    }

    #[test]
    fn validate_empty_command() {
        let mut a = sample_agent();
        a.command = String::new();
        assert!(!a.validate());
    }

    #[test]
    fn validate_invalid_id_chars() {
        let mut a = sample_agent();
        a.id = "has spaces".to_string();
        assert!(!a.validate());
    }

    #[test]
    fn build_args_normal() {
        let agent = sample_agent();
        let args = agent.build_args(SessionMode::Normal);
        assert_eq!(args, vec!["--flag"]);
    }

    #[test]
    fn build_args_continue() {
        let agent = sample_agent();
        let args = agent.build_args(SessionMode::Continue);
        assert_eq!(args, vec!["--flag", "--continue"]);
    }

    #[test]
    fn build_args_resume() {
        let agent = sample_agent();
        let args = agent.build_args(SessionMode::Resume);
        assert_eq!(args, vec!["--flag", "--resume"]);
    }

    #[test]
    fn build_args_no_mode_args() {
        let mut agent = sample_agent();
        agent.mode_args = None;
        let args = agent.build_args(SessionMode::Continue);
        assert_eq!(args, vec!["--flag"]);
    }

    #[test]
    fn serde_roundtrip() {
        let agent = sample_agent();
        let json = serde_json::to_string(&agent).unwrap();
        let parsed: CustomCodingAgent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, agent.id);
        assert_eq!(parsed.agent_type, agent.agent_type);
        assert_eq!(parsed.skip_permissions_args, agent.skip_permissions_args);
    }

    #[test]
    fn agent_type_serde() {
        assert_eq!(
            serde_json::to_string(&CustomAgentType::Command).unwrap(),
            "\"command\""
        );
        assert_eq!(
            serde_json::to_string(&CustomAgentType::Path).unwrap(),
            "\"path\""
        );
        assert_eq!(
            serde_json::to_string(&CustomAgentType::Bunx).unwrap(),
            "\"bunx\""
        );
    }

    #[test]
    fn toml_roundtrip_preserves_env_subtable() {
        let mut agent = sample_agent();
        agent.env.clear();
        agent
            .env
            .insert("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string());
        agent.env.insert(
            "ANTHROPIC_BASE_URL".to_string(),
            "http://proxy.local:32768".to_string(),
        );
        agent.env.insert(
            "ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(),
            "openai/gpt-oss-20b".to_string(),
        );

        let toml_text = toml::to_string(&agent).expect("serialize to TOML");
        let decoded: CustomCodingAgent = toml::from_str(&toml_text).expect("deserialize TOML");

        assert_eq!(decoded.id, agent.id);
        assert_eq!(decoded.env.len(), 3);
        assert_eq!(decoded.env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test");
        assert_eq!(
            decoded.env.get("ANTHROPIC_BASE_URL").unwrap(),
            "http://proxy.local:32768"
        );
        assert_eq!(
            decoded.env.get("ANTHROPIC_DEFAULT_OPUS_MODEL").unwrap(),
            "openai/gpt-oss-20b"
        );
    }

    #[test]
    fn toml_without_env_deserializes_with_empty_map() {
        // Legacy custom agent TOML without an [env] table must remain
        // readable (FR-059: backwards-compatible with existing custom agent rows).
        let toml_text = r#"
id = "legacy-agent"
display_name = "Legacy"
type = "command"
command = "legacy-cli"
"#;
        let decoded: CustomCodingAgent =
            toml::from_str(toml_text).expect("legacy TOML deserializes");
        assert_eq!(decoded.id, "legacy-agent");
        assert!(
            decoded.env.is_empty(),
            "missing env sub-table must default to empty map"
        );
    }

    #[test]
    fn toml_env_roundtrip_is_stable_across_multiple_cycles() {
        let mut agent = sample_agent();
        agent.env.clear();
        for i in 0..10 {
            agent.env.insert(format!("KEY_{i}"), format!("value_{i}"));
        }
        let t1 = toml::to_string(&agent).unwrap();
        let decoded1: CustomCodingAgent = toml::from_str(&t1).unwrap();
        let t2 = toml::to_string(&decoded1).unwrap();
        let decoded2: CustomCodingAgent = toml::from_str(&t2).unwrap();
        assert_eq!(decoded2.env.len(), 10);
        for i in 0..10 {
            assert_eq!(
                decoded2.env.get(&format!("KEY_{i}")).unwrap(),
                &format!("value_{i}")
            );
        }
    }
}
