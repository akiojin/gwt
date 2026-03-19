//! File-based inbox/outbox for Agent↔Assistant consultation.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsultationRequest {
    pub pane_id: String,
    pub agent_name: String,
    pub timestamp: String,
    pub status: String, // "waiting" or "responded"
    pub question: String,
    pub context: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsultationResponse {
    pub pane_id: String,
    pub timestamp: String,
    pub response: String,
}

fn inbox_dir(project_root: &Path) -> PathBuf {
    project_root.join(".gwt").join("assistant").join("inbox")
}

fn outbox_dir(project_root: &Path) -> PathBuf {
    project_root.join(".gwt").join("assistant").join("outbox")
}

pub fn list_pending_consultations(project_root: &Path) -> Result<Vec<ConsultationRequest>, String> {
    let dir = inbox_dir(project_root);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut results = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("Failed to read inbox: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(req) = parse_consultation_request(&content) {
                if req.status == "waiting" {
                    results.push(req);
                }
            }
        }
    }
    results.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    Ok(results)
}

pub fn count_pending_consultations(project_root: &Path) -> u32 {
    list_pending_consultations(project_root)
        .map(|list| list.len() as u32)
        .unwrap_or(0)
}

pub fn read_consultation(
    project_root: &Path,
    pane_id: &str,
    timestamp: &str,
) -> Result<ConsultationRequest, String> {
    let filename = format!("{}_{}.md", pane_id, timestamp);
    let path = inbox_dir(project_root).join(&filename);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read consultation {filename}: {e}"))?;
    parse_consultation_request(&content)
        .ok_or_else(|| format!("Failed to parse consultation {filename}"))
}

pub fn write_consultation_request(
    project_root: &Path,
    pane_id: &str,
    agent_name: &str,
    question: &str,
    context: &str,
) -> Result<String, String> {
    let dir = inbox_dir(project_root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create inbox dir: {e}"))?;
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let filename = format!("{}_{}.md", pane_id, timestamp);
    let content = format!(
        "---\npane_id: {pane_id}\nagent_name: {agent_name}\ntimestamp: {timestamp}\nstatus: waiting\n---\n\n## Question\n{question}\n\n## Context\n{context}\n"
    );
    std::fs::write(dir.join(&filename), &content)
        .map_err(|e| format!("Failed to write consultation: {e}"))?;
    Ok(timestamp)
}

pub fn write_consultation_response(
    project_root: &Path,
    pane_id: &str,
    timestamp: &str,
    response: &str,
) -> Result<(), String> {
    // Write response to outbox
    let outbox = outbox_dir(project_root);
    std::fs::create_dir_all(&outbox).map_err(|e| format!("Failed to create outbox dir: {e}"))?;
    let response_filename = format!("{}_{}_response.md", pane_id, timestamp);
    let response_content = format!(
        "---\npane_id: {pane_id}\ntimestamp: {timestamp}\n---\n\n## Response\n{response}\n"
    );
    std::fs::write(outbox.join(&response_filename), &response_content)
        .map_err(|e| format!("Failed to write response: {e}"))?;

    // Update inbox status to "responded"
    let inbox_filename = format!("{}_{}.md", pane_id, timestamp);
    let inbox_path = inbox_dir(project_root).join(&inbox_filename);
    if let Ok(content) = std::fs::read_to_string(&inbox_path) {
        let updated = content.replace("status: waiting", "status: responded");
        let _ = std::fs::write(&inbox_path, updated);
    }
    Ok(())
}

pub fn check_consultation_response(
    project_root: &Path,
    pane_id: &str,
) -> Result<Option<String>, String> {
    let outbox = outbox_dir(project_root);
    if !outbox.exists() {
        return Ok(None);
    }
    let prefix = format!("{}_", pane_id);
    let mut latest: Option<(String, String)> = None;
    let entries = std::fs::read_dir(&outbox).map_err(|e| format!("Failed to read outbox: {e}"))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) && name.ends_with("_response.md") {
            let content = std::fs::read_to_string(entry.path())
                .map_err(|e| format!("Failed to read response: {e}"))?;
            match &latest {
                Some((existing_name, _)) if name > *existing_name => {
                    latest = Some((name, content));
                }
                None => {
                    latest = Some((name, content));
                }
                _ => {}
            }
        }
    }
    Ok(latest.map(|(_, content)| content))
}

fn parse_consultation_request(content: &str) -> Option<ConsultationRequest> {
    // Parse YAML frontmatter between --- markers
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return None;
    }
    let frontmatter = parts[1].trim();
    let body = parts[2];

    let mut pane_id = String::new();
    let mut agent_name = String::new();
    let mut timestamp = String::new();
    let mut status = String::new();

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("pane_id:") {
            pane_id = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("agent_name:") {
            agent_name = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("timestamp:") {
            timestamp = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("status:") {
            status = value.trim().to_string();
        }
    }

    let question = extract_section(body, "## Question").unwrap_or_default();
    let context = extract_section(body, "## Context").unwrap_or_default();

    Some(ConsultationRequest {
        pane_id,
        agent_name,
        timestamp,
        status,
        question,
        context,
    })
}

fn extract_section(body: &str, header: &str) -> Option<String> {
    let start = body.find(header)?;
    let after_header = &body[start + header.len()..];
    let content = if let Some(next_header) = after_header.find("\n## ") {
        &after_header[..next_header]
    } else {
        after_header
    };
    Some(content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_consultation_request_roundtrip() {
        let content = "---\npane_id: pane-1\nagent_name: claude\ntimestamp: 20260319T120000Z\nstatus: waiting\n---\n\n## Question\nHow should I handle this?\n\n## Context\nWorking on feature X\n";
        let req = parse_consultation_request(content).unwrap();
        assert_eq!(req.pane_id, "pane-1");
        assert_eq!(req.agent_name, "claude");
        assert_eq!(req.status, "waiting");
        assert_eq!(req.question, "How should I handle this?");
        assert_eq!(req.context, "Working on feature X");
    }

    #[test]
    fn count_pending_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(count_pending_consultations(dir.path()), 0);
    }

    #[test]
    fn write_and_list_consultation() {
        let dir = tempfile::tempdir().unwrap();
        write_consultation_request(dir.path(), "pane-1", "claude", "Help?", "Context here")
            .unwrap();
        let pending = list_pending_consultations(dir.path()).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].pane_id, "pane-1");
        assert_eq!(pending[0].status, "waiting");
    }

    #[test]
    fn respond_updates_inbox_status() {
        let dir = tempfile::tempdir().unwrap();
        let ts =
            write_consultation_request(dir.path(), "pane-1", "claude", "Help?", "Ctx").unwrap();
        write_consultation_response(dir.path(), "pane-1", &ts, "Here's the answer").unwrap();

        // Inbox should now be "responded"
        let pending = list_pending_consultations(dir.path()).unwrap();
        assert_eq!(pending.len(), 0);

        // Outbox should have response
        let response = check_consultation_response(dir.path(), "pane-1").unwrap();
        assert!(response.is_some());
        assert!(response.unwrap().contains("Here's the answer"));
    }
}
