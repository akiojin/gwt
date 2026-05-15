use std::{
    io::Write as _,
    path::{Path, PathBuf},
};

use gwt::{UiTraceEntry, UiTracePayload};

#[derive(Debug)]
pub(super) struct UiTraceSaveResult {
    pub(super) path: PathBuf,
    pub(super) entries: usize,
}

pub(super) fn save_ui_trace_to_log_dir(
    log_dir: &Path,
    trace: UiTracePayload,
) -> Result<UiTraceSaveResult, String> {
    let entries = trace.entries().map_err(str::to_string)?;
    if entries.len() > 5_000 {
        return Err("trace payload has too many entries".to_string());
    }
    std::fs::create_dir_all(log_dir)
        .map_err(|error| format!("failed to create trace log directory: {error}"))?;

    let session_id = trace
        .session_id()
        .map(sanitize_ui_trace_session_id)
        .unwrap_or_else(|| "trace".to_string());
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let path = log_dir.join(format!("ui-trace-{timestamp}-{session_id}.jsonl"));
    let file = std::fs::File::create(&path)
        .map_err(|error| format!("failed to create UI trace artifact: {error}"))?;
    let mut writer = std::io::BufWriter::new(file);
    for entry in entries {
        let sanitized = sanitize_ui_trace_entry(entry);
        serde_json::to_writer(&mut writer, &sanitized)
            .map_err(|error| format!("failed to serialize UI trace entry: {error}"))?;
        writer
            .write_all(b"\n")
            .map_err(|error| format!("failed to write UI trace entry: {error}"))?;
    }
    writer
        .flush()
        .map_err(|error| format!("failed to flush UI trace artifact: {error}"))?;
    Ok(UiTraceSaveResult {
        path,
        entries: entries.len(),
    })
}

fn sanitize_ui_trace_session_id(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                Some(ch)
            } else if ch == '.' {
                Some('-')
            } else {
                None
            }
        })
        .take(64)
        .collect();
    if sanitized.is_empty() {
        "trace".to_string()
    } else {
        sanitized
    }
}

fn sanitize_ui_trace_entry(entry: &UiTraceEntry) -> serde_json::Value {
    const BLOCKED_FIELDS: &[&str] = &[
        "body",
        "chunk",
        "data",
        "data_base64",
        "input",
        "payload",
        "text",
    ];
    let Some(object) = entry.fields() else {
        return serde_json::json!({ "kind": "invalid_entry" });
    };
    let mut sanitized = serde_json::Map::new();
    for (key, value) in object {
        let normalized_key = key.chars().fold(String::new(), |mut acc, ch| {
            if ch.is_ascii_uppercase() {
                acc.push('_');
                acc.push(ch.to_ascii_lowercase());
            } else {
                acc.push(ch);
            }
            acc
        });
        if BLOCKED_FIELDS.contains(&normalized_key.as_str()) {
            continue;
        }
        if value.is_null() || value.is_boolean() || value.is_number() || value.is_string() {
            sanitized.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(sanitized)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn save_ui_trace_to_log_dir_writes_jsonl_artifact() {
        let temp = tempdir().expect("tempdir");
        let result = save_ui_trace_to_log_dir(
            temp.path(),
            serde_json::from_value::<UiTracePayload>(serde_json::json!({
                "session_id": "../trace/slash",
                "entries": [
                    "malformed-entry",
                    { "kind": "trace_start", "ts": 1 },
                    {
                        "kind": "pointer_move_ignored",
                        "reason": "pointer_id_mismatch",
                        "data_base64": "must-not-leak"
                    }
                ]
            }))
            .expect("typed ui trace payload"),
        )
        .expect("save ui trace");

        assert_eq!(result.entries, 3);
        assert!(result.path.starts_with(temp.path()));
        let file_name = result
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf8 filename");
        assert!(file_name.starts_with("ui-trace-"));
        assert!(!file_name.contains('/'));

        let contents = fs::read_to_string(&result.path).expect("read trace");
        let lines: Vec<_> = contents.lines().collect();
        assert_eq!(lines.len(), 3);
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("json line");
        assert_eq!(first["kind"], "invalid_entry");
        let second: serde_json::Value = serde_json::from_str(lines[1]).expect("json line");
        assert_eq!(second["kind"], "trace_start");
        let third: serde_json::Value = serde_json::from_str(lines[2]).expect("json line");
        assert_eq!(third["reason"], "pointer_id_mismatch");
        assert!(!contents.contains("must-not-leak"));
    }

    #[test]
    fn save_ui_trace_to_log_dir_rejects_missing_entries_at_runtime() {
        let temp = tempdir().expect("tempdir");
        let error = save_ui_trace_to_log_dir(
            temp.path(),
            serde_json::from_value::<UiTracePayload>(serde_json::json!({
                "session_id": "trace-1"
            }))
            .expect("typed ui trace payload"),
        )
        .expect_err("missing entries must fail");

        assert_eq!(error, "trace payload missing entries array");
        assert!(fs::read_dir(temp.path())
            .expect("read temp")
            .next()
            .is_none());
    }
}
