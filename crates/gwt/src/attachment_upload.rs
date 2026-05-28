use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UploadedAttachment {
    pub(crate) path: PathBuf,
    pub(crate) filename: String,
    pub(crate) mime_type: Option<String>,
    pub(crate) size: u64,
}

#[derive(Clone)]
pub(crate) struct AttachmentUploadStore {
    root: Arc<PathBuf>,
    uploads: Arc<Mutex<HashMap<String, UploadedAttachment>>>,
}

impl AttachmentUploadStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root: Arc::new(root),
            uploads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn in_system_temp() -> Self {
        Self::new(
            std::env::temp_dir()
                .join("gwt-attachment-uploads")
                .join(Uuid::new_v4().to_string()),
        )
    }

    pub(crate) fn root(&self) -> &Path {
        self.root.as_ref().as_path()
    }

    pub(crate) fn allocate_path(&self) -> (String, PathBuf) {
        let upload_id = Uuid::new_v4().to_string();
        let path = self.root().join(format!("{upload_id}.upload"));
        (upload_id, path)
    }

    pub(crate) fn insert(
        &self,
        upload_id: String,
        attachment: UploadedAttachment,
    ) -> Result<(), String> {
        let mut uploads = self
            .uploads
            .lock()
            .map_err(|error| format!("attachment upload store lock poisoned: {error}"))?;
        uploads.insert(upload_id, attachment);
        Ok(())
    }

    pub(crate) fn take(&self, upload_id: &str) -> Result<Option<UploadedAttachment>, String> {
        let mut uploads = self
            .uploads
            .lock()
            .map_err(|error| format!("attachment upload store lock poisoned: {error}"))?;
        Ok(uploads.remove(upload_id))
    }
}
