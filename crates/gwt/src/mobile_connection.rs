//! Transport-independent mobile device authorization (Issue #6476).
//!
//! This module does not open a listener or enable remote access. Its owner must
//! keep approval/revocation on the trusted PC control plane, persist device
//! snapshots securely, and use an authenticated encrypted transport. Browser
//! keys use WebCrypto ECDSA P-256/SHA-256 with raw SEC1 public keys and r||s
//! signatures; the Host never receives a device's private key.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, Read},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, RwLock,
    },
};

use gwt_core::repo_hash::ProjectKey;
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const PAIRING_TTL_SECS: u64 = 300;
const CHALLENGE_TTL_SECS: u64 = 60;
const SESSION_TTL_SECS: u64 = 300;
const DEVICE_TTL_SECS: u64 = 90 * 24 * 60 * 60;
const MAX_PENDING_PAIRINGS: usize = 8;
const MAX_CHALLENGES: usize = 64;
const MAX_CHALLENGES_PER_DEVICE: usize = 4;
const MAX_DEVICES: usize = 64;
const MAX_SNAPSHOT_BYTES: u64 = 128 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDevices {
    schema_version: u32,
    devices: Vec<DeviceRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthorizationError {
    #[error("mobile access is disabled")]
    Disabled,
    #[error("unknown pairing request or authentication challenge")]
    UnknownRequest,
    #[error("unknown paired device")]
    UnknownDevice,
    #[error("invalid device record")]
    InvalidDevice,
    #[error("public key is already paired")]
    DuplicateKey,
    #[error("mobile authorization capacity reached")]
    Capacity,
    #[error("credential expired or clock moved before issuance")]
    Expired,
    #[error("device revoked")]
    Revoked,
    #[error("device is not authorized for this project")]
    WrongProject,
    #[error("invalid device signature")]
    InvalidSignature,
    #[error("session belongs to a previous Host epoch")]
    StaleSession,
    #[error("command sequence is repeated or out of order")]
    Replay,
    #[error("mobile authorization synchronization is unavailable")]
    Unavailable,
}

/// Authorization rejection does not write; a storage error disables access.
#[derive(Debug, thiserror::Error)]
pub enum DeviceUpdateError {
    #[error(transparent)]
    Authorization(#[from] AuthorizationError),
    #[error("could not persist mobile device authorization: {0}")]
    Storage(#[source] io::Error),
}

/// Durable public metadata only. Revocation tombstones survive snapshot restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRecord {
    id: String,
    name: String,
    public_key: Vec<u8>,
    project: String,
    issued_at: u64,
    expires_at: u64,
    revoked: bool,
}

impl DeviceRecord {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn project(&self) -> &str {
        &self.project
    }
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
    pub fn is_revoked(&self) -> bool {
        self.revoked
    }

    fn check(&self, project: &ProjectKey, now: u64) -> Result<(), AuthorizationError> {
        if self.revoked {
            return Err(AuthorizationError::Revoked);
        }
        check_time(self.issued_at, self.expires_at, now)?;
        if self.project != project.as_str() {
            return Err(AuthorizationError::WrongProject);
        }
        Ok(())
    }
}

/// A request identifier is not a device credential or permission to connect.
#[derive(Debug, Clone)]
pub struct PendingPairing {
    pub id: String,
    pub name: String,
    pub project: ProjectKey,
    pub expires_at: u64,
}

struct Pairing {
    request: PendingPairing,
    public_key: Vec<u8>,
    issued_at: u64,
}

/// Sign these exact bytes, without reserializing them on the client.
#[derive(Debug)]
pub struct AuthenticationChallenge {
    pub id: String,
    pub signing_payload: Vec<u8>,
}

struct Challenge {
    public: AuthenticationChallenge,
    device_id: String,
    project: ProjectKey,
    issued_at: u64,
    expires_at: u64,
}

/// Host-only, non-cloneable session. Never deserialize client-supplied sessions.
#[derive(Debug)]
pub struct AuthenticatedSession {
    host_epoch: Uuid,
    device_id: String,
    project: ProjectKey,
    issued_at: u64,
    expires_at: u64,
    next_sequence: u64,
}

/// Host-only connection shared as `Arc<MobileConnection>` across queued work.
/// Every reference uses the same session sequence and Host revocation state.
/// This does not register a client, open a transport, or enable mobile access.
pub struct MobileConnection {
    authorization: Arc<RwLock<MobileAuthorization>>,
    session: Mutex<AuthenticatedSession>,
    next_ticket: AtomicU64,
    current_turn: Mutex<u64>,
    turns_changed: Condvar,
    order_failed: AtomicBool,
}

impl MobileConnection {
    /// The trusted Host adapter supplies its single shared authority and a
    /// session authenticated on this connection. Never accept client sessions.
    /// PC control-plane mutations take this same authority's write lock.
    pub fn new(
        authorization: Arc<RwLock<MobileAuthorization>>,
        session: AuthenticatedSession,
    ) -> Self {
        Self {
            authorization,
            session: Mutex::new(session),
            next_ticket: AtomicU64::new(0),
            current_turn: Mutex::new(0),
            turns_changed: Condvar::new(),
            order_failed: AtomicBool::new(false),
        }
    }

    /// Call at the synchronous effect, never just to enqueue it. Read trusted
    /// Host time after both locks, including time spent waiting for them.
    /// Neither callback may reenter this connection or mutate its authority;
    /// never call dispatch while holding that authority's write lock.
    /// The adapter must also recheck the live client and current Project binding.
    pub fn dispatch<T>(
        &self,
        project: &ProjectKey,
        sequence: u64,
        now: impl FnOnce() -> u64,
        effect: impl FnOnce() -> T,
    ) -> Result<T, AuthorizationError> {
        // Waiters on this session must not retain a Host read lock and delay
        // revocation. Poisoned state is never recovered as valid authority.
        let mut session = self
            .session
            .lock()
            .map_err(|_| AuthorizationError::Unavailable)?;
        let authorization = self
            .authorization
            .read()
            .map_err(|_| AuthorizationError::Unavailable)?;
        authorization.dispatch(&mut session, project, sequence, now(), effect)
    }
}

/// Internal scheduling order only, never accepted sequence or authorization.
#[doc(hidden)]
pub struct MobileInputTurn {
    connection: Arc<MobileConnection>,
    ticket: u64,
    finished: bool,
}

impl MobileInputTurn {
    pub fn wait(&self) -> Result<(), AuthorizationError> {
        self.wait_with_observer(|| {})
    }

    #[doc(hidden)]
    pub fn wait_with_observer(&self, on_wait: impl FnOnce()) -> Result<(), AuthorizationError> {
        let mut turn = self
            .connection
            .current_turn
            .lock()
            .map_err(|_| AuthorizationError::Unavailable)?;
        let mut on_wait = Some(on_wait);
        loop {
            if self.connection.order_failed.load(Ordering::Acquire) {
                return Err(AuthorizationError::Unavailable);
            }
            if *turn == self.ticket {
                return Ok(());
            }
            if let Some(on_wait) = on_wait.take() {
                on_wait();
            }
            turn = self
                .connection
                .turns_changed
                .wait(turn)
                .map_err(|_| AuthorizationError::Unavailable)?;
        }
    }

    pub fn finish(mut self) -> Result<(), AuthorizationError> {
        let mut turn = self
            .connection
            .current_turn
            .lock()
            .map_err(|_| AuthorizationError::Unavailable)?;
        if *turn != self.ticket {
            return Err(AuthorizationError::Unavailable);
        }
        *turn = turn.checked_add(1).ok_or(AuthorizationError::Unavailable)?;
        self.finished = true;
        self.connection.turns_changed.notify_all();
        Ok(())
    }
}

impl Drop for MobileInputTurn {
    fn drop(&mut self) {
        if !self.finished {
            // A dropped/failed worker must not strand a ticket gap or allow an
            // out-of-order operation. A fresh authenticated connection is needed.
            self.connection.order_failed.store(true, Ordering::Release);
            self.connection.turns_changed.notify_all();
        }
    }
}

/// Host-only terminal input; never deserialize a connection from wire data.
/// Share this immutable request with `Arc` so queueing cannot replace its
/// payload or fork the session sequence. It deliberately cannot carry generic
/// Frontend events or the asynchronously submitted PaneSendInput command.
pub struct MobileInputRequest {
    connection: Arc<MobileConnection>,
    client_id: String,
    project: ProjectKey,
    sequence: u64,
    window_id: String,
    data: String,
}

impl std::fmt::Debug for MobileInputRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MobileInputRequest").finish_non_exhaustive()
    }
}

impl MobileInputRequest {
    /// The trusted encrypted transport supplies its authenticated connection
    /// and unique live client registration, with the command's wire sequence.
    pub fn new(
        connection: Arc<MobileConnection>,
        client_id: String,
        project: ProjectKey,
        sequence: u64,
        window_id: String,
        data: String,
    ) -> Self {
        Self {
            connection,
            client_id,
            project,
            sequence,
            window_id,
            data,
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }
    pub fn project(&self) -> &ProjectKey {
        &self.project
    }

    #[doc(hidden)]
    pub fn book_turn(&self) -> Result<MobileInputTurn, AuthorizationError> {
        if self.connection.order_failed.load(Ordering::Acquire) {
            return Err(AuthorizationError::Unavailable);
        }
        let ticket = loop {
            let current = self.connection.next_ticket.load(Ordering::Acquire);
            let next = current
                .checked_add(1)
                .ok_or(AuthorizationError::Unavailable)?;
            if self
                .connection
                .next_ticket
                .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break current;
            }
        };
        Ok(MobileInputTurn {
            connection: self.connection.clone(),
            ticket,
            finished: false,
        })
    }

    #[doc(hidden)]
    pub fn window_id(&self) -> &str {
        &self.window_id
    }

    #[doc(hidden)]
    pub fn data(&self) -> &str {
        &self.data
    }

    /// Accept an operation at final synchronous dispatch, using current time
    /// after the authorization locks. Retain those locks until the effect ends.
    /// Recheck the live client here, and the current window owner in `effect`.
    /// Neither callback may reenter this connection or its Host writer.
    /// A rejected live binding also consumes this attempted command sequence.
    pub fn dispatch<T>(
        &self,
        current_project: impl FnOnce() -> Option<ProjectKey>,
        now: impl FnOnce() -> u64,
        effect: impl FnOnce(&str, &str) -> Result<T, AuthorizationError>,
    ) -> Result<T, AuthorizationError> {
        self.connection
            .dispatch(&self.project, self.sequence, now, || {
                if current_project().as_ref() != Some(&self.project) {
                    return Err(AuthorizationError::WrongProject);
                }
                effect(&self.window_id, &self.data)
            })?
    }
}

/// A single authority per Host, shared behind its owner's synchronization fence.
/// Cloning it would fork replay/revocation state, so it deliberately has no Clone.
pub struct MobileAuthorization {
    enabled: bool,
    host_epoch: Uuid,
    devices: HashMap<String, DeviceRecord>,
    pending: HashMap<String, Pairing>,
    challenges: HashMap<String, Challenge>,
}

impl Default for MobileAuthorization {
    fn default() -> Self {
        Self {
            enabled: false,
            host_epoch: Uuid::new_v4(),
            devices: HashMap::new(),
            pending: HashMap::new(),
            challenges: HashMap::new(),
        }
    }
}

impl MobileAuthorization {
    /// Only the trusted PC control plane may enable access or approve devices.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
        self.host_epoch = Uuid::new_v4();
        self.pending.clear();
        self.challenges.clear();
    }

    pub fn devices(&self) -> Vec<&DeviceRecord> {
        let mut devices: Vec<_> = self.devices.values().collect();
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        devices
    }

    pub fn request_pairing(
        &mut self,
        name: &str,
        public_key: &[u8],
        project: ProjectKey,
        now: u64,
    ) -> Result<PendingPairing, AuthorizationError> {
        self.check_enabled()?;
        validate_identity(name, public_key)?;
        if self.devices.values().any(|d| d.public_key == public_key) {
            return Err(AuthorizationError::DuplicateKey);
        }
        self.pending.retain(|_, p| now < p.request.expires_at);
        if self.pending.len() >= MAX_PENDING_PAIRINGS {
            return Err(AuthorizationError::Capacity);
        }
        let request = PendingPairing {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            project,
            expires_at: deadline(now, PAIRING_TTL_SECS)?,
        };
        self.pending.insert(
            request.id.clone(),
            Pairing {
                request: request.clone(),
                public_key: public_key.to_vec(),
                issued_at: now,
            },
        );
        Ok(request)
    }

    /// Call only after the PC user explicitly approves this exact request/key.
    pub fn approve(
        &mut self,
        request_id: &str,
        now: u64,
    ) -> Result<DeviceRecord, AuthorizationError> {
        self.check_enabled()?;
        let pending = self
            .pending
            .remove(request_id)
            .ok_or(AuthorizationError::UnknownRequest)?;
        check_time(pending.issued_at, pending.request.expires_at, now)?;
        if self.devices.len() >= MAX_DEVICES {
            return Err(AuthorizationError::Capacity);
        }
        if self
            .devices
            .values()
            .any(|d| d.public_key == pending.public_key)
        {
            return Err(AuthorizationError::DuplicateKey);
        }
        let device = DeviceRecord {
            id: Uuid::new_v4().to_string(),
            name: pending.request.name,
            public_key: pending.public_key,
            project: pending.request.project.to_string(),
            issued_at: now,
            expires_at: deadline(now, DEVICE_TTL_SECS)?,
            revoked: false,
        };
        self.devices.insert(device.id.clone(), device.clone());
        Ok(device)
    }

    /// The trusted owner must durably save the snapshot before acknowledging revocation.
    pub fn revoke(&mut self, device_id: &str) -> Result<(), AuthorizationError> {
        self.devices
            .get_mut(device_id)
            .ok_or(AuthorizationError::UnknownDevice)?
            .revoked = true;
        self.challenges
            .retain(|_, challenge| challenge.device_id != device_id);
        Ok(())
    }

    /// Approve on the trusted PC control plane, returning success only after save.
    /// The owner holds its single-writer fence through this entire call.
    /// See [`Self::revoke_and_save`] for storage failure and recovery semantics.
    pub fn approve_and_save(
        &mut self,
        request_id: &str,
        now: u64,
        path: &Path,
    ) -> Result<DeviceRecord, DeviceUpdateError> {
        let device = self.approve(request_id, now)?;
        self.save_or_disable(path)?;
        Ok(device)
    }

    /// Revoke on the trusted PC control plane, returning success only after save.
    ///
    /// Keep the owner's single-writer fence through this call. A storage error
    /// disables access and invalidates all sessions/pending requests/challenges,
    /// retaining the latest in-memory records, including revocation tombstones.
    /// Replacement may already have happened: never roll back to an old snapshot.
    /// Repair storage and save this same authority successfully before the trusted
    /// owner explicitly enables it again; successful save alone does not enable it.
    /// The private/trusted path requirements of [`Self::save`] still apply.
    pub fn revoke_and_save(
        &mut self,
        device_id: &str,
        path: &Path,
    ) -> Result<(), DeviceUpdateError> {
        self.revoke(device_id)?;
        self.save_or_disable(path)
    }

    fn save_or_disable(&mut self, path: &Path) -> Result<(), DeviceUpdateError> {
        if let Err(error) = self.save(path) {
            self.disable();
            return Err(DeviceUpdateError::Storage(error));
        }
        Ok(())
    }

    pub fn challenge(
        &mut self,
        device_id: &str,
        project: &ProjectKey,
        now: u64,
    ) -> Result<AuthenticationChallenge, AuthorizationError> {
        self.check_enabled()?;
        let device = self
            .devices
            .get(device_id)
            .ok_or(AuthorizationError::UnknownDevice)?;
        device.check(project, now)?;
        let expires_at = deadline(now, CHALLENGE_TTL_SECS)?.min(device.expires_at);
        self.challenges.retain(|_, c| now < c.expires_at);
        if self.challenges.len() >= MAX_CHALLENGES
            || self
                .challenges
                .values()
                .filter(|challenge| challenge.device_id == device_id)
                .count()
                >= MAX_CHALLENGES_PER_DEVICE
        {
            return Err(AuthorizationError::Capacity);
        }
        let id = Uuid::new_v4().to_string();
        // JSON tuple gives unambiguous UTF-8 encoding and explicit protocol/Host binding.
        let signing_payload = serde_json::to_vec(&(
            "gwt-mobile-auth-v1",
            self.host_epoch.to_string(),
            device_id,
            project.as_str(),
            &id,
            now,
            expires_at,
        ))
        .expect("string/integer tuple is serializable");
        self.challenges.insert(
            id.clone(),
            Challenge {
                public: AuthenticationChallenge {
                    id: id.clone(),
                    signing_payload: signing_payload.clone(),
                },
                device_id: device_id.to_string(),
                project: project.clone(),
                issued_at: now,
                expires_at,
            },
        );
        Ok(AuthenticationChallenge {
            id,
            signing_payload,
        })
    }

    /// A challenge is consumed even on failure. Transport adapters must keep its
    /// issue/response bound to the same authenticated encrypted connection.
    pub fn authenticate(
        &mut self,
        challenge_id: &str,
        signature: &[u8],
        now: u64,
    ) -> Result<AuthenticatedSession, AuthorizationError> {
        self.check_enabled()?;
        let challenge = self
            .challenges
            .remove(challenge_id)
            .ok_or(AuthorizationError::UnknownRequest)?;
        check_time(challenge.issued_at, challenge.expires_at, now)?;
        let device = self
            .devices
            .get(&challenge.device_id)
            .ok_or(AuthorizationError::UnknownDevice)?;
        device.check(&challenge.project, now)?;
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &device.public_key)
            .verify(&challenge.public.signing_payload, signature)
            .map_err(|_| AuthorizationError::InvalidSignature)?;
        Ok(AuthenticatedSession {
            host_epoch: self.host_epoch,
            device_id: device.id.clone(),
            project: challenge.project,
            issued_at: now,
            expires_at: deadline(now, SESSION_TTL_SECS)?.min(device.expires_at),
            next_sequence: 0,
        })
    }

    /// Recheck at the actual effect, including commands previously queued.
    ///
    /// Hold the owner's read lock throughout this call; revocation takes its
    /// write lock. Complete the effect synchronously inside `effect`: returning
    /// a future or enqueueing more work does not authorize that later effect.
    /// The transport must not clone/fan out sessions or reuse one across sockets.
    pub fn dispatch<T>(
        &self,
        session: &mut AuthenticatedSession,
        project: &ProjectKey,
        sequence: u64,
        now: u64,
        effect: impl FnOnce() -> T,
    ) -> Result<T, AuthorizationError> {
        self.check_enabled()?;
        if session.host_epoch != self.host_epoch {
            return Err(AuthorizationError::StaleSession);
        }
        let device = self
            .devices
            .get(&session.device_id)
            .ok_or(AuthorizationError::UnknownDevice)?;
        device.check(project, now)?;
        if &session.project != project {
            return Err(AuthorizationError::WrongProject);
        }
        check_time(session.issued_at, session.expires_at, now)?;
        if sequence != session.next_sequence {
            return Err(AuthorizationError::Replay);
        }
        session.next_sequence = sequence.checked_add(1).ok_or(AuthorizationError::Replay)?;
        Ok(effect())
    }

    pub fn snapshot(&self) -> Vec<DeviceRecord> {
        self.devices().into_iter().cloned().collect()
    }

    /// Load public device records, disabled and with a fresh Host epoch.
    /// Only a missing file starts empty; corrupt or unreadable state fails closed.
    /// The owner supplies a path in its private, trusted directory.
    pub fn load(path: &Path) -> io::Result<Self> {
        gwt_config::home::assert_test_path(path);
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(MAX_SNAPSHOT_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "mobile device snapshot exceeds size limit",
            ));
        }
        let stored: StoredDevices = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if stored.schema_version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported mobile device snapshot schema",
            ));
        }
        Self::restore(stored.devices)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Atomically replace the public device snapshot, including revoked records.
    ///
    /// The owner must hold its single-writer fence through mutation and save,
    /// and must not acknowledge approval/revocation until this succeeds. On
    /// error, keep access disabled rather than restoring older credentials.
    /// This method does not change the in-memory authority or create directories.
    /// The parent must be private and trusted (Windows inherits its ACL).
    /// The file is synced before replacement; Unix also syncs the directory.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        gwt_config::home::assert_test_path(path);
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        // NamedTempFile creates with mode 0600 on Unix, before writing any data.
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(
            &mut temp,
            &StoredDevices {
                schema_version: 1,
                devices: self.snapshot(),
            },
        )
        .map_err(io::Error::other)?;
        temp.as_file().sync_all()?;
        temp.persist(path).map_err(|error| error.error)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    }

    /// Restores public credentials, never pending approvals or active sessions.
    /// Access remains disabled until the trusted PC owner explicitly enables it.
    pub fn restore(devices: Vec<DeviceRecord>) -> Result<Self, AuthorizationError> {
        if devices.len() > MAX_DEVICES {
            return Err(AuthorizationError::Capacity);
        }
        let mut authority = Self::default();
        let mut keys = HashSet::new();
        for device in devices {
            validate_identity(&device.name, &device.public_key)?;
            ProjectKey::parse(&device.project).map_err(|_| AuthorizationError::InvalidDevice)?;
            if Uuid::parse_str(&device.id)
                .map(|id| id.to_string() != device.id)
                .unwrap_or(true)
                || device.expires_at != deadline(device.issued_at, DEVICE_TTL_SECS)?
                || authority.devices.contains_key(&device.id)
            {
                return Err(AuthorizationError::InvalidDevice);
            }
            if !keys.insert(device.public_key.clone()) {
                return Err(AuthorizationError::DuplicateKey);
            }
            authority.devices.insert(device.id.clone(), device);
        }
        Ok(authority)
    }

    fn check_enabled(&self) -> Result<(), AuthorizationError> {
        if self.enabled {
            Ok(())
        } else {
            Err(AuthorizationError::Disabled)
        }
    }
}

fn validate_identity(name: &str, public_key: &[u8]) -> Result<(), AuthorizationError> {
    // ring verifies curve membership together with the signature. Malformed or
    // off-curve points can never authenticate, even if a PC approved their shape.
    if name.trim().is_empty()
        || name.chars().count() > 64
        || name.chars().any(char::is_control)
        || public_key.len() != 65
        || public_key[0] != 4
    {
        return Err(AuthorizationError::InvalidDevice);
    }
    Ok(())
}

fn deadline(now: u64, ttl: u64) -> Result<u64, AuthorizationError> {
    now.checked_add(ttl).ok_or(AuthorizationError::Expired)
}

fn check_time(issued_at: u64, expires_at: u64, now: u64) -> Result<(), AuthorizationError> {
    if now < issued_at || now >= expires_at {
        return Err(AuthorizationError::Expired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::{
        rand::SystemRandom,
        signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
    };

    const NOW: u64 = 1_000_000;

    fn project() -> ProjectKey {
        ProjectKey::parse("0123456789abcdef").unwrap()
    }
    fn other_project() -> ProjectKey {
        ProjectKey::parse("fedcba9876543210").unwrap()
    }
    fn key() -> EcdsaKeyPair {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap()
    }
    fn paired(auth: &mut MobileAuthorization, key: &EcdsaKeyPair) -> DeviceRecord {
        let pending = auth
            .request_pairing("Phone", key.public_key().as_ref(), project(), NOW)
            .unwrap();
        auth.approve(&pending.id, NOW).unwrap()
    }
    fn session(
        auth: &mut MobileAuthorization,
        device: &DeviceRecord,
        key: &EcdsaKeyPair,
    ) -> AuthenticatedSession {
        let challenge = auth.challenge(&device.id, &project(), NOW).unwrap();
        let signature = key
            .sign(&SystemRandom::new(), &challenge.signing_payload)
            .unwrap();
        auth.authenticate(&challenge.id, signature.as_ref(), NOW)
            .unwrap()
    }

    #[test]
    fn mobile_input_transport_preserves_payload_and_authorizes_only_at_dispatch() {
        use crate::project_transport::{ClientHub, TransportEvent, TransportState};
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let session = session(&mut auth, &device, &key);
        let auth = Arc::new(RwLock::new(auth));
        let connection = Arc::new(MobileConnection::new(auth.clone(), session));
        let request = Arc::new(MobileInputRequest::new(
            connection.clone(),
            "phone".into(),
            project(),
            0,
            "pane".into(),
            "secret input".into(),
        ));
        assert!(!format!("{request:?}").contains("secret input"));
        let (tx, rx) = std::sync::mpsc::channel();
        let transport = TransportState::new(
            ClientHub::default(),
            Default::default(),
            "host".into(),
            Arc::new(move |event| tx.send(event).unwrap()),
        );
        transport.send_mobile_input(request.clone());
        let TransportEvent::MobileInput(queued) = rx.recv().unwrap() else {
            panic!("mobile event")
        };
        assert!(Arc::ptr_eq(&queued, &request));
        assert_eq!(queued.client_id(), "phone");
        assert_eq!(queued.project(), &project());
        queued
            .dispatch(
                || Some(project()),
                || NOW,
                |id, data| {
                    assert!(
                        auth.try_write().is_err(),
                        "retain revocation fence through the actual effect"
                    );
                    assert_eq!((id, data), ("pane", "secret input"));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(
            request.dispatch(|| Some(project()), || NOW, |_, _| panic!("replay")),
            Err::<(), _>(AuthorizationError::Replay)
        );
        let next = MobileInputRequest::new(
            connection,
            "phone".into(),
            project(),
            1,
            "pane".into(),
            "submit".into(),
        );
        auth.write().unwrap().revoke(&device.id).unwrap();
        assert_eq!(
            next.dispatch(|| Some(project()), || NOW, |_, _| panic!("revoked")),
            Err::<(), _>(AuthorizationError::Revoked)
        );
    }

    #[test]
    fn mobile_input_rechecks_live_scope_and_time_after_queueing() {
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let session = session(&mut auth, &device, &key);
        let auth = Arc::new(RwLock::new(auth));
        let connection = Arc::new(MobileConnection::new(auth, session));
        for (sequence, scope) in [None, Some(other_project())].into_iter().enumerate() {
            let request = MobileInputRequest::new(
                connection.clone(),
                "phone".into(),
                project(),
                sequence as u64,
                "pane".into(),
                "input".into(),
            );
            assert_eq!(
                request.dispatch(|| scope, || NOW, |_, _| panic!("unbound client")),
                Err::<(), _>(AuthorizationError::WrongProject)
            );
        }
        let request = MobileInputRequest::new(
            connection,
            "phone".into(),
            project(),
            2,
            "pane".into(),
            "input".into(),
        );
        assert_eq!(
            request.dispatch(
                || Some(project()),
                || NOW + SESSION_TTL_SECS,
                |_, _| panic!("expired")
            ),
            Err::<(), _>(AuthorizationError::Expired)
        );
    }

    #[test]
    fn connection_shares_sequence_and_holds_locks_through_clock_and_effect() {
        use std::sync::{Arc, RwLock};
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let session = session(&mut auth, &device, &key);
        let auth = Arc::new(RwLock::new(auth));
        let connection = Arc::new(MobileConnection::new(auth.clone(), session));
        let queued = connection.clone();
        let check_locks = || {
            assert!(auth.try_write().is_err());
            assert!(connection.session.try_lock().is_err());
        };
        assert_eq!(
            queued.dispatch(
                &project(),
                0,
                || {
                    check_locks();
                    NOW
                },
                || {
                    check_locks();
                    "written"
                }
            ),
            Ok("written")
        );
        assert_eq!(
            connection.dispatch(&project(), 0, || NOW, || panic!("replayed effect")),
            Err(AuthorizationError::Replay)
        );
        assert!(auth.try_write().is_ok());
    }

    #[test]
    fn connection_rechecks_authority_when_queued_effect_runs() {
        use std::{
            cell::Cell,
            sync::{Arc, RwLock},
        };
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let session = session(&mut auth, &device, &key);
        let auth = Arc::new(RwLock::new(auth));
        let connection = MobileConnection::new(auth.clone(), session);
        let now = Cell::new(NOW);
        let queued =
            || connection.dispatch(&project(), 0, || now.get(), || panic!("expired effect"));
        now.set(NOW + SESSION_TTL_SECS);
        assert_eq!(queued(), Err(AuthorizationError::Expired));
        assert_eq!(
            connection.dispatch(&other_project(), 0, || NOW, || panic!("wrong project")),
            Err(AuthorizationError::WrongProject)
        );
        let queued = || connection.dispatch(&project(), 0, || NOW, || panic!("revoked effect"));
        auth.write().unwrap().revoke(device.id()).unwrap();
        assert_eq!(queued(), Err(AuthorizationError::Revoked));
    }

    #[test]
    fn connection_rejects_poisoned_session_or_host_without_running_effect() {
        use std::sync::{Arc, RwLock};
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let first = session(&mut auth, &device, &key);
        let second = session(&mut auth, &device, &key);
        let auth = Arc::new(RwLock::new(auth));
        let connection = MobileConnection::new(auth.clone(), first);
        assert!(std::panic::catch_unwind(|| {
            let _guard = connection.session.lock().unwrap();
            panic!("poison session");
        })
        .is_err());
        assert_eq!(
            connection.dispatch(&project(), 0, || panic!("clock"), || ()),
            Err(AuthorizationError::Unavailable)
        );
        let connection = MobileConnection::new(auth.clone(), second);
        assert!(std::panic::catch_unwind(|| {
            let _guard = auth.write().unwrap();
            panic!("poison host");
        })
        .is_err());
        assert_eq!(
            connection.dispatch(&project(), 0, || panic!("clock"), || ()),
            Err(AuthorizationError::Unavailable)
        );
    }

    #[test]
    fn pairing_requires_enable_and_explicit_single_use_pc_approval() {
        let mut auth = MobileAuthorization::default();
        let key = key();
        assert_eq!(
            auth.request_pairing("Phone", key.public_key().as_ref(), project(), NOW)
                .unwrap_err(),
            AuthorizationError::Disabled
        );
        auth.enable();
        let pending = auth
            .request_pairing("Phone", key.public_key().as_ref(), project(), NOW)
            .unwrap();
        assert!(auth.devices().is_empty());
        assert_eq!(
            auth.challenge(&pending.id, &project(), NOW).unwrap_err(),
            AuthorizationError::UnknownDevice
        );
        let device = auth.approve(&pending.id, NOW).unwrap();
        assert_eq!(device.project, project().as_str());
        assert_eq!(
            auth.approve(&pending.id, NOW).unwrap_err(),
            AuthorizationError::UnknownRequest
        );
        let second = paired(&mut auth, &self::key());
        assert_ne!(device.id, second.id);
        assert_eq!(device.name, second.name);
        assert_eq!(
            auth.request_pairing("Phone", key.public_key().as_ref(), project(), NOW)
                .unwrap_err(),
            AuthorizationError::DuplicateKey
        );
    }

    #[test]
    fn pending_requests_expire_and_are_bounded_without_wall_clock_waits() {
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        for _ in 0..MAX_PENDING_PAIRINGS {
            auth.request_pairing("Phone", key.public_key().as_ref(), project(), NOW)
                .unwrap();
        }
        assert_eq!(
            auth.request_pairing("Phone", key.public_key().as_ref(), project(), NOW)
                .unwrap_err(),
            AuthorizationError::Capacity
        );
        let pending = auth
            .request_pairing(
                "Phone",
                key.public_key().as_ref(),
                project(),
                NOW + PAIRING_TTL_SECS,
            )
            .unwrap();
        assert_eq!(
            auth.approve(&pending.id, NOW + 2 * PAIRING_TTL_SECS)
                .unwrap_err(),
            AuthorizationError::Expired
        );
    }

    #[test]
    fn webcrypto_fixed_signature_is_project_bound_and_not_replayable() {
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        assert_eq!(
            auth.challenge(&device.id, &other_project(), NOW)
                .unwrap_err(),
            AuthorizationError::WrongProject
        );
        let challenge = auth.challenge(&device.id, &project(), NOW).unwrap();
        let signature = key
            .sign(&SystemRandom::new(), &challenge.signing_payload)
            .unwrap();
        let mut session = auth
            .authenticate(&challenge.id, signature.as_ref(), NOW)
            .unwrap();
        assert_eq!(
            auth.authenticate(&challenge.id, signature.as_ref(), NOW)
                .unwrap_err(),
            AuthorizationError::UnknownRequest
        );
        let mut dispatched = 0;
        auth.dispatch(&mut session, &project(), 0, NOW, || dispatched += 1)
            .unwrap();
        assert_eq!(
            auth.dispatch(&mut session, &project(), 0, NOW, || dispatched += 1),
            Err(AuthorizationError::Replay)
        );
        assert_eq!(
            auth.dispatch(&mut session, &other_project(), 1, NOW, || dispatched += 1),
            Err(AuthorizationError::WrongProject)
        );
        auth.dispatch(&mut session, &project(), 1, NOW, || dispatched += 1)
            .unwrap();
        assert_eq!(dispatched, 2);
    }

    #[test]
    fn bad_signatures_and_expired_challenges_never_authenticate() {
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let challenge = auth.challenge(&device.id, &project(), NOW).unwrap();
        assert_eq!(
            auth.authenticate(&challenge.id, &[0; 64], NOW).unwrap_err(),
            AuthorizationError::InvalidSignature
        );
        assert_eq!(
            auth.authenticate(&challenge.id, &[0; 64], NOW).unwrap_err(),
            AuthorizationError::UnknownRequest
        );
        let challenge = auth.challenge(&device.id, &project(), NOW).unwrap();
        let signature = key
            .sign(&SystemRandom::new(), &challenge.signing_payload)
            .unwrap();
        assert_eq!(
            auth.authenticate(&challenge.id, signature.as_ref(), NOW + CHALLENGE_TTL_SECS)
                .unwrap_err(),
            AuthorizationError::Expired
        );
    }

    #[test]
    fn challenge_capacity_is_per_device_and_recovers_after_consumption_or_expiry() {
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let mut challenges: Vec<_> = (0..4)
            .map(|_| auth.challenge(&device.id, &project(), NOW).unwrap())
            .collect();
        assert_eq!(
            auth.challenge(&device.id, &project(), NOW).unwrap_err(),
            AuthorizationError::Capacity
        );
        let other_key = self::key();
        let other = paired(&mut auth, &other_key);
        let mut peer = session(&mut auth, &other, &other_key);
        auth.dispatch(&mut peer, &project(), 0, NOW, || ()).unwrap();

        // Refusal preserves issued challenges; either response consumes one slot.
        let challenge = challenges.pop().unwrap();
        let signature = key
            .sign(&SystemRandom::new(), &challenge.signing_payload)
            .unwrap();
        auth.authenticate(&challenge.id, signature.as_ref(), NOW)
            .unwrap();
        let replacement = auth.challenge(&device.id, &project(), NOW).unwrap();
        assert_eq!(
            auth.authenticate(&replacement.id, &[0; 64], NOW)
                .unwrap_err(),
            AuthorizationError::InvalidSignature
        );
        auth.challenge(&device.id, &project(), NOW).unwrap();
        assert_eq!(
            auth.challenge(&device.id, &project(), NOW).unwrap_err(),
            AuthorizationError::Capacity
        );
        auth.challenge(&device.id, &project(), NOW + CHALLENGE_TTL_SECS)
            .unwrap();
    }

    #[test]
    fn challenge_capacity_remains_globally_bounded_and_recovers_after_revocation() {
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let devices: Vec<_> = (0..16).map(|_| paired(&mut auth, &key())).collect();
        for device in &devices {
            for _ in 0..4 {
                auth.challenge(&device.id, &project(), NOW).unwrap();
            }
        }
        let other = paired(&mut auth, &key());
        assert_eq!(
            auth.challenge(&other.id, &project(), NOW).unwrap_err(),
            AuthorizationError::Capacity
        );
        auth.revoke(&devices[0].id).unwrap();
        auth.challenge(&other.id, &project(), NOW).unwrap();
    }

    #[test]
    fn revocation_rejects_live_sessions_queued_operations_and_reconnection() {
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let mut session = session(&mut auth, &device, &key);
        let challenge = auth.challenge(&device.id, &project(), NOW).unwrap();
        auth.revoke(&device.id).unwrap();
        assert_eq!(
            auth.dispatch(&mut session, &project(), 0, NOW, || panic!(
                "revoked effect ran"
            )),
            Err(AuthorizationError::Revoked)
        );
        assert_eq!(
            auth.challenge(&device.id, &project(), NOW).unwrap_err(),
            AuthorizationError::Revoked
        );
        assert_eq!(
            auth.authenticate(&challenge.id, &[0; 64], NOW).unwrap_err(),
            AuthorizationError::UnknownRequest
        );
    }

    #[test]
    fn expiry_disable_and_host_restart_invalidate_sessions() {
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let mut old = session(&mut auth, &device, &key);
        assert_eq!(
            auth.dispatch(&mut old, &project(), 0, NOW + SESSION_TTL_SECS, || ()),
            Err(AuthorizationError::Expired)
        );
        let mut old = session(&mut auth, &device, &key);
        auth.disable();
        assert_eq!(
            auth.dispatch(&mut old, &project(), 0, NOW, || ()),
            Err(AuthorizationError::Disabled)
        );
        auth.enable();
        assert_eq!(
            auth.dispatch(&mut old, &project(), 0, NOW, || ()),
            Err(AuthorizationError::StaleSession)
        );
        let mut old = session(&mut auth, &device, &key);
        auth.dispatch(&mut old, &project(), 0, NOW, || ()).unwrap();
        let encoded = serde_json::to_string(&auth.snapshot()).unwrap();
        let mut restored =
            MobileAuthorization::restore(serde_json::from_str(&encoded).unwrap()).unwrap();
        assert_eq!(
            restored.challenge(&device.id, &project(), NOW).unwrap_err(),
            AuthorizationError::Disabled
        );
        restored.enable();
        assert_eq!(
            restored.dispatch(&mut old, &project(), 1, NOW, || ()),
            Err(AuthorizationError::StaleSession)
        );
        let mut fresh = session(&mut restored, &device, &key);
        restored
            .dispatch(&mut fresh, &project(), 0, NOW, || ())
            .unwrap();
        assert_eq!(
            restored
                .challenge(&device.id, &project(), NOW + DEVICE_TTL_SECS)
                .unwrap_err(),
            AuthorizationError::Expired
        );
        restored.revoke(&device.id).unwrap();
        let mut restored = MobileAuthorization::restore(restored.snapshot()).unwrap();
        restored.enable();
        assert_eq!(
            restored.challenge(&device.id, &project(), NOW).unwrap_err(),
            AuthorizationError::Revoked
        );
    }

    #[test]
    fn snapshot_restore_rejects_corrupt_and_duplicate_credentials() {
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let device = paired(&mut auth, &key());
        assert!(matches!(
            MobileAuthorization::restore(vec![device.clone(); MAX_DEVICES + 1]),
            Err(AuthorizationError::Capacity)
        ));
        assert!(matches!(
            MobileAuthorization::restore(vec![device.clone(), device.clone()]),
            Err(AuthorizationError::InvalidDevice)
        ));
        let mut duplicate_key = device.clone();
        duplicate_key.id = Uuid::new_v4().to_string();
        assert!(matches!(
            MobileAuthorization::restore(vec![device.clone(), duplicate_key]),
            Err(AuthorizationError::DuplicateKey)
        ));
        let mut invalid_project = device.clone();
        invalid_project.project = "../other-project".into();
        assert!(matches!(
            MobileAuthorization::restore(vec![invalid_project]),
            Err(AuthorizationError::InvalidDevice)
        ));
        let mut invalid_expiry = device.clone();
        invalid_expiry.expires_at += 1;
        assert!(matches!(
            MobileAuthorization::restore(vec![invalid_expiry]),
            Err(AuthorizationError::InvalidDevice)
        ));
        let mut invalid_key = device;
        invalid_key.public_key.clear();
        assert!(matches!(
            MobileAuthorization::restore(vec![invalid_key]),
            Err(AuthorizationError::InvalidDevice)
        ));
    }

    #[test]
    fn disk_roundtrip_restores_devices_disabled_without_transient_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let mut auth = MobileAuthorization::load(&path).unwrap();
        assert!(auth.devices().is_empty());
        assert_eq!(auth.check_enabled(), Err(AuthorizationError::Disabled));
        assert!(!path.exists());
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let mut old = session(&mut auth, &device, &key);
        let pending = auth
            .request_pairing("Tablet", self::key().public_key().as_ref(), project(), NOW)
            .unwrap();
        let challenge = auth.challenge(&device.id, &project(), NOW).unwrap();
        let signature = key
            .sign(&SystemRandom::new(), &challenge.signing_payload)
            .unwrap();
        auth.save(&path).unwrap();

        let mut restored = MobileAuthorization::load(&path).unwrap();
        assert_eq!(restored.check_enabled(), Err(AuthorizationError::Disabled));
        restored.enable();
        assert_eq!(
            restored.dispatch(&mut old, &project(), 0, NOW, || ()),
            Err(AuthorizationError::StaleSession)
        );
        assert_eq!(
            restored.approve(&pending.id, NOW).unwrap_err(),
            AuthorizationError::UnknownRequest
        );
        assert_eq!(
            restored
                .authenticate(&challenge.id, signature.as_ref(), NOW)
                .unwrap_err(),
            AuthorizationError::UnknownRequest
        );
        let mut fresh = session(&mut restored, &device, &key);
        restored
            .dispatch(&mut fresh, &project(), 0, NOW, || ())
            .unwrap();
    }

    #[test]
    fn disk_replacement_preserves_revocation_and_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let device = paired(&mut auth, &key());
        auth.save(&path).unwrap();
        auth.revoke(&device.id).unwrap();
        auth.save(&path).unwrap();
        let mut restored = MobileAuthorization::load(&path).unwrap();
        restored.enable();
        assert_eq!(
            restored.challenge(&device.id, &project(), NOW).unwrap_err(),
            AuthorizationError::Revoked
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn disk_load_rejects_corruption_unknown_schema_oversize_and_invalid_devices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let mut auth = MobileAuthorization::default();
        auth.enable();
        paired(&mut auth, &key());
        auth.save(&path).unwrap();
        let valid = std::fs::read_to_string(&path).unwrap();
        let mut invalid_device: serde_json::Value = serde_json::from_str(&valid).unwrap();
        invalid_device["devices"][0]["project"] = "../other-project".into();
        for invalid in [
            "{".into(),
            r#"{"schema_version":2,"devices":[]}"#.into(),
            r#"{"schema_version":1,"devices":[],"enabled":true}"#.into(),
            format!("{valid}{}", " ".repeat(128 * 1024)),
            invalid_device.to_string(),
        ] {
            std::fs::write(&path, &invalid).unwrap();
            assert!(matches!(
                MobileAuthorization::load(&path),
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData
            ));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
        }
    }

    #[test]
    fn persisted_updates_save_before_success_and_leave_authorization_rejections_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let pending = auth
            .request_pairing("Phone", key.public_key().as_ref(), project(), NOW)
            .unwrap();
        let device = auth.approve_and_save(&pending.id, NOW, &path).unwrap();
        let mut restored = MobileAuthorization::load(&path).unwrap();
        restored.enable();
        session(&mut restored, &device, &key);

        let before = std::fs::read(&path).unwrap();
        let missing = dir.path().join("missing/devices.json");
        assert!(matches!(
            auth.approve_and_save(&pending.id, NOW, &missing),
            Err(DeviceUpdateError::Authorization(
                AuthorizationError::UnknownRequest
            ))
        ));
        assert!(matches!(
            auth.revoke_and_save("unknown", &missing),
            Err(DeviceUpdateError::Authorization(
                AuthorizationError::UnknownDevice
            ))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(auth.check_enabled(), Ok(()));

        auth.revoke_and_save(&device.id, &path).unwrap();
        let mut restored = MobileAuthorization::load(&path).unwrap();
        restored.enable();
        assert_eq!(
            restored.challenge(&device.id, &project(), NOW).unwrap_err(),
            AuthorizationError::Revoked
        );
        assert_eq!(auth.check_enabled(), Ok(()));
    }

    #[test]
    fn failed_approval_save_disables_access_and_clears_transient_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/devices.json");
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let mut old = session(&mut auth, &device, &key);
        let challenge = auth.challenge(&device.id, &project(), NOW).unwrap();
        let pending = auth
            .request_pairing("Tablet", self::key().public_key().as_ref(), project(), NOW)
            .unwrap();
        let unapproved = auth
            .request_pairing("Other", self::key().public_key().as_ref(), project(), NOW)
            .unwrap();
        assert!(matches!(
            auth.approve_and_save(&pending.id, NOW, &path),
            Err(DeviceUpdateError::Storage(error)) if error.kind() == io::ErrorKind::NotFound
        ));
        assert_eq!(auth.check_enabled(), Err(AuthorizationError::Disabled));
        assert_eq!(auth.devices().len(), 2);
        assert_eq!(
            auth.dispatch(&mut old, &project(), 0, NOW, || panic!("disabled effect")),
            Err(AuthorizationError::Disabled)
        );

        std::fs::create_dir(path.parent().unwrap()).unwrap();
        auth.save(&path).unwrap();
        assert_eq!(auth.check_enabled(), Err(AuthorizationError::Disabled));
        auth.enable();
        assert_eq!(
            auth.dispatch(&mut old, &project(), 0, NOW, || panic!("stale effect")),
            Err(AuthorizationError::StaleSession)
        );
        assert_eq!(
            auth.authenticate(&challenge.id, &[], NOW).unwrap_err(),
            AuthorizationError::UnknownRequest
        );
        assert_eq!(
            auth.approve(&unapproved.id, NOW).unwrap_err(),
            AuthorizationError::UnknownRequest
        );
        assert_eq!(MobileAuthorization::load(&path).unwrap().devices().len(), 2);
        session(&mut auth, &device, &key);
    }

    #[test]
    fn failed_revocation_save_preserves_tombstone_through_repair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let mut auth = MobileAuthorization::default();
        auth.enable();
        let key = key();
        let device = paired(&mut auth, &key);
        let mut old = session(&mut auth, &device, &key);
        auth.save(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(
            auth.revoke_and_save(&device.id, &path),
            Err(DeviceUpdateError::Storage(_))
        ));
        assert_eq!(auth.check_enabled(), Err(AuthorizationError::Disabled));
        assert!(auth.devices()[0].is_revoked());
        assert_eq!(
            auth.dispatch(&mut old, &project(), 0, NOW, || panic!("disabled effect")),
            Err(AuthorizationError::Disabled)
        );
        std::fs::remove_dir(&path).unwrap();
        auth.save(&path).unwrap();
        auth.enable();
        let mut restored = MobileAuthorization::load(&path).unwrap();
        restored.enable();
        for authority in [&mut auth, &mut restored] {
            assert_eq!(
                authority
                    .challenge(&device.id, &project(), NOW)
                    .unwrap_err(),
                AuthorizationError::Revoked
            );
        }
    }

    #[test]
    fn disk_io_failures_are_returned_without_leaving_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        std::fs::create_dir(&path).unwrap();
        let auth = MobileAuthorization::default();
        assert!(MobileAuthorization::load(&path).is_err());
        assert!(auth.save(&path).is_err());
        assert!(path.is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(auth.save(&path.join("missing/devices.json")).is_err());
    }
}
