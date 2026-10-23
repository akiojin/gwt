//! Exact-launch bridge diagnostics, adjacent to the hook-owned runtime sidecar.

use std::{
    collections::BTreeMap,
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::{SessionExecutionIdentity, SessionRuntimeState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostBridgeKind {
    WorkspaceUpdate,
    ExecutionContinuation,
    ExecutionAdoption,
    WorkTerminalization,
    BuildAbortTerminalization,
    WorkMaterialization,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LaunchIdentity {
    execution: SessionExecutionIdentity,
    incarnation: u64,
    host_started_at: u64,
    child_pid: u32,
    child_started_at: u64,
}

impl LaunchIdentity {
    fn from_runtime(runtime: &SessionRuntimeState) -> Option<Self> {
        Some(Self {
            execution: runtime.execution_identity.clone()?,
            incarnation: runtime.runtime_incarnation.filter(|value| *value != 0)?,
            host_started_at: runtime.host_started_at.filter(|value| *value != 0)?,
            child_pid: runtime.child_pid.filter(|value| *value != 0)?,
            child_started_at: runtime.child_started_at.filter(|value| *value != 0)?,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct BridgeResult {
    started_at: DateTime<Utc>,
    transport_failed: bool,
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    launch: LaunchIdentity,
    bridges: BTreeMap<HostBridgeKind, BridgeResult>,
}

fn receipt_path(runtime_path: &Path) -> PathBuf {
    runtime_path.with_extension("bridge-receipt")
}

fn load_receipt(path: &Path) -> io::Result<Option<Receipt>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Capture before sending HTTP so a late result never labels a successor.
pub struct SessionBridgeObservation {
    runtime_path: PathBuf,
    launch: LaunchIdentity,
    started_at: DateTime<Utc>,
}

impl SessionBridgeObservation {
    pub fn capture(runtime_path: &Path, session_id: &str) -> io::Result<Option<Self>> {
        let runtime = SessionRuntimeState::load(runtime_path)?;
        Ok(LaunchIdentity::from_runtime(&runtime)
            .filter(|launch| launch.execution.session_id == session_id)
            .map(|launch| Self {
                runtime_path: runtime_path.to_owned(),
                launch,
                started_at: Utc::now(),
            }))
    }

    /// Publish only this bridge's latest request. Other bridges' faults remain.
    pub fn record(&self, bridge: HostBridgeKind, transport_failed: bool) -> io::Result<bool> {
        let path = receipt_path(&self.runtime_path);
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("bridge-lock"))?;
        lock.lock_exclusive()?;
        let current = SessionRuntimeState::load(&self.runtime_path)?;
        if LaunchIdentity::from_runtime(&current).as_ref() != Some(&self.launch) {
            return Ok(false);
        }
        let mut receipt = load_receipt(&path)?
            .filter(|record| record.launch == self.launch)
            .unwrap_or_else(|| Receipt {
                launch: self.launch.clone(),
                bridges: BTreeMap::new(),
            });
        if receipt
            .bridges
            .get(&bridge)
            .is_some_and(|value| value.started_at > self.started_at)
        {
            return Ok(false);
        }
        receipt.bridges.insert(
            bridge,
            BridgeResult {
                started_at: self.started_at,
                transport_failed,
            },
        );
        let dir = path
            .parent()
            .ok_or_else(|| io::Error::other("runtime sidecar has no parent"))?;
        let temp_path = dir.join(format!(".bridge-{}.tmp", uuid::Uuid::new_v4()));
        let mut temp = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)?;
        serde_json::to_writer(&mut temp, &receipt).map_err(io::Error::other)?;
        temp.write_all(b"\n")?;
        temp.sync_all()?;
        drop(temp);
        if let Err(error) = fs::rename(&temp_path, &path) {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
        Ok(true)
    }
}

/// A missing receipt means no bridge fault; unreadable evidence is an error.
pub fn has_unresolved_host_bridge_fault(
    runtime_path: &Path,
    runtime: &SessionRuntimeState,
) -> io::Result<bool> {
    let Some(launch) = LaunchIdentity::from_runtime(runtime) else {
        return Err(io::Error::other("runtime lacks exact launch identity"));
    };
    Ok(load_receipt(&receipt_path(runtime_path))?
        .filter(|receipt| receipt.launch == launch)
        .is_some_and(|receipt| receipt.bridges.values().any(|value| value.transport_failed)))
}
