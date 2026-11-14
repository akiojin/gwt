//! Unconnected coordinator registration and invitation gate (Issue #6476).
//!
//! A trusted, serialized actor owns this value and its opaque handles. This is
//! not a listener or proof of PC enable, TLS peer identity or device authority.
//! The future actor must register only after explicit PC enable/authenticated
//! transport, call `close` on disable/EOF, close actual sockets, and enforce the
//! Host's separate request-start/nonce lease and final read/input fence.
//! Trusted wall observations are checkpointed to the private store before
//! request validation. Storage failure closes the entire gate; recovery needs
//! explicit store reopen and fresh registrations, never old handle adoption.

use super::{
    invite::{InviteClaim, InviteError, InviteRegistry, InviteScope},
    storage::{CoordinatorStore, StoreError},
    CredentialError, CredentialIdentity,
};
use gwt_core::repo_hash::ProjectKey;
use std::time::{Duration, Instant};
use uuid::Uuid;

const MAX_REGISTRATIONS: usize = 64;
const LEASE_SECS: u64 = 30;

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("Host epoch must be non-nil; start a fresh explicit PC enable")]
    InvalidEpoch,
    #[error(
        "registration is foreign, closed or expired; authenticate a fresh registration and invite"
    )]
    DeadRegistration,
    #[error("64 live registrations are retained; close one or wait for its lease before authenticating again")]
    Capacity,
    #[error("trusted registration clock moved backwards or is unavailable; repair the clock and explicitly reopen the store and gate")]
    ClockRollback,
    #[error("private coordinator storage is unavailable; repair storage and explicitly reopen the store and gate")]
    StorageUnavailable,
    #[error("coordinator gate is terminal; repair its cause and explicitly reopen the store with fresh registrations and invites")]
    Unusable,
    #[error(transparent)]
    Credential(CredentialError),
    #[error(transparent)]
    Invite(InviteError),
}

/// A trusted actor's correlation handle, never a browser-supplied destination.
/// No constructor, bearer, Clone, Debug, serialization or cross-gate adoption.
pub struct RegistrationHandle {
    owner: Uuid,
    generation: Uuid,
}

impl RegistrationHandle {
    /// Non-secret correlation value the Host's lease matches replies against.
    pub fn generation(&self) -> Uuid {
        self.generation
    }
}

struct Registration {
    identity: CredentialIdentity,
    secret: [u8; 32],
    scope: InviteScope,
    credential_expires_at: u64,
    deadline: Instant,
}

/// Owns the private store guard for its entire lifetime. Bearers remain only in
/// private RAM for live re-verification; no state or handle is serializable.
pub struct CoordinatorSessions {
    store: CoordinatorStore,
    invites: InviteRegistry,
    registrations: Vec<Registration>,
    owner: Uuid,
    last_instant: Instant,
    last_wall: u64,
    failed: bool,
}

impl CoordinatorSessions {
    pub fn new(store: CoordinatorStore, now: Instant, wall: u64) -> Self {
        Self {
            store,
            invites: InviteRegistry::new(now),
            registrations: Vec::new(),
            owner: Uuid::new_v4(),
            last_instant: now,
            last_wall: wall,
            failed: false,
        }
    }

    /// Authenticate the selected Project before retaining any bearer. The
    /// epoch comes from trusted PC state, not a mobile message or enable bool.
    pub fn register(
        &mut self,
        identity: &CredentialIdentity,
        project: &ProjectKey,
        secret: &[u8],
        host_epoch: Uuid,
        now: Instant,
        wall: u64,
    ) -> Result<RegistrationHandle, RegistrationError> {
        self.observe(now, wall)?;
        if host_epoch.is_nil() {
            return Err(RegistrationError::InvalidEpoch);
        }
        let result = self.store.registry().and_then(|registry| {
            registry
                .verify_expiry(identity, project, secret, wall)
                .map_err(StoreError::from)
        });
        let expiry = result.map_err(|error| self.auth_error(error, now))?;
        if self.registrations.len() >= MAX_REGISTRATIONS {
            return Err(RegistrationError::Capacity);
        }
        let generation = Uuid::new_v4();
        let scope = InviteScope::new(identity.host_id, project.clone(), host_epoch, generation)
            .map_err(RegistrationError::Invite)?;
        let deadline = self.deadline(expiry, now, wall)?;
        self.registrations.push(Registration {
            identity: *identity,
            secret: secret
                .try_into()
                .map_err(|_| RegistrationError::Credential(CredentialError::Unauthorized))?,
            scope,
            credential_expires_at: expiry,
            deadline,
        });
        Ok(RegistrationHandle {
            owner: self.owner,
            generation,
        })
    }

    /// Coordinator-side synchronous renewal only. Host response delay and
    /// one-shot nonce validation remain the separate RegistrationLease contract.
    pub fn renew(
        &mut self,
        handle: &RegistrationHandle,
        now: Instant,
        wall: u64,
    ) -> Result<(), RegistrationError> {
        let index = self.check(handle, now, wall)?;
        self.registrations[index].deadline =
            self.deadline(self.registrations[index].credential_expires_at, now, wall)?;
        Ok(())
    }

    pub fn register_invite(
        &mut self,
        handle: &RegistrationHandle,
        secret: &[u8; 32],
        ttl_secs: u64,
        now: Instant,
        wall: u64,
    ) -> Result<(), RegistrationError> {
        let index = self.check(handle, now, wall)?;
        let result =
            self.invites
                .register_invite(secret, &self.registrations[index].scope, ttl_secs, now);
        self.invite_result(result, now)
    }

    /// The routing actor supplies its saved handle. A browser may supply the
    /// invite secret, but cannot construct a handle from its claimed scope.
    pub fn claim_invite(
        &mut self,
        handle: &RegistrationHandle,
        secret: &[u8],
        connection_id: Uuid,
        now: Instant,
        wall: u64,
    ) -> Result<InviteClaim, RegistrationError> {
        let index = self.check(handle, now, wall)?;
        let result =
            self.invites
                .claim_invite(secret, &self.registrations[index].scope, connection_id, now);
        self.invite_result(result, now)
    }

    pub fn close_connection(
        &mut self,
        handle: &RegistrationHandle,
        connection_id: Uuid,
        now: Instant,
        wall: u64,
    ) -> Result<(), RegistrationError> {
        let index = self.check(handle, now, wall)?;
        let result = self.invites.close_scoped_connection(
            &self.registrations[index].scope,
            connection_id,
            now,
        );
        self.invite_result(result, now)
    }

    /// Logical close is idempotent and precedes caller-owned socket cleanup.
    /// A delayed old/foreign handle cannot close a replacement registration.
    pub fn close(
        &mut self,
        handle: &RegistrationHandle,
        now: Instant,
        wall: u64,
    ) -> Result<(), RegistrationError> {
        self.observe(now, wall)?;
        if let Some(index) = self.index(handle) {
            self.retire(index, now)?;
        }
        Ok(())
    }

    /// Read-only: the current coordinator lease deadline of a live handle. It
    /// never extends or validates the lease; operations still re-verify.
    pub fn lease_deadline(&self, handle: &RegistrationHandle) -> Option<Instant> {
        self.index(handle)
            .map(|index| self.registrations[index].deadline)
    }

    #[cfg(test)]
    pub(super) fn live_registrations(&self) -> usize {
        self.registrations.len()
    }

    fn index(&self, handle: &RegistrationHandle) -> Option<usize> {
        (handle.owner == self.owner)
            .then(|| {
                self.registrations
                    .iter()
                    .position(|r| r.scope.registration_generation() == handle.generation)
            })
            .flatten()
    }

    fn check(
        &mut self,
        handle: &RegistrationHandle,
        now: Instant,
        wall: u64,
    ) -> Result<usize, RegistrationError> {
        self.observe(now, wall)?;
        let index = self
            .index(handle)
            .ok_or(RegistrationError::DeadRegistration)?;
        let registration = &self.registrations[index];
        let result = self.store.registry().and_then(|registry| {
            registry
                .verify(
                    &registration.identity,
                    registration.scope.project(),
                    &registration.secret,
                    wall,
                )
                .map_err(StoreError::from)
        });
        if let Err(error) = result {
            let error = self.auth_error(error, now);
            if !self.failed {
                self.retire(index, now)?;
            }
            return Err(error);
        }
        Ok(index)
    }

    fn observe(&mut self, now: Instant, wall: u64) -> Result<(), RegistrationError> {
        if self.failed {
            return Err(RegistrationError::Unusable);
        }
        if now < self.last_instant || wall < self.last_wall {
            return Err(self.stop(RegistrationError::ClockRollback, now));
        }
        if let Err(error) = self.store.checkpoint(wall) {
            let error = match error {
                StoreError::Credential(CredentialError::ClockRollback) => {
                    RegistrationError::ClockRollback
                }
                _ => RegistrationError::StorageUnavailable,
            };
            return Err(self.stop(error, now));
        }
        self.last_instant = now;
        self.last_wall = wall;
        let mut index = 0;
        while index < self.registrations.len() {
            let r = &self.registrations[index];
            if now >= r.deadline || wall >= r.credential_expires_at {
                self.retire(index, now)?;
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    fn retire(&mut self, index: usize, now: Instant) -> Result<(), RegistrationError> {
        let result = self
            .invites
            .invalidate_registration(&self.registrations[index].scope, now);
        self.invite_result(result, now)?;
        self.registrations.swap_remove(index);
        Ok(())
    }

    fn deadline(
        &mut self,
        expiry: u64,
        now: Instant,
        wall: u64,
    ) -> Result<Instant, RegistrationError> {
        now.checked_add(Duration::from_secs(LEASE_SECS.min(expiry - wall)))
            .ok_or_else(|| self.stop(RegistrationError::ClockRollback, now))
    }

    fn auth_error(&mut self, error: StoreError, now: Instant) -> RegistrationError {
        match error {
            StoreError::Credential(CredentialError::ClockRollback) => {
                self.stop(RegistrationError::ClockRollback, now)
            }
            StoreError::Credential(error) => RegistrationError::Credential(error),
            _ => self.stop(RegistrationError::StorageUnavailable, now),
        }
    }

    fn invite_result<T>(
        &mut self,
        result: Result<T, InviteError>,
        now: Instant,
    ) -> Result<T, RegistrationError> {
        match result {
            Err(InviteError::ClockRollback | InviteError::Unusable) => {
                Err(self.stop(RegistrationError::ClockRollback, now))
            }
            other => other.map_err(RegistrationError::Invite),
        }
    }

    fn stop(&mut self, error: RegistrationError, now: Instant) -> RegistrationError {
        self.failed = true;
        self.registrations.clear();
        self.invites = InviteRegistry::new(now);
        error
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{
        mobile_connection::HostBinding,
        mobile_coordinator::{storage::CoordinatorStore, CredentialError, CredentialIdentity},
        mobile_host::credential::{load_host_credential, LoadedHostCredential},
    };
    use gwt_core::repo_hash::ProjectKey;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    use uuid::Uuid;

    fn fixture(expiry: u64) -> (tempfile::TempDir, CoordinatorSessions, LoadedHostCredential) {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = CoordinatorStore::initialize(directory.path(), 100).unwrap();
        let project = ProjectKey::parse("0123456789abcdef").unwrap();
        let issued = store
            .issue(
                "https://mobile.example",
                Uuid::new_v4(),
                std::slice::from_ref(&project),
                expiry,
                100,
            )
            .unwrap();
        let binding = HostBinding::new(
            &issued.identity.host_id.to_string(),
            "https://mobile.example",
        )
        .unwrap();
        let credential = load_host_credential(&issued.path, &binding, &project, 100).unwrap();
        let sessions = CoordinatorSessions::new(store, Instant::now(), 100);
        (directory, sessions, credential)
    }

    fn register(
        sessions: &mut CoordinatorSessions,
        credential: &LoadedHostCredential,
        now: Instant,
        wall: u64,
    ) -> RegistrationHandle {
        sessions
            .register(
                credential.identity(),
                credential.project(),
                credential.secret_bytes(),
                Uuid::new_v4(),
                now,
                wall,
            )
            .unwrap()
    }

    #[test]
    fn registration_observations_survive_restart_including_rejected_requests() {
        use std::os::unix::fs::MetadataExt;

        let (directory, mut sessions, credential) = fixture(200);
        let now = sessions.last_instant;
        register(&mut sessions, &credential, now, 110);
        drop(sessions);
        assert!(matches!(
            CoordinatorStore::open(directory.path(), 109),
            Err(StoreError::Credential(CredentialError::ClockRollback))
        ));

        let store = CoordinatorStore::open(directory.path(), 110).unwrap();
        let mut sessions = CoordinatorSessions::new(store, now, 110);
        assert!(matches!(
            sessions.register(
                credential.identity(),
                credential.project(),
                credential.secret_bytes(),
                Uuid::nil(),
                now,
                120,
            ),
            Err(RegistrationError::InvalidEpoch)
        ));
        drop(sessions);
        assert!(matches!(
            CoordinatorStore::open(directory.path(), 119),
            Err(StoreError::Credential(CredentialError::ClockRollback))
        ));

        let path = directory.path().join("registry.json");
        let before = fs::metadata(&path).unwrap().ino();
        let store = CoordinatorStore::open(directory.path(), 130).unwrap();
        let mut sessions = CoordinatorSessions::new(store, now, 130);
        assert!(matches!(
            sessions.register(
                credential.identity(),
                credential.project(),
                &[0; 32],
                Uuid::new_v4(),
                now,
                130,
            ),
            Err(RegistrationError::Credential(CredentialError::Unauthorized))
        ));
        let checkpointed = fs::metadata(&path).unwrap().ino();
        assert_ne!(before, checkpointed, "open's RAM-only time must be saved");
        let handle = register(&mut sessions, &credential, now, 130);
        sessions
            .register_invite(&handle, &[1; 32], 300, now, 130)
            .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().ino(), checkpointed);
        drop(sessions);
        assert!(matches!(
            CoordinatorStore::open(directory.path(), 129),
            Err(StoreError::Credential(CredentialError::ClockRollback))
        ));
        let store = CoordinatorStore::open(directory.path(), 130).unwrap();
        let mut fresh = CoordinatorSessions::new(store, now, 130);
        register(&mut fresh, &credential, now, 130);
    }

    #[test]
    fn checkpoint_failure_closes_the_gate_until_explicit_reopen() {
        let (directory, mut sessions, credential) = fixture(200);
        let now = sessions.last_instant;
        let handle = register(&mut sessions, &credential, now, 100);
        sessions
            .register_invite(&handle, &[2; 32], 300, now, 100)
            .unwrap();
        let path = directory.path().join("registry.json");
        let original = fs::read(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            sessions.renew(&handle, now, 110),
            Err(RegistrationError::StorageUnavailable)
        ));
        assert!(sessions.registrations.is_empty());
        assert!(matches!(
            sessions.store.registry(),
            Err(StoreError::Unusable)
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            sessions.claim_invite(&handle, &[2; 32], Uuid::new_v4(), now, 110),
            Err(RegistrationError::Unusable)
        ));
        drop(sessions);
        let store = CoordinatorStore::open(directory.path(), 110).unwrap();
        let mut fresh = CoordinatorSessions::new(store, now, 110);
        assert!(matches!(
            fresh.renew(&handle, now, 110),
            Err(RegistrationError::DeadRegistration)
        ));
        let handle = register(&mut fresh, &credential, now, 110);
        fresh
            .register_invite(&handle, &[3; 32], 300, now, 110)
            .unwrap();
        fresh
            .claim_invite(&handle, &[3; 32], Uuid::new_v4(), now, 110)
            .unwrap();
    }

    #[test]
    fn authenticates_full_scope_and_routes_only_with_owned_handles() {
        let (_directory, mut sessions, credential) = fixture(200);
        let now = sessions.last_instant;
        for identity in [
            CredentialIdentity {
                host_id: Uuid::new_v4(),
                ..*credential.identity()
            },
            CredentialIdentity {
                credential_id: Uuid::new_v4(),
                ..*credential.identity()
            },
            CredentialIdentity {
                generation: Uuid::new_v4(),
                ..*credential.identity()
            },
        ] {
            assert!(matches!(
                sessions.register(
                    &identity,
                    credential.project(),
                    credential.secret_bytes(),
                    Uuid::new_v4(),
                    now,
                    100
                ),
                Err(RegistrationError::Credential(CredentialError::Unauthorized))
            ));
        }
        let foreign_project = ProjectKey::parse("fedcba9876543210").unwrap();
        assert!(matches!(
            sessions.register(
                credential.identity(),
                &foreign_project,
                credential.secret_bytes(),
                Uuid::new_v4(),
                now,
                100
            ),
            Err(RegistrationError::Credential(CredentialError::Unauthorized))
        ));
        assert!(sessions
            .register(
                credential.identity(),
                credential.project(),
                &[0; 31],
                Uuid::new_v4(),
                now,
                100
            )
            .is_err());
        assert!(sessions
            .register(
                credential.identity(),
                credential.project(),
                &[0; 32],
                Uuid::new_v4(),
                now,
                100
            )
            .is_err());
        assert!(matches!(
            sessions.register(
                credential.identity(),
                credential.project(),
                credential.secret_bytes(),
                Uuid::nil(),
                now,
                100
            ),
            Err(RegistrationError::InvalidEpoch)
        ));
        let handle = register(&mut sessions, &credential, now, 100);
        let other = register(&mut sessions, &credential, now, 100);
        let (_foreign_directory, mut foreign_sessions, foreign_credential) = fixture(200);
        let foreign_now = foreign_sessions.last_instant;
        let foreign = register(&mut foreign_sessions, &foreign_credential, foreign_now, 100);
        assert!(matches!(
            sessions.register_invite(&foreign, &[1; 32], 300, now, 100),
            Err(RegistrationError::DeadRegistration)
        ));
        sessions
            .register_invite(&handle, &[1; 32], 300, now, 100)
            .unwrap();
        assert!(sessions
            .claim_invite(&other, &[1; 32], Uuid::new_v4(), now, 100)
            .is_err());
        let connection = Uuid::new_v4();
        let claim = sessions
            .claim_invite(&handle, &[1; 32], connection, now, 100)
            .unwrap();
        assert_eq!(claim.scope.host_id(), credential.identity().host_id);
        assert_eq!(claim.scope.project(), credential.project());
        assert_eq!(claim.connection_id, connection);
        assert!(sessions
            .claim_invite(&handle, &[1; 32], connection, now, 100)
            .is_err());
        assert!(sessions
            .close_connection(&other, connection, now, 100)
            .is_err());
        sessions
            .close_connection(&handle, connection, now, 100)
            .unwrap();
    }

    #[test]
    fn close_and_lease_expiry_never_revive_old_registrations() {
        let (_directory, mut sessions, credential) = fixture(200);
        let now = sessions.last_instant;
        let old = register(&mut sessions, &credential, now, 100);
        sessions
            .register_invite(&old, &[2; 32], 300, now, 100)
            .unwrap();
        sessions.close(&old, now, 100).unwrap();
        assert!(matches!(
            sessions.register_invite(&old, &[3; 32], 300, now, 100),
            Err(RegistrationError::DeadRegistration)
        ));
        assert!(sessions
            .claim_invite(&old, &[2; 32], Uuid::new_v4(), now, 100)
            .is_err());
        assert!(sessions.renew(&old, now, 100).is_err());
        let fresh = register(&mut sessions, &credential, now, 100);
        sessions.close(&old, now, 100).unwrap();
        assert!(sessions
            .claim_invite(&fresh, &[2; 32], Uuid::new_v4(), now, 100)
            .is_err());
        sessions
            .register_invite(&fresh, &[4; 32], 300, now, 100)
            .unwrap();
        sessions
            .renew(&fresh, now + Duration::from_secs(10), 110)
            .unwrap();
        assert!(sessions
            .claim_invite(
                &fresh,
                &[4; 32],
                Uuid::new_v4(),
                now + Duration::from_secs(39),
                139
            )
            .is_ok());
        assert!(matches!(
            sessions.renew(&fresh, now + Duration::from_secs(40), 140),
            Err(RegistrationError::DeadRegistration)
        ));
        assert!(sessions
            .register_invite(&fresh, &[5; 32], 300, now + Duration::from_secs(40), 140)
            .is_err());
        let next = register(
            &mut sessions,
            &credential,
            now + Duration::from_secs(40),
            140,
        );
        sessions
            .register_invite(&next, &[6; 32], 300, now + Duration::from_secs(40), 140)
            .unwrap();
    }

    #[test]
    fn credentials_clocks_and_storage_fail_closed_with_explicit_recovery() {
        let (_directory, mut sessions, credential) = fixture(105);
        let now = sessions.last_instant;
        let handle = register(&mut sessions, &credential, now, 100);
        sessions
            .renew(&handle, now + Duration::from_secs(2), 102)
            .unwrap();
        assert!(sessions
            .renew(&handle, now + Duration::from_secs(5), 102)
            .is_err());
        assert!(sessions
            .register(
                credential.identity(),
                credential.project(),
                credential.secret_bytes(),
                Uuid::new_v4(),
                now + Duration::from_secs(5),
                105
            )
            .is_err());

        let (_directory, mut sessions, credential) = fixture(200);
        let now = sessions.last_instant;
        let handle = register(&mut sessions, &credential, now, 100);
        sessions
            .register_invite(&handle, &[7; 32], 300, now, 100)
            .unwrap();
        sessions
            .store
            .revoke(credential.identity().credential_id, 100)
            .unwrap();
        assert!(matches!(
            sessions.register_invite(&handle, &[8; 32], 300, now, 100),
            Err(RegistrationError::Credential(CredentialError::Unauthorized))
        ));
        assert!(matches!(
            sessions.renew(&handle, now, 100),
            Err(RegistrationError::DeadRegistration)
        ));

        for rollback_wall in [false, true] {
            let (_directory, mut sessions, credential) = fixture(200);
            let now = sessions.last_instant;
            let handle = register(&mut sessions, &credential, now, 100);
            sessions
                .renew(&handle, now + Duration::from_secs(1), 101)
                .unwrap();
            let (instant, wall) = if rollback_wall {
                (now + Duration::from_secs(2), 100)
            } else {
                (now, 101)
            };
            assert!(matches!(
                sessions.renew(&handle, instant, wall),
                Err(RegistrationError::ClockRollback)
            ));
            assert!(matches!(
                sessions.register_invite(&handle, &[9; 32], 300, now + Duration::from_secs(3), 103),
                Err(RegistrationError::Unusable)
            ));
            assert!(sessions.registrations.is_empty());
        }
        let (directory, mut sessions, credential) = fixture(200);
        let now = sessions.last_instant;
        let handle = register(&mut sessions, &credential, now, 100);
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            sessions.renew(&handle, now, 100),
            Err(RegistrationError::StorageUnavailable)
        ));
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            sessions.renew(&handle, now, 100),
            Err(RegistrationError::Unusable)
        ));
        drop(sessions);
        let store = CoordinatorStore::open(directory.path(), 100).unwrap();
        let mut fresh = CoordinatorSessions::new(store, now, 100);
        assert!(fresh.renew(&handle, now, 100).is_err());
        let next = register(&mut fresh, &credential, now, 100);
        fresh
            .register_invite(&next, &[10; 32], 300, now, 100)
            .unwrap();
    }

    #[test]
    fn registration_capacity_recovers_by_close_and_expiry() {
        let (_directory, mut sessions, credential) = fixture(200);
        let now = sessions.last_instant;
        let handles: Vec<_> = (0..64)
            .map(|_| register(&mut sessions, &credential, now, 100))
            .collect();
        assert!(matches!(
            sessions.register(
                credential.identity(),
                credential.project(),
                credential.secret_bytes(),
                Uuid::new_v4(),
                now,
                100
            ),
            Err(RegistrationError::Capacity)
        ));
        sessions.close(&handles[0], now, 100).unwrap();
        let next = register(&mut sessions, &credential, now, 100);
        sessions
            .register_invite(&next, &[11; 32], 300, now, 100)
            .unwrap();
        let later = now + Duration::from_secs(30);
        let recovered = register(&mut sessions, &credential, later, 130);
        assert_eq!(sessions.registrations.len(), 1);
        assert!(sessions
            .claim_invite(&recovered, &[11; 32], Uuid::new_v4(), later, 130)
            .is_err());
        assert!(sessions.renew(&next, later, 130).is_err());
        sessions
            .register_invite(&recovered, &[12; 32], 300, later, 130)
            .unwrap();
    }
}
