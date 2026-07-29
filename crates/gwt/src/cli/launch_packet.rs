//! Phase Launch Packet (SPEC-3248 P7C, T-275 staged core).
//!
//! Launch materialization for a linked owner emits a compact, machine-
//! readable launch context at `.gwt/skill-state/phase-launch-packet.json`:
//! the owner binding, the entrypoint, the artifact operability references
//! from the per-owner ledger (T-274 — section sizes, parts, hashes,
//! resident locations), and the canonical refresh/readback commands. Agents
//! read the packet instead of re-deriving artifact shape from GitHub
//! snapshots, and the packet is the substrate T-276 will bind `phase_slice`
//! enforcement to.
//!
//! STAGED rollout (user decision, 2026-07-28): packet GENERATION ships
//! first as an additive artifact. `phase_slice` stays `None` until launches
//! carry one, and broad-launch REJECTION (T-276) remains off until
//! explicitly opted in — blanket permission was deliberately not read as
//! consent to refuse the current wholesale-launch workflow.
//!
//! The packet is informational launch context, not gate-trusted state: it
//! is not in the direct-write guard list and lives only in the worktree.

use std::{io, path::Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Worktree-relative path of the packet.
pub const PHASE_LAUNCH_PACKET_RELATIVE: &str = ".gwt/skill-state/phase-launch-packet.json";

/// One artifact reference copied from the operability ledger (T-274).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketArtifactRef {
    pub section: String,
    pub bytes: usize,
    pub parts: usize,
    pub location: String,
    pub sha256: String,
}

/// The compact launch context (T-275 staged core).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseLaunchPacket {
    pub owner_kind: String,
    pub owner_number: u64,
    pub session_id: String,
    pub entrypoint: String,
    /// Not carried by launches yet — populated once T-276 lands. A broad
    /// launch SHOULD name one phase slice; the packet records the gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_slice: Option<String>,
    /// Artifact operability references for the owner (empty when the
    /// ledger has no record yet).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<PacketArtifactRef>,
    /// Canonical commands to refresh and read the owner's sections.
    pub refresh_commands: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

/// Write the packet for a launch. Best-effort from the launch path: a
/// packet failure must not fail the launch.
pub fn write_best_effort(
    worktree: &Path,
    owner_kind: &str,
    owner_number: u64,
    session_id: &str,
    entrypoint: &str,
) {
    if let Err(error) = write(worktree, owner_kind, owner_number, session_id, entrypoint) {
        tracing::warn!(?error, "phase launch packet write failed");
    }
}

fn write(
    worktree: &Path,
    owner_kind: &str,
    owner_number: u64,
    session_id: &str,
    entrypoint: &str,
) -> io::Result<()> {
    let artifacts = crate::cli::artifact_operability::load(worktree, owner_number)
        .ok()
        .flatten()
        .map(|record| {
            record
                .sections
                .into_iter()
                .map(|(section, operability)| PacketArtifactRef {
                    section,
                    bytes: operability.bytes,
                    parts: operability.parts,
                    location: operability.location,
                    sha256: operability.sha256,
                })
                .collect()
        })
        .unwrap_or_default();
    let packet = PhaseLaunchPacket {
        owner_kind: owner_kind.to_string(),
        owner_number,
        session_id: session_id.to_string(),
        entrypoint: entrypoint.to_string(),
        phase_slice: None,
        artifacts,
        refresh_commands: vec![
            format!("gwtd {{\"operation\":\"issue.spec.pull\",\"params\":{{\"numbers\":[{owner_number}]}}}}"),
            format!("gwtd {{\"operation\":\"issue.spec.section\",\"params\":{{\"number\":{owner_number},\"section\":\"tasks\"}}}}"),
        ],
        generated_at: Utc::now(),
    };
    let serialized = serde_json::to_vec_pretty(&packet)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    gwt_github::cache::write_atomic(&worktree.join(PHASE_LAUNCH_PACKET_RELATIVE), &serialized)
}

/// Read the packet (informational; absent is normal for unlinked launches).
pub fn load(worktree: &Path) -> io::Result<Option<PhaseLaunchPacket>> {
    match std::fs::read_to_string(worktree.join(PHASE_LAUNCH_PACKET_RELATIVE)) {
        Ok(contents) => {
            Ok(Some(serde_json::from_str(&contents).map_err(|err| {
                io::Error::new(io::ErrorKind::InvalidData, err)
            })?))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    // T-275 staged core: launches emit the packet; operability references
    // flow in from the T-274 ledger when it exists.
    #[test]
    fn packet_carries_owner_binding_and_operability_refs() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        // No ledger yet: packet still lands with empty artifacts.
        write_best_effort(dir.path(), "spec", 3248, "sess-1", "$gwt-execute");
        let packet = load(dir.path()).unwrap().unwrap();
        assert_eq!(packet.owner_number, 3248);
        assert_eq!(packet.phase_slice, None);
        assert!(packet.artifacts.is_empty());
        assert!(packet.refresh_commands[0].contains("issue.spec.pull"));

        // Ledger entries surface as artifact references.
        crate::cli::artifact_operability::record_write(
            dir.path(),
            3248,
            "tasks",
            &gwt_github::WriteReceipt {
                bytes: 90_000,
                parts: 2,
                sha256: "hash-tasks".to_string(),
                location: "comments".to_string(),
                comment_ids: vec![1, 2],
                largest_part_bytes: 60_000,
            },
        )
        .unwrap();
        write_best_effort(dir.path(), "spec", 3248, "sess-1", "$gwt-execute");
        let packet = load(dir.path()).unwrap().unwrap();
        assert_eq!(packet.artifacts.len(), 1);
        assert_eq!(packet.artifacts[0].section, "tasks");
        assert_eq!(packet.artifacts[0].parts, 2);
        assert_eq!(packet.artifacts[0].location, "comments");
    }
}
