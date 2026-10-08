use super::{same_path, PmConversationMessage, PmConversationRole};
use serde_json::Value;
use std::path::Path;

pub(super) fn message(
    value: &Value,
    id: &str,
    cwd: &Path,
) -> Result<Option<PmConversationMessage>, &'static str> {
    let kind = value["type"].as_str();
    if !matches!(kind, Some("user" | "assistant")) {
        return Ok(None);
    }
    if value["isSidechain"].as_bool() == Some(true) {
        return Ok(None);
    }
    if value["isMeta"].as_bool() == Some(true)
        || value["isCompactSummary"].as_bool() == Some(true)
        || value["promptSource"].as_str() == Some("system")
        || value["origin"]["kind"]
            .as_str()
            .is_some_and(|kind| kind != "human")
    {
        return Ok(None);
    }
    if value["sessionId"].as_str() != Some(id)
        || !value["cwd"]
            .as_str()
            .is_some_and(|path| same_path(Path::new(path), cwd))
    {
        return Err("Conversation identity does not match this PM session.");
    }
    let message = &value["message"];
    if message["role"].as_str() != kind {
        return Ok(None);
    }
    let role = if kind == Some("user") {
        PmConversationRole::User
    } else {
        PmConversationRole::Assistant
    };
    let text = match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return Ok(None),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let uuid = value["uuid"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Conversation message identity is missing.")?;
    // UUID identifies a transcript record. Message IDs are shared by thinking,
    // tool, and text blocks and cannot be used to discard a whole message.
    Ok(Some(PmConversationMessage {
        id: uuid.to_owned(),
        role,
        text,
    }))
}
