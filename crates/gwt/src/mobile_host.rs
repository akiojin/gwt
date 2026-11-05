//! Host registration lease bookkeeping (Issue #6476).
//!
//! This foundation opens no transport and grants no device authorization. A
//! trusted caller must authenticate coordinator replies, own this value under
//! the Host authorization fence, and check it at the final read/input effect.
//! Network failure/EOF must invalidate it and disable the Host with a new epoch.
//! Renewal scheduling (every ten seconds) and that runtime wiring are separate.

use std::time::{Duration, Instant};
use uuid::Uuid;

const MAX_LEASE_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LeaseError {
    #[error("mobile registration generation must be non-nil")]
    InvalidRegistration,
    #[error("mobile registration is disabled; a fresh PC enable/registration is required")]
    Disabled,
    #[error("mobile registration clock moved backwards")]
    ClockRollback,
    #[error("mobile Host credential expired")]
    CredentialExpired,
    #[error("mobile registration lease or pending renewal expired")]
    LeaseExpired,
    #[error("mobile registration has no accepted lease")]
    NotRegistered,
    #[error("mobile renewal generation/nonce is stale or already consumed")]
    StaleRenewal,
    #[error("mobile lease TTL must be between 1 and 30 seconds")]
    InvalidTtl,
}

/// Non-secret correlation values, not proof of an authenticated reply.
#[derive(Debug, PartialEq, Eq)]
pub struct RegistrationRenewal {
    generation: Uuid,
    nonce: Uuid,
}

impl RegistrationRenewal {
    pub fn generation(&self) -> Uuid {
        self.generation
    }

    pub fn nonce(&self) -> Uuid {
        self.nonce
    }
}

struct PendingRenewal {
    request: RegistrationRenewal,
    started_at: Instant,
    maximum_deadline: Instant,
}

/// One registration generation, one outstanding renewal and one active lease.
/// Never clone/persist it or accept clocks/registration identity from a client.
/// Terminal failure requires a new value after explicit PC enable; a response
/// cannot revive this generation. Callers must preserve lock order:
/// session -> Host authorization -> lease, with no network wait under the fence.
pub struct RegistrationLease {
    generation: Uuid,
    credential_expires_at: u64,
    last_instant: Instant,
    last_wall: u64,
    pending: Option<PendingRenewal>,
    active_until: Option<Instant>,
    disabled: bool,
}

impl RegistrationLease {
    /// `wall` and credential expiry are Unix seconds from trusted Host state.
    pub fn new(
        generation: Uuid,
        credential_expires_at: u64,
        now: Instant,
        wall: u64,
    ) -> Result<Self, LeaseError> {
        if generation.is_nil() {
            return Err(LeaseError::InvalidRegistration);
        }
        if credential_expires_at <= wall {
            return Err(LeaseError::CredentialExpired);
        }
        Ok(Self {
            generation,
            credential_expires_at,
            last_instant: now,
            last_wall: wall,
            pending: None,
            active_until: None,
            disabled: false,
        })
    }

    /// Replace the sole pending request. Older responses cannot consume it.
    pub fn begin_renewal(
        &mut self,
        now: Instant,
        wall: u64,
    ) -> Result<RegistrationRenewal, LeaseError> {
        self.observe(now, wall)?;
        let nonce = Uuid::new_v4();
        let request = RegistrationRenewal {
            generation: self.generation,
            nonce,
        };
        self.pending = Some(PendingRenewal {
            request: RegistrationRenewal {
                generation: self.generation,
                nonce,
            },
            started_at: now,
            maximum_deadline: now
                + Duration::from_secs(MAX_LEASE_SECS.min(self.credential_expires_at - wall)),
        });
        Ok(request)
    }

    /// Accept only an authenticated response for the current generation/nonce.
    /// A matching response is consumed even when its TTL is invalid or elapsed.
    /// The deadline starts at the request, never at response arrival.
    pub fn accept_renewal(
        &mut self,
        generation: Uuid,
        nonce: Uuid,
        ttl_secs: u64,
        now: Instant,
        wall: u64,
    ) -> Result<(), LeaseError> {
        self.observe(now, wall)?;
        let Some(pending) = self.pending.as_ref().filter(|pending| {
            pending.request.generation == generation && pending.request.nonce == nonce
        }) else {
            return Err(LeaseError::StaleRenewal);
        };
        let started_at = pending.started_at;
        let maximum_deadline = pending.maximum_deadline;
        self.pending = None;
        if !(1..=MAX_LEASE_SECS).contains(&ttl_secs) {
            return Err(self.close(LeaseError::InvalidTtl));
        }
        let deadline = maximum_deadline.min(started_at + Duration::from_secs(ttl_secs));
        if now >= deadline {
            return Err(self.close(LeaseError::LeaseExpired));
        }
        self.active_until = Some(deadline);
        Ok(())
    }

    /// Check at the synchronous effect while holding the Host authority fence.
    pub fn check(&mut self, now: Instant, wall: u64) -> Result<(), LeaseError> {
        self.observe(now, wall)?;
        self.active_until
            .ok_or(LeaseError::NotRegistered)
            .map(|_| ())
    }

    pub fn invalidate(&mut self) {
        self.disabled = true;
        self.pending = None;
        self.active_until = None;
    }

    fn close(&mut self, error: LeaseError) -> LeaseError {
        self.invalidate();
        error
    }

    fn observe(&mut self, now: Instant, wall: u64) -> Result<(), LeaseError> {
        if self.disabled {
            return Err(LeaseError::Disabled);
        }
        if now < self.last_instant || wall < self.last_wall {
            return Err(self.close(LeaseError::ClockRollback));
        }
        self.last_instant = now;
        self.last_wall = wall;
        if wall >= self.credential_expires_at {
            return Err(self.close(LeaseError::CredentialExpired));
        }
        if self.active_until.is_some_and(|deadline| now >= deadline)
            || self
                .pending
                .as_ref()
                .is_some_and(|pending| now >= pending.maximum_deadline)
        {
            return Err(self.close(LeaseError::LeaseExpired));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use uuid::Uuid;

    fn lease(now: Instant, expires_at: u64) -> RegistrationLease {
        RegistrationLease::new(Uuid::new_v4(), expires_at, now, 100).unwrap()
    }

    fn grant(lease: &mut RegistrationLease, now: Instant, wall: u64) {
        let request = lease.begin_renewal(now, wall).unwrap();
        lease
            .accept_renewal(request.generation(), request.nonce(), 30, now, wall)
            .unwrap();
    }

    #[test]
    fn grant_deadline_starts_at_request_and_fresh_registration_recovers() {
        let now = Instant::now();
        let mut host = lease(now, 200);
        assert_eq!(host.check(now, 100), Err(LeaseError::NotRegistered));
        let request = host.begin_renewal(now, 100).unwrap();
        host.accept_renewal(
            request.generation(),
            request.nonce(),
            30,
            now + Duration::from_secs(20),
            120,
        )
        .unwrap();
        assert_eq!(host.check(now + Duration::from_secs(29), 129), Ok(()));
        assert_eq!(
            host.check(now + Duration::from_secs(30), 130),
            Err(LeaseError::LeaseExpired)
        );
        assert_eq!(
            host.begin_renewal(now + Duration::from_secs(31), 131),
            Err(LeaseError::Disabled)
        );
        let mut fresh = lease(now + Duration::from_secs(31), 200);
        grant(&mut fresh, now + Duration::from_secs(31), 131);
        assert_eq!(fresh.check(now + Duration::from_secs(32), 132), Ok(()));
    }

    #[test]
    fn renewal_consumes_only_the_current_generation_and_nonce_once() {
        let now = Instant::now();
        let mut host = lease(now, 200);
        grant(&mut host, now, 100);
        let old = host
            .begin_renewal(now + Duration::from_secs(10), 110)
            .unwrap();
        let current = host
            .begin_renewal(now + Duration::from_secs(11), 111)
            .unwrap();
        assert_ne!(old.nonce(), current.nonce());
        assert_eq!(
            host.accept_renewal(
                old.generation(),
                old.nonce(),
                30,
                now + Duration::from_secs(12),
                112
            ),
            Err(LeaseError::StaleRenewal)
        );
        assert_eq!(
            host.accept_renewal(
                Uuid::new_v4(),
                current.nonce(),
                30,
                now + Duration::from_secs(12),
                112
            ),
            Err(LeaseError::StaleRenewal)
        );
        host.accept_renewal(
            current.generation(),
            current.nonce(),
            30,
            now + Duration::from_secs(13),
            113,
        )
        .unwrap();
        assert_eq!(
            host.accept_renewal(
                current.generation(),
                current.nonce(),
                30,
                now + Duration::from_secs(14),
                114
            ),
            Err(LeaseError::StaleRenewal)
        );
        assert_eq!(host.check(now + Duration::from_secs(40), 140), Ok(()));
        assert_eq!(
            host.check(now + Duration::from_secs(41), 141),
            Err(LeaseError::LeaseExpired)
        );
    }

    #[test]
    fn expired_pending_or_active_registration_cannot_be_renewed() {
        let now = Instant::now();
        let mut pending = lease(now, 200);
        let request = pending.begin_renewal(now, 100).unwrap();
        assert_eq!(
            pending.begin_renewal(now + Duration::from_secs(30), 130),
            Err(LeaseError::LeaseExpired)
        );
        assert_eq!(
            pending.accept_renewal(
                request.generation(),
                request.nonce(),
                30,
                now + Duration::from_secs(31),
                131
            ),
            Err(LeaseError::Disabled)
        );

        let mut active = lease(now, 200);
        grant(&mut active, now, 100);
        let request = active
            .begin_renewal(now + Duration::from_secs(20), 120)
            .unwrap();
        assert_eq!(
            active.accept_renewal(
                Uuid::new_v4(),
                request.nonce(),
                30,
                now + Duration::from_secs(30),
                130
            ),
            Err(LeaseError::LeaseExpired)
        );
        assert_eq!(
            active.accept_renewal(
                request.generation(),
                request.nonce(),
                30,
                now + Duration::from_secs(31),
                131
            ),
            Err(LeaseError::Disabled)
        );
    }

    #[test]
    fn credential_expiry_caps_grants_and_invalid_registration_is_rejected() {
        let now = Instant::now();
        assert!(matches!(
            RegistrationLease::new(Uuid::nil(), 200, now, 100),
            Err(LeaseError::InvalidRegistration)
        ));
        assert!(matches!(
            RegistrationLease::new(Uuid::new_v4(), 100, now, 100),
            Err(LeaseError::CredentialExpired)
        ));
        let mut host = lease(now, 115);
        let request = host.begin_renewal(now, 100).unwrap();
        host.accept_renewal(
            request.generation(),
            request.nonce(),
            30,
            now + Duration::from_secs(10),
            110,
        )
        .unwrap();
        assert_eq!(host.check(now + Duration::from_secs(14), 114), Ok(()));
        // Monotonic expiry also caps the grant when wall time advances slowly.
        assert_eq!(
            host.check(now + Duration::from_secs(15), 114),
            Err(LeaseError::LeaseExpired)
        );
        let mut expired = lease(now, 115);
        assert_eq!(
            expired.begin_renewal(now + Duration::from_secs(10), 115),
            Err(LeaseError::CredentialExpired)
        );
    }

    #[test]
    fn invalid_or_already_elapsed_grant_is_terminal() {
        let now = Instant::now();
        for ttl in [0, 31] {
            let mut host = lease(now, 200);
            grant(&mut host, now, 100);
            let request = host
                .begin_renewal(now + Duration::from_secs(10), 110)
                .unwrap();
            assert_eq!(
                host.accept_renewal(
                    request.generation(),
                    request.nonce(),
                    ttl,
                    now + Duration::from_secs(11),
                    111
                ),
                Err(LeaseError::InvalidTtl)
            );
            assert_eq!(
                host.accept_renewal(
                    request.generation(),
                    request.nonce(),
                    30,
                    now + Duration::from_secs(12),
                    112
                ),
                Err(LeaseError::Disabled)
            );
        }
        let mut host = lease(now, 200);
        let request = host.begin_renewal(now, 100).unwrap();
        assert_eq!(
            host.accept_renewal(
                request.generation(),
                request.nonce(),
                5,
                now + Duration::from_secs(6),
                106
            ),
            Err(LeaseError::LeaseExpired)
        );
        assert_eq!(
            host.begin_renewal(now + Duration::from_secs(7), 107),
            Err(LeaseError::Disabled)
        );
    }

    #[test]
    fn clock_rollback_and_explicit_invalidation_never_resurrect_registration() {
        let now = Instant::now();
        let mut wall = lease(now, 200);
        grant(&mut wall, now, 100);
        wall.check(now + Duration::from_secs(2), 102).unwrap();
        assert_eq!(
            wall.begin_renewal(now + Duration::from_secs(3), 101),
            Err(LeaseError::ClockRollback)
        );
        assert_eq!(
            wall.check(now + Duration::from_secs(4), 104),
            Err(LeaseError::Disabled)
        );
        let mut monotonic = lease(now, 200);
        grant(&mut monotonic, now, 100);
        monotonic.check(now + Duration::from_secs(2), 102).unwrap();
        assert_eq!(
            monotonic.check(now + Duration::from_secs(1), 103),
            Err(LeaseError::ClockRollback)
        );
        let mut disabled = lease(now, 200);
        let request = disabled.begin_renewal(now, 100).unwrap();
        disabled.invalidate();
        assert_eq!(
            disabled.accept_renewal(request.generation(), request.nonce(), 30, now, 100),
            Err(LeaseError::Disabled)
        );
    }
}
