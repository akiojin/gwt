//! Coordinator Host credential registry and offline administration (Issue #6476).
//!
//! No listener, device approval or PTY authority lives here. The storage module
//! holds a separate registry lock, saves candidate secrets to
//! private files first, atomically persists the registry second, and reloads
//! before reporting success. Preparing a candidate is not successful issuance.

pub mod invite;
pub mod registration;
pub mod storage;

pub mod admin;

mod config;
pub use config::{ConfigError, CoordinatorConfig, CoordinatorLimits, TlsError};
pub mod listener;

use gwt_core::repo_hash::ProjectKey;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use subtle::ConstantTimeEq;
use uuid::Uuid;

const MAX_REGISTRY_BYTES: usize = 128 * 1024;
const MAX_RECORDS: usize = 256;
const MAX_PROJECTS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("invalid mobile Host credential registry or issuance parameters")]
    InvalidRegistry,
    #[error("mobile Host credential registry capacity reached; tombstones are retained")]
    Capacity,
    #[error("mobile Host registration credential is unauthorized")]
    Unauthorized,
    #[error("mobile Host registration credential expired")]
    Expired,
    #[error("mobile coordinator clock moved backwards; explicit recovery is required")]
    ClockRollback,
    #[error("mobile Host credential randomness is unavailable")]
    Unavailable,
}

/// Public correlation values. These do not authorize a Host or a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialIdentity {
    pub host_id: Uuid,
    pub credential_id: Uuid,
    pub generation: Uuid,
}

/// Deliberately has no Debug, Clone or Serialize implementation. The future
/// private-file writer must explicitly borrow the bearer, never log it.
pub struct HostCredential {
    identity: CredentialIdentity,
    secret: [u8; 32],
}

impl HostCredential {
    pub fn identity(&self) -> &CredentialIdentity {
        &self.identity
    }

    pub fn secret_bytes(&self) -> &[u8; 32] {
        &self.secret
    }
}

/// No activation/verification API: the candidate is not the live registry.
/// Its non-secret snapshot contains the new digest and any old tombstone.
pub struct PreparedCredentialUpdate {
    snapshot: Vec<u8>,
    credential: HostCredential,
}

impl PreparedCredentialUpdate {
    pub fn registry_snapshot(&self) -> &[u8] {
        &self.snapshot
    }

    pub fn credential(&self) -> &HostCredential {
        &self.credential
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialRecord {
    identity: CredentialIdentity,
    projects: Vec<String>,
    issued_at: u64,
    expires_at: u64,
    revoked: bool,
    digest: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRegistry {
    schema_version: u32,
    last_observed: u64,
    records: Vec<CredentialRecord>,
}

/// Runtime verification updates its trusted wall-clock watermark, even on a
/// failed lookup. Rollback is terminal for this value and prevents candidate
/// export. The future adapter must persist watermarks/handle operator recovery;
/// reloading an old file is not proof of clock safety across process restarts.
pub struct CoordinatorRegistry {
    stored: StoredRegistry,
    clock_failed: bool,
}

impl CoordinatorRegistry {
    pub fn empty(now: u64) -> Self {
        Self {
            stored: StoredRegistry {
                schema_version: 1,
                last_observed: now,
                records: Vec::new(),
            },
            clock_failed: false,
        }
    }

    /// Read only a trusted private registry, never a browser-supplied snapshot.
    pub fn from_json(bytes: &[u8], now: u64) -> Result<Self, CredentialError> {
        if bytes.len() > MAX_REGISTRY_BYTES {
            return Err(CredentialError::Capacity);
        }
        let stored: StoredRegistry =
            serde_json::from_slice(bytes).map_err(|_| CredentialError::InvalidRegistry)?;
        if stored.records.len() > MAX_RECORDS {
            return Err(CredentialError::Capacity);
        }
        let mut ids = HashSet::new();
        let mut active_hosts = HashSet::new();
        if stored.schema_version != 1
            || stored.records.iter().any(|record| {
                record.identity.host_id.is_nil()
                    || record.identity.credential_id.is_nil()
                    || record.identity.generation.is_nil()
                    || record.issued_at >= record.expires_at
                    || record.issued_at > stored.last_observed
                    || !valid_projects(&record.projects)
                    || !ids.insert(record.identity.credential_id)
                    || (!record.revoked && !active_hosts.insert(record.identity.host_id))
            })
        {
            return Err(CredentialError::InvalidRegistry);
        }
        let mut registry = Self {
            stored,
            clock_failed: false,
        };
        registry.observe(now)?;
        Ok(registry)
    }

    /// Only Host registration/signaling scope, never device/Project execution.
    pub fn verify(
        &mut self,
        identity: &CredentialIdentity,
        project: &ProjectKey,
        secret: &[u8],
        now: u64,
    ) -> Result<(), CredentialError> {
        self.verify_expiry(identity, project, secret, now)
            .map(|_| ())
    }

    fn verify_expiry(
        &mut self,
        identity: &CredentialIdentity,
        project: &ProjectKey,
        secret: &[u8],
        now: u64,
    ) -> Result<u64, CredentialError> {
        self.observe(now)?;
        let record = self
            .stored
            .records
            .iter()
            .find(|r| r.identity == *identity && !r.revoked)
            .ok_or(CredentialError::Unauthorized)?;
        if secret.len() != 32 {
            return Err(CredentialError::Unauthorized);
        }
        let digest: [u8; 32] = Sha256::digest(secret).into();
        if !bool::from(record.digest.ct_eq(&digest))
            || !record.projects.iter().any(|p| p == project.as_str())
        {
            return Err(CredentialError::Unauthorized);
        }
        if now >= record.expires_at {
            return Err(CredentialError::Expired);
        }
        Ok(record.expires_at)
    }

    /// Authority records remain unchanged; only the clock watermark advances.
    pub fn prepare_issue(
        &mut self,
        host_id: Uuid,
        projects: &[ProjectKey],
        expires_at: u64,
        now: u64,
    ) -> Result<PreparedCredentialUpdate, CredentialError> {
        self.observe(now)?;
        let projects: Vec<_> = projects.iter().map(|p| p.as_str().to_owned()).collect();
        if host_id.is_nil()
            || !valid_projects(&projects)
            || self
                .stored
                .records
                .iter()
                .any(|r| r.identity.host_id == host_id && !r.revoked)
        {
            return Err(CredentialError::InvalidRegistry);
        }
        self.prepare(self.stored.clone(), host_id, projects, expires_at, now)
    }

    /// Preserve Host/Project binding while atomically replacing its credential.
    /// An expired credential may be rotated offline; a revoked one requires issue.
    pub fn prepare_rotate(
        &mut self,
        credential_id: Uuid,
        expires_at: u64,
        now: u64,
    ) -> Result<PreparedCredentialUpdate, CredentialError> {
        self.observe(now)?;
        let mut next = self.stored.clone();
        let old = next
            .records
            .iter_mut()
            .find(|r| r.identity.credential_id == credential_id && !r.revoked)
            .ok_or(CredentialError::Unauthorized)?;
        old.revoked = true;
        let host_id = old.identity.host_id;
        let projects = old.projects.clone();
        self.prepare(next, host_id, projects, expires_at, now)
    }

    pub fn prepare_revoke(
        &mut self,
        credential_id: Uuid,
        now: u64,
    ) -> Result<Vec<u8>, CredentialError> {
        self.observe(now)?;
        let mut next = self.stored.clone();
        next.records
            .iter_mut()
            .find(|r| r.identity.credential_id == credential_id)
            .ok_or(CredentialError::Unauthorized)?
            .revoked = true;
        encode(&next)
    }

    fn prepare(
        &self,
        mut next: StoredRegistry,
        host_id: Uuid,
        projects: Vec<String>,
        expires_at: u64,
        now: u64,
    ) -> Result<PreparedCredentialUpdate, CredentialError> {
        if expires_at <= now {
            return Err(CredentialError::InvalidRegistry);
        }
        if next.records.len() >= MAX_RECORDS {
            return Err(CredentialError::Capacity);
        }
        let mut secret = [0; 32];
        SystemRandom::new()
            .fill(&mut secret)
            .map_err(|_| CredentialError::Unavailable)?;
        let credential = HostCredential {
            identity: CredentialIdentity {
                host_id,
                credential_id: Uuid::new_v4(),
                generation: Uuid::new_v4(),
            },
            secret,
        };
        next.records.push(CredentialRecord {
            identity: credential.identity,
            projects,
            issued_at: now,
            expires_at,
            revoked: false,
            digest: Sha256::digest(secret).into(),
        });
        Ok(PreparedCredentialUpdate {
            snapshot: encode(&next)?,
            credential,
        })
    }

    fn observe(&mut self, now: u64) -> Result<(), CredentialError> {
        if self.clock_failed || now < self.stored.last_observed {
            self.clock_failed = true;
            return Err(CredentialError::ClockRollback);
        }
        self.stored.last_observed = now;
        Ok(())
    }
}

fn valid_projects(projects: &[String]) -> bool {
    let mut unique = HashSet::new();
    (1..=MAX_PROJECTS).contains(&projects.len())
        && projects
            .iter()
            .all(|p| ProjectKey::parse(p).is_ok() && unique.insert(p))
}

fn encode(stored: &StoredRegistry) -> Result<Vec<u8>, CredentialError> {
    let bytes = serde_json::to_vec(stored).map_err(|_| CredentialError::InvalidRegistry)?;
    if bytes.len() > MAX_REGISTRY_BYTES {
        return Err(CredentialError::Capacity);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> ProjectKey {
        ProjectKey::parse("0123456789abcdef").unwrap()
    }

    fn issue(registry: &mut CoordinatorRegistry) -> PreparedCredentialUpdate {
        registry
            .prepare_issue(Uuid::new_v4(), &[project()], 200, 100)
            .unwrap()
    }

    #[test]
    fn issued_candidate_is_inactive_and_binds_all_registration_fields() {
        let mut original = CoordinatorRegistry::empty(100);
        let candidate = issue(&mut original);
        let credential = candidate.credential();
        let identity = credential.identity();
        let secret = credential.secret_bytes();
        assert_eq!(secret.len(), 32);
        assert_ne!(secret, &[0; 32]);
        assert_eq!(
            original.verify(identity, &project(), secret, 100),
            Err(CredentialError::Unauthorized)
        );
        let json = candidate.registry_snapshot();
        assert!(!String::from_utf8_lossy(json).contains(&hex::encode(secret)));
        let stored: serde_json::Value = serde_json::from_slice(json).unwrap();
        assert!(stored["records"][0].get("secret").is_none());
        let mut loaded = CoordinatorRegistry::from_json(json, 100).unwrap();
        loaded.verify(identity, &project(), secret, 100).unwrap();
        let other = ProjectKey::parse("fedcba9876543210").unwrap();
        assert_eq!(
            loaded.verify(identity, &other, secret, 100),
            Err(CredentialError::Unauthorized)
        );
        for foreign in [
            CredentialIdentity {
                host_id: Uuid::new_v4(),
                ..*identity
            },
            CredentialIdentity {
                credential_id: Uuid::new_v4(),
                ..*identity
            },
            CredentialIdentity {
                generation: Uuid::new_v4(),
                ..*identity
            },
        ] {
            assert_eq!(
                loaded.verify(&foreign, &project(), secret, 100),
                Err(CredentialError::Unauthorized)
            );
        }
        assert_eq!(
            loaded.verify(identity, &project(), &[1; 32], 100),
            Err(CredentialError::Unauthorized)
        );
        assert_eq!(
            loaded.verify(identity, &project(), &secret[..31], 100),
            Err(CredentialError::Unauthorized)
        );
        assert!(matches!(
            original.prepare_issue(Uuid::nil(), &[project()], 200, 100),
            Err(CredentialError::InvalidRegistry)
        ));
        assert!(matches!(
            original.prepare_issue(Uuid::new_v4(), &[], 200, 100),
            Err(CredentialError::InvalidRegistry)
        ));
        assert!(matches!(
            original.prepare_issue(Uuid::new_v4(), &[project()], 100, 100),
            Err(CredentialError::InvalidRegistry)
        ));
    }

    #[test]
    fn rotation_and_revocation_are_atomic_candidates_with_retained_tombstones() {
        let first = issue(&mut CoordinatorRegistry::empty(100));
        let old = first.credential();
        let mut registry = CoordinatorRegistry::from_json(first.registry_snapshot(), 100).unwrap();
        let rotated = registry
            .prepare_rotate(old.identity().credential_id, 300, 110)
            .unwrap();
        let new = rotated.credential();
        assert_eq!(new.identity().host_id, old.identity().host_id);
        assert_ne!(new.identity().credential_id, old.identity().credential_id);
        assert_ne!(new.identity().generation, old.identity().generation);
        assert_ne!(new.secret_bytes(), old.secret_bytes());
        registry
            .verify(old.identity(), &project(), old.secret_bytes(), 110)
            .unwrap();
        let mut loaded = CoordinatorRegistry::from_json(rotated.registry_snapshot(), 110).unwrap();
        assert_eq!(
            loaded.verify(old.identity(), &project(), old.secret_bytes(), 110),
            Err(CredentialError::Unauthorized)
        );
        loaded
            .verify(new.identity(), &project(), new.secret_bytes(), 110)
            .unwrap();
        let revoked = loaded
            .prepare_revoke(new.identity().credential_id, 120)
            .unwrap();
        loaded
            .verify(new.identity(), &project(), new.secret_bytes(), 120)
            .unwrap();
        let mut revoked_registry = CoordinatorRegistry::from_json(&revoked, 120).unwrap();
        assert_eq!(
            revoked_registry.verify(new.identity(), &project(), new.secret_bytes(), 120),
            Err(CredentialError::Unauthorized)
        );
        let snapshot: serde_json::Value = serde_json::from_slice(&revoked).unwrap();
        assert_eq!(snapshot["records"].as_array().unwrap().len(), 2);
        assert!(snapshot["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["revoked"] == true));
        assert!(matches!(
            revoked_registry.prepare_rotate(new.identity().credential_id, 300, 120),
            Err(CredentialError::Unauthorized)
        ));
        assert!(matches!(
            registry.prepare_issue(old.identity().host_id, &[project()], 300, 120),
            Err(CredentialError::InvalidRegistry)
        ));
        // A revoked Host may explicitly receive fresh credentials; old IDs stay revoked.
        revoked_registry
            .prepare_issue(new.identity().host_id, &[project()], 300, 120)
            .unwrap();
    }

    #[test]
    fn expiry_is_observed_before_lookup_and_clock_rollback_is_terminal() {
        let candidate = issue(&mut CoordinatorRegistry::empty(100));
        let credential = candidate.credential();
        let mut registry =
            CoordinatorRegistry::from_json(candidate.registry_snapshot(), 100).unwrap();
        registry
            .verify(
                credential.identity(),
                &project(),
                credential.secret_bytes(),
                199,
            )
            .unwrap();
        assert_eq!(
            registry.verify(
                credential.identity(),
                &project(),
                credential.secret_bytes(),
                200
            ),
            Err(CredentialError::Expired)
        );
        assert_eq!(
            registry.verify(
                credential.identity(),
                &project(),
                credential.secret_bytes(),
                199
            ),
            Err(CredentialError::ClockRollback)
        );
        assert_eq!(
            registry.verify(
                credential.identity(),
                &project(),
                credential.secret_bytes(),
                201
            ),
            Err(CredentialError::ClockRollback)
        );
        assert!(matches!(
            registry.prepare_revoke(credential.identity().credential_id, 201),
            Err(CredentialError::ClockRollback)
        ));
        assert!(matches!(
            CoordinatorRegistry::from_json(candidate.registry_snapshot(), 99),
            Err(CredentialError::ClockRollback)
        ));
        let mut unknown = CoordinatorRegistry::empty(100);
        assert_eq!(
            unknown.verify(
                credential.identity(),
                &project(),
                credential.secret_bytes(),
                120
            ),
            Err(CredentialError::Unauthorized)
        );
        assert_eq!(
            unknown.verify(
                credential.identity(),
                &project(),
                credential.secret_bytes(),
                110
            ),
            Err(CredentialError::ClockRollback)
        );
    }

    #[test]
    fn load_revalidates_schema_identity_scope_and_duplicate_records() {
        let candidate = issue(&mut CoordinatorRegistry::empty(100));
        let valid: serde_json::Value =
            serde_json::from_slice(candidate.registry_snapshot()).unwrap();
        let mut cases = Vec::new();
        let mut bad = valid.clone();
        bad["schema_version"] = 2.into();
        cases.push(bad);
        let mut bad = valid.clone();
        bad["secret"] = "unexpected".into();
        cases.push(bad);
        let mut bad = valid.clone();
        bad["records"][0]["secret"] = "unexpected".into();
        cases.push(bad);
        let mut bad = valid.clone();
        bad["records"][0]["identity"]["host_id"] = Uuid::nil().to_string().into();
        cases.push(bad);
        let mut bad = valid.clone();
        bad["records"][0]["projects"] = serde_json::json!(["../bad"]);
        cases.push(bad);
        let mut bad = valid.clone();
        bad["records"][0]["projects"] = serde_json::json!([project().as_str(), project().as_str()]);
        cases.push(bad);
        let mut bad = valid.clone();
        bad["records"][0]["expires_at"] = 100.into();
        cases.push(bad);
        let mut bad = valid.clone();
        bad["records"]
            .as_array_mut()
            .unwrap()
            .push(valid["records"][0].clone());
        cases.push(bad);
        let mut bad = valid.clone();
        let mut duplicate_host = valid["records"][0].clone();
        duplicate_host["identity"]["credential_id"] = Uuid::new_v4().to_string().into();
        bad["records"].as_array_mut().unwrap().push(duplicate_host);
        cases.push(bad);
        for bad in cases {
            assert!(matches!(
                CoordinatorRegistry::from_json(&serde_json::to_vec(&bad).unwrap(), 100),
                Err(CredentialError::InvalidRegistry)
            ));
        }
        assert!(matches!(
            CoordinatorRegistry::from_json(&vec![b' '; 128 * 1024 + 1], 100),
            Err(CredentialError::Capacity)
        ));
    }

    #[test]
    fn capacity_includes_tombstones_and_failed_rotation_keeps_old_authority() {
        let candidate = issue(&mut CoordinatorRegistry::empty(100));
        let credential = candidate.credential();
        let mut snapshot: serde_json::Value =
            serde_json::from_slice(candidate.registry_snapshot()).unwrap();
        let mut tombstone = snapshot["records"][0].clone();
        tombstone["revoked"] = true.into();
        for _ in 1..256 {
            tombstone["identity"]["credential_id"] = Uuid::new_v4().to_string().into();
            snapshot["records"]
                .as_array_mut()
                .unwrap()
                .push(tombstone.clone());
        }
        let mut full =
            CoordinatorRegistry::from_json(&serde_json::to_vec(&snapshot).unwrap(), 100).unwrap();
        assert!(matches!(
            full.prepare_rotate(credential.identity().credential_id, 300, 100),
            Err(CredentialError::Capacity)
        ));
        assert!(matches!(
            full.prepare_issue(Uuid::new_v4(), &[project()], 300, 100),
            Err(CredentialError::Capacity)
        ));
        full.verify(
            credential.identity(),
            &project(),
            credential.secret_bytes(),
            100,
        )
        .unwrap();
        snapshot["records"].as_array_mut().unwrap().push(tombstone);
        assert!(matches!(
            CoordinatorRegistry::from_json(&serde_json::to_vec(&snapshot).unwrap(), 100),
            Err(CredentialError::Capacity)
        ));
        let projects: Vec<_> = (0..17)
            .map(|i| ProjectKey::parse(&format!("{i:016x}")).unwrap())
            .collect();
        assert!(matches!(
            CoordinatorRegistry::empty(100).prepare_issue(Uuid::new_v4(), &projects, 200, 100),
            Err(CredentialError::InvalidRegistry)
        ));
        // Byte capacity can be reached before the record count. A candidate
        // must fail without discarding tombstones or revoking existing access.
        snapshot["records"].as_array_mut().unwrap().pop();
        for record in snapshot["records"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .skip(1)
        {
            record["projects"] = serde_json::json!(projects[..16]
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>());
            record["digest"] = serde_json::json!(vec![255u8; 32]);
        }
        while serde_json::to_vec(&snapshot).unwrap().len() > 128 * 1024 {
            snapshot["records"].as_array_mut().unwrap().pop();
        }
        let mut nearly_full =
            CoordinatorRegistry::from_json(&serde_json::to_vec(&snapshot).unwrap(), 100).unwrap();
        assert!(snapshot["records"].as_array().unwrap().len() < 256);
        assert!(matches!(
            nearly_full.prepare_issue(Uuid::new_v4(), &projects[..16], 200, 100),
            Err(CredentialError::Capacity)
        ));
        nearly_full
            .verify(
                credential.identity(),
                &project(),
                credential.secret_bytes(),
                100,
            )
            .unwrap();
    }
}
