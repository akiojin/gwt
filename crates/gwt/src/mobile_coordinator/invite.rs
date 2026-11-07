//! Bounded, single-use pairing invitations (Issue #6476).
//!
//! This unconnected foundation grants no device or execution authority. Before
//! every operation, the future signaling adapter must verify the authenticated
//! Host's live registration, PC enable state and lease, and supply its trusted
//! scope and monotonic clock. Browser-supplied destinations are not authority.
//! On disable, lease loss or disconnect, invalidate that registration and close
//! its connections. Restart discards all invitations; never restore old tokens.

use gwt_core::repo_hash::ProjectKey;
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use uuid::Uuid;

const MAX_TTL_SECS: u64 = 300;
const MAX_HOST_PENDING: usize = 8;
const MAX_RECORDS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InviteError {
    #[error("invite scope requires non-nil Host, epoch and registration IDs")]
    InvalidScope,
    #[error("invite TTL must be between 1 and 300 seconds")]
    InvalidTtl,
    #[error("invite connection ID must be non-nil and unused until its prior invite expires")]
    InvalidConnection,
    #[error("invite is already registered; use a fresh PC-generated secret")]
    AlreadyRegistered,
    #[error("Host has eight pending invites; close or claim one, or wait for expiry")]
    HostCapacity,
    #[error("invite registry holds 64 unexpired records including replay fences; wait for expiry")]
    Capacity,
    #[error("invite is invalid, expired, consumed or outside the current registration; request a new PC invite")]
    Unauthorized,
    #[error(
        "trusted invite clock moved backwards; start a fresh registration and invite registry"
    )]
    ClockRollback,
    #[error(
        "invite registry is unusable after clock rollback; start a fresh registration and registry"
    )]
    Unusable,
    #[error("invite randomness is unavailable; retry generation before registering or displaying an invite")]
    Unavailable,
}

/// Host-generated secret. Deliberately not Debug, Clone or serializable.
/// The Host must generate a fresh value for each pairing attempt. Never put it
/// in logs or persistent storage. Only its digest is retained by the registry.
pub struct InviteSecret([u8; 32]);

impl InviteSecret {
    pub fn generate() -> Result<Self, InviteError> {
        let mut bytes = [0; 32];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| InviteError::Unavailable)?;
        Ok(Self(bytes))
    }

    pub fn secret_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Non-secret routing context from an authenticated, enabled registration.
/// Constructing this value does not authenticate it. Registration generation
/// is a runtime lifetime, distinct from the credential registry's generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteScope {
    host_id: Uuid,
    project: ProjectKey,
    host_epoch: Uuid,
    registration_generation: Uuid,
}

impl InviteScope {
    pub fn new(
        host_id: Uuid,
        project: ProjectKey,
        host_epoch: Uuid,
        registration_generation: Uuid,
    ) -> Result<Self, InviteError> {
        if host_id.is_nil() || host_epoch.is_nil() || registration_generation.is_nil() {
            return Err(InviteError::InvalidScope);
        }
        Ok(Self {
            host_id,
            project,
            host_epoch,
            registration_generation,
        })
    }

    pub fn host_id(&self) -> Uuid {
        self.host_id
    }

    pub fn project(&self) -> &ProjectKey {
        &self.project
    }

    pub fn host_epoch(&self) -> Uuid {
        self.host_epoch
    }

    pub fn registration_generation(&self) -> Uuid {
        self.registration_generation
    }

    fn same_registration(&self, other: &Self) -> bool {
        self.host_id == other.host_id
            && self.host_epoch == other.host_epoch
            && self.registration_generation == other.registration_generation
    }
}

/// Correlation only, not proof of device approval, key possession or read/input
/// authority. Route using this saved scope, never a later browser destination.
#[derive(Debug, PartialEq, Eq)]
pub struct InviteClaim {
    pub scope: InviteScope,
    pub connection_id: Uuid,
}

struct InviteRecord {
    digest: [u8; 32],
    scope: InviteScope,
    expires_at: Instant,
    connection_id: Option<Uuid>,
    closed: bool,
}

/// Synchronous transitions must be serialized by the trusted coordinator.
/// All unexpired records, including claimed/closed replay fences, count toward
/// the global bound. Clock rollback is terminal; recovery requires a fresh
/// PC enable/authenticated registration and new registry, not old-token reuse.
pub struct InviteRegistry {
    records: Vec<InviteRecord>,
    last_observed: Instant,
    clock_failed: bool,
}

impl InviteRegistry {
    pub fn new(now: Instant) -> Self {
        Self {
            records: Vec::new(),
            last_observed: now,
            clock_failed: false,
        }
    }

    pub fn register_invite(
        &mut self,
        secret: &[u8; 32],
        scope: &InviteScope,
        ttl_secs: u64,
        now: Instant,
    ) -> Result<(), InviteError> {
        self.observe(now)?;
        if !(1..=MAX_TTL_SECS).contains(&ttl_secs) {
            return Err(InviteError::InvalidTtl);
        }
        let expires_at = now
            .checked_add(Duration::from_secs(ttl_secs))
            .ok_or(InviteError::InvalidTtl)?;
        let digest: [u8; 32] = Sha256::digest(secret).into();
        if self
            .records
            .iter()
            .any(|r| bool::from(r.digest.ct_eq(&digest)))
        {
            return Err(InviteError::AlreadyRegistered);
        }
        if self.records.len() >= MAX_RECORDS {
            return Err(InviteError::Capacity);
        }
        if self
            .records
            .iter()
            .filter(|r| r.scope.host_id == scope.host_id && !r.closed && r.connection_id.is_none())
            .count()
            >= MAX_HOST_PENDING
        {
            return Err(InviteError::HostCapacity);
        }
        self.records.push(InviteRecord {
            digest,
            scope: scope.clone(),
            expires_at,
            connection_id: None,
            closed: false,
        });
        Ok(())
    }

    /// The connection ID is generated by the trusted adapter, never chosen by
    /// a browser. Invalid claims do not consume another connection's invite.
    pub fn claim_invite(
        &mut self,
        secret: &[u8],
        expected: &InviteScope,
        connection_id: Uuid,
        now: Instant,
    ) -> Result<InviteClaim, InviteError> {
        self.observe(now)?;
        if connection_id.is_nil() {
            return Err(InviteError::InvalidConnection);
        }
        if secret.len() != 32 {
            return Err(InviteError::Unauthorized);
        }
        let digest: [u8; 32] = Sha256::digest(secret).into();
        let index = self
            .records
            .iter()
            .position(|r| {
                bool::from(r.digest.ct_eq(&digest))
                    && r.scope == *expected
                    && !r.closed
                    && r.connection_id.is_none()
            })
            .ok_or(InviteError::Unauthorized)?;
        if self
            .records
            .iter()
            .any(|r| r.connection_id == Some(connection_id))
        {
            return Err(InviteError::InvalidConnection);
        }
        let record = &mut self.records[index];
        record.connection_id = Some(connection_id);
        Ok(InviteClaim {
            scope: record.scope.clone(),
            connection_id,
        })
    }

    pub fn close_connection(
        &mut self,
        connection_id: Uuid,
        now: Instant,
    ) -> Result<(), InviteError> {
        self.observe(now)?;
        for record in &mut self.records {
            if record.connection_id == Some(connection_id) {
                record.closed = true;
            }
        }
        Ok(())
    }

    /// Close all Projects in exactly this registration. A delayed close from
    /// an old registration must not close a newer generation or Host epoch.
    /// The adapter must also reject future operations from the dead registration.
    pub fn invalidate_registration(
        &mut self,
        scope: &InviteScope,
        now: Instant,
    ) -> Result<(), InviteError> {
        self.observe(now)?;
        for record in &mut self.records {
            if record.scope.same_registration(scope) {
                record.closed = true;
            }
        }
        Ok(())
    }

    fn observe(&mut self, now: Instant) -> Result<(), InviteError> {
        if self.clock_failed {
            return Err(InviteError::Unusable);
        }
        if now < self.last_observed {
            self.clock_failed = true;
            return Err(InviteError::ClockRollback);
        }
        self.last_observed = now;
        self.records.retain(|r| now < r.expires_at);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::repo_hash::ProjectKey;
    use std::time::{Duration, Instant};
    use uuid::Uuid;

    fn invite_scope() -> InviteScope {
        InviteScope::new(
            Uuid::new_v4(),
            ProjectKey::parse("0123456789abcdef").unwrap(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .unwrap()
    }

    #[test]
    fn claim_is_single_use_and_binds_saved_scope_and_connection() {
        let now = Instant::now();
        let mut registry = InviteRegistry::new(now);
        let scope = invite_scope();
        let secret = InviteSecret::generate().unwrap();
        registry
            .register_invite(secret.secret_bytes(), &scope, 300, now)
            .unwrap();
        let connection = Uuid::new_v4();
        for foreign in [
            InviteScope {
                host_id: Uuid::new_v4(),
                ..scope.clone()
            },
            InviteScope {
                project: ProjectKey::parse("fedcba9876543210").unwrap(),
                ..scope.clone()
            },
            InviteScope {
                host_epoch: Uuid::new_v4(),
                ..scope.clone()
            },
            InviteScope {
                registration_generation: Uuid::new_v4(),
                ..scope.clone()
            },
        ] {
            assert_eq!(
                registry.claim_invite(secret.secret_bytes(), &foreign, connection, now),
                Err(InviteError::Unauthorized)
            );
        }
        assert_eq!(
            registry.claim_invite(&[0; 31], &scope, connection, now),
            Err(InviteError::Unauthorized)
        );
        let mut wrong_secret = *secret.secret_bytes();
        wrong_secret[0] ^= 1;
        assert_eq!(
            registry.claim_invite(&wrong_secret, &scope, connection, now),
            Err(InviteError::Unauthorized)
        );
        assert_eq!(
            registry.claim_invite(secret.secret_bytes(), &scope, Uuid::nil(), now),
            Err(InviteError::InvalidConnection)
        );
        let claim = registry
            .claim_invite(secret.secret_bytes(), &scope, connection, now)
            .unwrap();
        assert_eq!(claim.scope, scope);
        assert_eq!(claim.connection_id, connection);
        assert_eq!(
            registry.claim_invite(secret.secret_bytes(), &scope, connection, now),
            Err(InviteError::Unauthorized)
        );
        let next = InviteSecret::generate().unwrap();
        registry
            .register_invite(next.secret_bytes(), &scope, 300, now)
            .unwrap();
        assert_eq!(
            registry.claim_invite(next.secret_bytes(), &scope, connection, now),
            Err(InviteError::InvalidConnection)
        );
        assert!(registry
            .claim_invite(next.secret_bytes(), &scope, Uuid::new_v4(), now)
            .is_ok());
        assert_eq!(
            InviteScope::new(
                Uuid::nil(),
                scope.project,
                scope.host_epoch,
                scope.registration_generation
            ),
            Err(InviteError::InvalidScope)
        );
    }

    #[test]
    fn close_and_registration_invalidation_preserve_replay_fences() {
        let now = Instant::now();
        let mut registry = InviteRegistry::new(now);
        let scope = invite_scope();
        let other_project = InviteScope {
            project: ProjectKey::parse("fedcba9876543210").unwrap(),
            ..scope.clone()
        };
        let new_registration = InviteScope {
            registration_generation: Uuid::new_v4(),
            ..scope.clone()
        };
        let new_epoch = InviteScope {
            host_epoch: Uuid::new_v4(),
            ..scope.clone()
        };
        for (n, target) in [&scope, &other_project, &new_registration, &new_epoch]
            .into_iter()
            .enumerate()
        {
            registry
                .register_invite(InviteSecret([n as u8; 32]).secret_bytes(), target, 300, now)
                .unwrap();
        }
        let connection = Uuid::new_v4();
        registry
            .claim_invite(&[0; 32], &scope, connection, now)
            .unwrap();
        registry.close_connection(connection, now).unwrap();
        assert_eq!(
            registry.claim_invite(&[0; 32], &scope, Uuid::new_v4(), now),
            Err(InviteError::Unauthorized)
        );
        registry.invalidate_registration(&scope, now).unwrap();
        assert_eq!(
            registry.claim_invite(&[1; 32], &other_project, Uuid::new_v4(), now),
            Err(InviteError::Unauthorized)
        );
        assert_eq!(
            registry.register_invite(InviteSecret([1; 32]).secret_bytes(), &scope, 300, now),
            Err(InviteError::AlreadyRegistered)
        );
        assert!(registry
            .claim_invite(&[2; 32], &new_registration, Uuid::new_v4(), now)
            .is_ok());
        assert!(registry
            .claim_invite(&[3; 32], &new_epoch, Uuid::new_v4(), now)
            .is_ok());
        let fresh = InviteSecret::generate().unwrap();
        registry
            .register_invite(fresh.secret_bytes(), &new_registration, 300, now)
            .unwrap();
        assert!(registry
            .claim_invite(fresh.secret_bytes(), &new_registration, Uuid::new_v4(), now)
            .is_ok());
    }

    #[test]
    fn duplicate_registration_cannot_extend_deadline_and_fresh_invite_recovers() {
        let now = Instant::now();
        let mut registry = InviteRegistry::new(now);
        let scope = invite_scope();
        let secret = InviteSecret::generate().unwrap();
        assert_eq!(
            registry.register_invite(secret.secret_bytes(), &scope, 0, now),
            Err(InviteError::InvalidTtl)
        );
        assert_eq!(
            registry.register_invite(secret.secret_bytes(), &scope, 301, now),
            Err(InviteError::InvalidTtl)
        );
        registry
            .register_invite(secret.secret_bytes(), &scope, 10, now)
            .unwrap();
        let later = now + Duration::from_secs(9);
        assert_eq!(
            registry.register_invite(secret.secret_bytes(), &scope, 300, later),
            Err(InviteError::AlreadyRegistered)
        );
        let expired = now + Duration::from_secs(10);
        assert_eq!(
            registry.claim_invite(secret.secret_bytes(), &scope, Uuid::new_v4(), expired),
            Err(InviteError::Unauthorized)
        );
        let fresh = InviteSecret::generate().unwrap();
        registry
            .register_invite(fresh.secret_bytes(), &scope, 300, expired)
            .unwrap();
        assert!(registry
            .claim_invite(fresh.secret_bytes(), &scope, Uuid::new_v4(), expired)
            .is_ok());
    }

    #[test]
    fn host_pending_and_global_retained_limits_recover_only_with_capacity() {
        let now = Instant::now();
        let mut registry = InviteRegistry::new(now);
        let scope = invite_scope();
        for n in 0..8 {
            registry
                .register_invite(InviteSecret([n; 32]).secret_bytes(), &scope, 300, now)
                .unwrap();
        }
        let changed = InviteScope {
            project: ProjectKey::parse("fedcba9876543210").unwrap(),
            host_epoch: Uuid::new_v4(),
            registration_generation: Uuid::new_v4(),
            ..scope.clone()
        };
        assert_eq!(
            registry.register_invite(InviteSecret([8; 32]).secret_bytes(), &changed, 300, now),
            Err(InviteError::HostCapacity)
        );
        registry
            .claim_invite(&[0; 32], &scope, Uuid::new_v4(), now)
            .unwrap();
        registry
            .register_invite(InviteSecret([8; 32]).secret_bytes(), &changed, 300, now)
            .unwrap();

        let mut full = InviteRegistry::new(now);
        let scopes: Vec<_> = (0..64).map(|_| invite_scope()).collect();
        for (n, scope) in scopes.iter().enumerate() {
            full.register_invite(InviteSecret([n as u8; 32]).secret_bytes(), scope, 300, now)
                .unwrap();
        }
        let connection = Uuid::new_v4();
        full.claim_invite(&[0; 32], &scopes[0], connection, now)
            .unwrap();
        assert_eq!(
            full.register_invite(InviteSecret([64; 32]).secret_bytes(), &scope, 300, now),
            Err(InviteError::Capacity)
        );
        full.close_connection(connection, now).unwrap();
        assert_eq!(
            full.register_invite(InviteSecret([64; 32]).secret_bytes(), &scope, 300, now),
            Err(InviteError::Capacity)
        );
        let expired = now + Duration::from_secs(300);
        full.register_invite(InviteSecret([64; 32]).secret_bytes(), &scope, 300, expired)
            .unwrap();
        registry
            .register_invite(InviteSecret([9; 32]).secret_bytes(), &scope, 300, expired)
            .unwrap();
    }

    #[test]
    fn backwards_clock_is_terminal_and_new_registration_starts_empty() {
        let now = Instant::now();
        let mut registry = InviteRegistry::new(now);
        let scope = invite_scope();
        let secret = InviteSecret::generate().unwrap();
        registry
            .register_invite(secret.secret_bytes(), &scope, 300, now)
            .unwrap();
        registry
            .close_connection(Uuid::new_v4(), now + Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            registry.claim_invite(secret.secret_bytes(), &scope, Uuid::new_v4(), now),
            Err(InviteError::ClockRollback)
        );
        assert_eq!(
            registry.claim_invite(
                secret.secret_bytes(),
                &scope,
                Uuid::new_v4(),
                now + Duration::from_secs(2)
            ),
            Err(InviteError::Unusable)
        );
        let mut fresh = InviteRegistry::new(now + Duration::from_secs(2));
        assert_eq!(
            fresh.claim_invite(
                secret.secret_bytes(),
                &scope,
                Uuid::new_v4(),
                now + Duration::from_secs(2)
            ),
            Err(InviteError::Unauthorized)
        );
        let target = InviteScope {
            registration_generation: Uuid::new_v4(),
            ..scope
        };
        let new_secret = InviteSecret::generate().unwrap();
        fresh
            .register_invite(
                new_secret.secret_bytes(),
                &target,
                300,
                now + Duration::from_secs(2),
            )
            .unwrap();
        assert!(fresh
            .claim_invite(
                new_secret.secret_bytes(),
                &target,
                Uuid::new_v4(),
                now + Duration::from_secs(2)
            )
            .is_ok());
    }
}
