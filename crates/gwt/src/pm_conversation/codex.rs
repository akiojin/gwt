use super::{same_path, PmConversationMessage, PmConversationRole};
use serde_json::Value;
use std::path::Path;

pub(super) fn validate_identity(value: &Value, id: &str, cwd: &Path) -> Result<bool, &'static str> {
    if value["type"] != "session_meta" {
        return Ok(false);
    }
    let meta = &value["payload"];
    if meta["id"].as_str() != Some(id)
        || !meta["cwd"]
            .as_str()
            .is_some_and(|path| same_path(Path::new(path), cwd))
    {
        return Err("Conversation identity does not match this PM session.");
    }
    Ok(true)
}

pub(super) fn message(value: &Value) -> Result<Option<PmConversationMessage>, &'static str> {
    let payload = &value["payload"];
    if value["type"] != "response_item" || payload["type"] != "message" {
        return Ok(None);
    }
    let (role, block_type) = match payload["role"].as_str() {
        Some("user") => (PmConversationRole::User, "input_text"),
        Some("assistant") => (PmConversationRole::Assistant, "output_text"),
        _ => return Ok(None),
    };
    // The response item is authoritative; event_msg may repeat its content.
    // Analysis and reasoning are never part of the public conversation.
    if payload["phase"]
        .as_str()
        .is_some_and(|phase| !matches!(phase, "commentary" | "final" | "final_answer"))
        || payload["channel"]
            .as_str()
            .is_some_and(|channel| !matches!(channel, "commentary" | "final"))
    {
        return Ok(None);
    }
    let Some(blocks) = payload["content"].as_array() else {
        return Ok(None);
    };
    let text = blocks
        .iter()
        .filter(|block| block["type"] == block_type)
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return Ok(None);
    }
    let id = payload["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Conversation message identity is missing.")?;
    Ok(Some(PmConversationMessage {
        id: id.to_owned(),
        role,
        text,
    }))
}
