//! Read-only proof of one Host update after its response was lost (#3838).
//! A reservation is not proof of publication: the exact source event and the
//! absence of pending transaction markers are required as well.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use gwt_agent::{LaunchRuntimeTarget, Session, SessionExecutionIdentity};
use gwt_core::{
    paths,
    workspace_projection::{
        workspace_state_transaction_is_pending_at, workspace_work_event_shard_matches, WorkEvent,
    },
};
use serde::{Deserialize, Serialize};

use crate::{AgentWorkspaceUpdateReceipt, AgentWorkspaceUpdateRequest};

pub(crate) const OPERATION_HEADER: &str = "x-gwt-workspace-operation-id";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateOperation {
    schema_version: u32,
    operation_id: String,
    identity: SessionExecutionIdentity,
    runtime_target: LaunchRuntimeTarget,
    docker_runtime_binding: Option<gwt_agent::DockerRuntimeBinding>,
    project_state_root: PathBuf,
    request: AgentWorkspaceUpdateRequest,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableReceipt {
    operation: UpdateOperation,
    event: WorkEvent,
    receipt: AgentWorkspaceUpdateReceipt,
}

#[derive(Serialize)]
pub(crate) struct ReceiptStatus {
    schema_version: u32,
    operation_id: String,
    status: &'static str,
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) receipt: Option<AgentWorkspaceUpdateReceipt>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(crate) fn validate_operation_id(id: &str) -> io::Result<()> {
    if uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id) {
        Ok(())
    } else {
        Err(invalid("workspace operation_id must be a canonical UUID"))
    }
}

fn receipt_path(root: &Path, id: &str) -> io::Result<PathBuf> {
    validate_operation_id(id)?;
    Ok(paths::gwt_workspace_projection_path_for_repo_path(root)
        .with_file_name("workspace-update-receipts")
        .join(format!("{id}.json")))
}

impl UpdateOperation {
    pub(crate) fn capture(
        root: &Path,
        session_id: &str,
        id: &str,
        request: &AgentWorkspaceUpdateRequest,
    ) -> io::Result<Self> {
        validate_operation_id(id)?;
        gwt_agent::validate_session_id_path_component(session_id)
            .map_err(|_| invalid("invalid Session id"))?;
        let session = Session::load(&paths::gwt_sessions_dir().join(format!("{session_id}.toml")))
            .map_err(|_| invalid("operation receipt Session is unreadable"))?;
        let identity = SessionExecutionIdentity::from_session(&session)
            .map_err(|_| invalid("operation receipt Session identity is invalid"))?
            .ok_or_else(|| invalid("operation receipt requires a bound Session"))?;
        if identity.session_id != session_id || request.claimed_session_id != session_id {
            return Err(invalid("operation receipt Session claim mismatch"));
        }
        Ok(Self {
            schema_version: 1,
            operation_id: id.to_string(),
            identity,
            runtime_target: session.runtime_target,
            docker_runtime_binding: session.docker_runtime_binding,
            project_state_root: root.to_path_buf(),
            request: request.clone(),
        })
    }

    /// Called under project locks BEFORE reserving a terminal obligation.
    pub(crate) fn require_unreserved(&self) -> io::Result<()> {
        match fs::symlink_metadata(receipt_path(&self.project_state_root, &self.operation_id)?) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
            Ok(_) => return Err(invalid("workspace operation_id was already reserved; inspect workspace.receipt instead of resending")),
        }
        self.require_current_identity()
    }

    fn require_current_identity(&self) -> io::Result<()> {
        let current = Self::capture(
            &self.project_state_root,
            &self.identity.session_id,
            &self.operation_id,
            &self.request,
        )?;
        if current.identity != self.identity
            || current.runtime_target != self.runtime_target
            || current.docker_runtime_binding != self.docker_runtime_binding
        {
            return Err(invalid("workspace operation receipt authority changed"));
        }
        Ok(())
    }

    /// Called under the same project locks AFTER settlement chooses final IDs.
    pub(crate) fn reserve(
        &self,
        event: &WorkEvent,
        receipt: AgentWorkspaceUpdateReceipt,
    ) -> io::Result<()> {
        let path = receipt_path(&self.project_state_root, &self.operation_id)?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid("receipt has no parent"))?;
        match fs::create_dir(parent) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        if !fs::symlink_metadata(parent)?.file_type().is_dir() {
            return Err(invalid("receipt directory is not a real directory"));
        }
        #[cfg(unix)]
        if let Some(project_directory) = parent.parent() {
            fs::File::open(project_directory)?.sync_all()?;
        }
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(
            &mut temp,
            &DurableReceipt {
                operation: self.clone(),
                event: event.clone(),
                receipt,
            },
        )?;
        temp.write_all(b"\n")?;
        temp.as_file().sync_all()?;
        temp.persist_noclobber(&path).map_err(|error| error.error)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}

/// Never contacts the bridge, takes a writer lock, or repairs/reconciles state.
pub(crate) fn inspect(
    root: &Path,
    session_id: &str,
    id: &str,
    expected: Option<(&AgentWorkspaceUpdateRequest, &SessionExecutionIdentity)>,
) -> io::Result<ReceiptStatus> {
    validate_operation_id(id)?;
    let proof = (|| -> io::Result<AgentWorkspaceUpdateReceipt> {
        let recovery =
            crate::agent_project_state::validated_workspace_recovery_session(root, session_id)
                .map_err(|_| invalid("current Session authority is unavailable"))?
                .ok_or_else(|| invalid("current Session authority is missing"))?;
        let crate::agent_project_state::ValidatedWorkspaceEnsureSession::Host(recovery) = recovery
        else {
            return Err(invalid(
                "receipt recovery requires exact local Host authority",
            ));
        };
        let path = receipt_path(&recovery.project_state_root, id)?;
        if !fs::symlink_metadata(
            path.parent()
                .ok_or_else(|| invalid("receipt has no parent"))?,
        )?
        .file_type()
        .is_dir()
        {
            return Err(invalid("receipt directory is not a real directory"));
        }
        if !fs::symlink_metadata(&path)?.file_type().is_file() {
            return Err(invalid("operation receipt is not a regular file"));
        }
        let durable: DurableReceipt = serde_json::from_slice(&fs::read(path)?)?;
        let operation = &durable.operation;
        let intent = &operation.request.intent;
        let expected_kind = match intent.status_category {
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Done) => {
                gwt_core::workspace_projection::WorkEventKind::Done
            }
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked) => {
                gwt_core::workspace_projection::WorkEventKind::Blocked
            }
            _ => gwt_core::workspace_projection::WorkEventKind::Update,
        };
        if operation.schema_version != 1
            || operation.operation_id != id
            || operation.identity.session_id != session_id
            || operation.project_state_root != recovery.project_state_root
            || operation.identity.worktree_path != recovery.session.worktree_path
            || operation.runtime_target != LaunchRuntimeTarget::Host
            || operation.docker_runtime_binding.is_some()
            || expected.is_some_and(|(request, identity)| {
                request != &operation.request || identity != &operation.identity
            })
            || durable.event.agent_session_id.as_deref() != Some(session_id)
            || durable.event.work_item_id != durable.receipt.work_id
            || durable.receipt.schema_version != crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION
            || durable.receipt.journal_entry_id.is_empty()
            || operation.request.schema_version != crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION
            || operation.request.claimed_session_id != session_id
            || durable.event.kind != expected_kind
            || durable.event.title.as_ref()
                != intent.title.as_ref().or(intent.title_summary.as_ref())
            || durable.event.intent != intent.current_focus
            || durable.event.summary.as_ref()
                != intent.summary.as_ref().or(intent.status_text.as_ref())
            || durable.event.progress_summary != intent.progress_summary
            || durable.event.status_category != intent.status_category
            || durable.event.next_action != intent.next_action
        {
            return Err(invalid("operation receipt request or authority mismatch"));
        }
        operation.require_current_identity()?;
        let current =
            paths::gwt_workspace_projection_path_for_repo_path(&recovery.project_state_root);
        let works =
            paths::gwt_workspace_work_items_path_for_repo_path(&recovery.project_state_root);
        let pending = || {
            workspace_state_transaction_is_pending_at(&current, &works)
                .map_err(|_| invalid("workspace publication state is unreadable"))
        };
        if pending()? {
            return Err(invalid("workspace publication is still pending"));
        }
        if !workspace_work_event_shard_matches(&operation.identity.worktree_path, &durable.event)
            .map_err(|_| invalid("operation source is unreadable"))?
            || pending()?
        {
            return Err(invalid(
                "operation source does not prove completed publication",
            ));
        }
        let final_authority =
            crate::agent_project_state::validated_workspace_recovery_session(root, session_id)
                .map_err(|_| invalid("current Session authority changed"))?;
        let Some(crate::agent_project_state::ValidatedWorkspaceEnsureSession::Host(
            final_authority,
        )) = final_authority
        else {
            return Err(invalid("current Host authority changed"));
        };
        crate::agent_project_state::validate_workspace_update_receipt_work_authority(
            &final_authority,
            &durable.event,
        )
        .map_err(|_| invalid("current Work authority changed"))?;
        operation.require_current_identity()?;
        if pending()? {
            return Err(invalid("workspace publication changed during inspection"));
        }
        Ok(durable.receipt)
    })();
    let (status, reason, receipt) = match proof {
        Ok(receipt) => ("applied", "exact durable Host publication confirmed", Some(receipt)),
        Err(_) => ("unconfirmed", "receipt, current Host authority or completed publication is unavailable; do not resend the update", None),
    };
    Ok(ReceiptStatus {
        schema_version: 1,
        operation_id: id.to_string(),
        status,
        reason: reason.to_string(),
        receipt,
    })
}
