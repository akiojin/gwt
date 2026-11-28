use super::{same_path, PmConversationMessage, PmConversationRole};
use gwt_skills::pm_guidance::REPORTS_ONLY_RESPONSE;
use serde_json::Value;
use std::path::Path;

pub(super) fn message(
    value: &Value,
    id: &str,
    cwd: &Path,
) -> Result<Option<PmConversationMessage>, &'static str> {
    let kind = value["type"].as_str();
    if kind == Some("attachment") {
        return queued_owner_message(value, id, cwd);
    }
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
    verify_identity(value, id, cwd)?;
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
    if text.trim().is_empty()
        || (role == PmConversationRole::Assistant && text.trim() == REPORTS_ONLY_RESPONSE)
    {
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
        timestamp: value["timestamp"].as_str().map(str::to_owned),
        source_key: None,
    }))
}

fn verify_identity(value: &Value, id: &str, cwd: &Path) -> Result<(), &'static str> {
    if value["sessionId"].as_str() != Some(id)
        || !value["cwd"]
            .as_str()
            .is_some_and(|path| same_path(Path::new(path), cwd))
    {
        return Err("Conversation identity does not match this PM session.");
    }
    Ok(())
}

/// Issue #8086: a message the owner submits while the PM is mid-turn is
/// absorbed into that turn and recorded only as a human-turn
/// `queued_command` attachment, never as a `user` record. Generated queued
/// commands (task notifications, scheduled wakes) carry no human turn.
fn queued_owner_message(
    value: &Value,
    id: &str,
    cwd: &Path,
) -> Result<Option<PmConversationMessage>, &'static str> {
    let attachment = &value["attachment"];
    if attachment["type"] != "queued_command"
        || attachment["humanTurn"].as_bool() != Some(true)
        || attachment["origin"]["kind"].as_str() != Some("human")
        || attachment["commandMode"]
            .as_str()
            .is_some_and(|mode| mode != "prompt")
        || value["isSidechain"].as_bool() == Some(true)
    {
        return Ok(None);
    }
    verify_identity(value, id, cwd)?;
    let Some(text) = attachment["prompt"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
    else {
        return Ok(None);
    };
    let uuid = value["uuid"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Conversation message identity is missing.")?;
    Ok(Some(PmConversationMessage {
        id: uuid.to_owned(),
        role: PmConversationRole::User,
        text: text.to_owned(),
        timestamp: attachment["timestamp"]
            .as_str()
            .or_else(|| value["timestamp"].as_str())
            .map(str::to_owned),
        source_key: None,
    }))
}
