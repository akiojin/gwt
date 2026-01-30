//! Conversion metadata storage and retrieval.
//!
//! This module handles persistent storage of conversion metadata, allowing
//! traceability of session conversions.

use std::fs;
use std::path::PathBuf;

use super::ConversionMetadata;

/// Error type for metadata store operations.
#[derive(Debug, thiserror::Error)]
pub enum MetadataStoreError {
    #[error("Failed to determine home directory")]
    HomeDirNotFound,
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Metadata not found for session: {0}")]
    NotFound(String),
}

/// Store for conversion metadata.
pub struct ConversionMetadataStore {
    base_dir: PathBuf,
}

impl ConversionMetadataStore {
    /// Creates a new metadata store with the default base directory.
    pub fn new() -> Result<Self, MetadataStoreError> {
        let home = dirs::home_dir().ok_or(MetadataStoreError::HomeDirNotFound)?;
        let base_dir = home.join(".gwt").join("conversions");
        Ok(Self { base_dir })
    }

    /// Creates a new metadata store with a custom base directory.
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Ensures the base directory exists.
    fn ensure_dir(&self) -> Result<(), MetadataStoreError> {
        if !self.base_dir.exists() {
            fs::create_dir_all(&self.base_dir)?;
        }
        Ok(())
    }

    /// Returns the path to the metadata file for a session.
    fn metadata_path(&self, session_id: &str) -> PathBuf {
        self.base_dir.join(format!("{}.json", session_id))
    }

    /// Saves conversion metadata for a session.
    pub fn save(
        &self,
        new_session_id: &str,
        metadata: &ConversionMetadata,
    ) -> Result<(), MetadataStoreError> {
        self.ensure_dir()?;
        let path = self.metadata_path(new_session_id);
        let content = serde_json::to_string_pretty(metadata)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Loads conversion metadata for a session.
    pub fn load(&self, session_id: &str) -> Result<ConversionMetadata, MetadataStoreError> {
        let path = self.metadata_path(session_id);
        if !path.exists() {
            return Err(MetadataStoreError::NotFound(session_id.to_string()));
        }
        let content = fs::read_to_string(path)?;
        let metadata = serde_json::from_str(&content)?;
        Ok(metadata)
    }

    /// Checks if metadata exists for a session.
    pub fn exists(&self, session_id: &str) -> bool {
        self.metadata_path(session_id).exists()
    }

    /// Lists all session IDs with stored metadata.
    pub fn list(&self) -> Result<Vec<String>, MetadataStoreError> {
        if !self.base_dir.exists() {
            return Ok(Vec::new());
        }

        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    ids.push(stem.to_string());
                }
            }
        }
        Ok(ids)
    }

    /// Deletes metadata for a session.
    pub fn delete(&self, session_id: &str) -> Result<(), MetadataStoreError> {
        let path = self.metadata_path(session_id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    fn sample_metadata() -> ConversionMetadata {
        ConversionMetadata {
            converted_from_agent: "Claude Code".to_string(),
            converted_from_session_id: "old-session-123".to_string(),
            converted_at: Utc::now(),
            dropped_messages: 0,
            dropped_tool_results: 2,
            lost_metadata_fields: vec!["custom_field".to_string()],
            loss_summary: "2 tool results dropped".to_string(),
            original_message_count: 10,
            original_tool_count: 5,
        }
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempdir().unwrap();
        let store = ConversionMetadataStore::with_base_dir(dir.path().to_path_buf());

        let metadata = sample_metadata();
        store.save("new-session-456", &metadata).unwrap();

        assert!(store.exists("new-session-456"));

        let loaded = store.load("new-session-456").unwrap();
        assert_eq!(loaded.converted_from_session_id, "old-session-123");
        assert_eq!(loaded.dropped_tool_results, 2);
    }

    #[test]
    fn test_load_not_found() {
        let dir = tempdir().unwrap();
        let store = ConversionMetadataStore::with_base_dir(dir.path().to_path_buf());

        let result = store.load("nonexistent");
        assert!(matches!(result, Err(MetadataStoreError::NotFound(_))));
    }

    #[test]
    fn test_list() {
        let dir = tempdir().unwrap();
        let store = ConversionMetadataStore::with_base_dir(dir.path().to_path_buf());

        let metadata = sample_metadata();
        store.save("session-a", &metadata).unwrap();
        store.save("session-b", &metadata).unwrap();

        let ids = store.list().unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"session-a".to_string()));
        assert!(ids.contains(&"session-b".to_string()));
    }

    #[test]
    fn test_delete() {
        let dir = tempdir().unwrap();
        let store = ConversionMetadataStore::with_base_dir(dir.path().to_path_buf());

        let metadata = sample_metadata();
        store.save("to-delete", &metadata).unwrap();
        assert!(store.exists("to-delete"));

        store.delete("to-delete").unwrap();
        assert!(!store.exists("to-delete"));
    }
}
