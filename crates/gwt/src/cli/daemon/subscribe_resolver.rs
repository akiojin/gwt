//! Read-only endpoint selection for Unix daemon subscriptions.
//!
//! Exact caller scope always wins. A same-repository sibling is eligible only
//! when exact evidence is absent or definitely dead, and only one compatible
//! live sibling exists.

use std::{
    collections::BTreeMap,
    fmt, fs,
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    time::Duration,
};

use gwt_core::daemon::{
    validate_handshake, DaemonEndpoint, IpcHandshakeRequest, IpcHandshakeResponse, RuntimeScope,
};
use tokio::{io::AsyncReadExt, io::AsyncWriteExt, net::UnixStream, time::timeout};

const SIBLING_HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_HANDSHAKE_RESPONSE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Missing,
    Ambiguous,
    InvalidEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeOutcome {
    Reachable,
    DefinitelyUnavailable,
    Uncertain,
}

struct SiblingScan {
    exact_path: PathBuf,
    exact_outcome: String,
    candidates: Vec<DaemonEndpoint>,
    rejected: Vec<&'static str>,
    evidence_problems: Vec<String>,
}

enum PreparedResolution {
    Exact(DaemonEndpoint),
    Siblings(SiblingScan),
}

impl FailureKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Ambiguous => "ambiguous",
            Self::InvalidEvidence => "invalid_evidence",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolutionFailure {
    kind: FailureKind,
    requested_scope: RuntimeScope,
    exact_path: PathBuf,
    exact_outcome: String,
    candidate_roots: Vec<PathBuf>,
    candidate_count: usize,
    rejected: Vec<&'static str>,
    evidence_problems: Vec<String>,
}

impl ResolutionFailure {
    fn invalid_evidence(
        requested_scope: &RuntimeScope,
        exact_path: PathBuf,
        exact_outcome: impl Into<String>,
    ) -> Box<Self> {
        Box::new(Self {
            kind: FailureKind::InvalidEvidence,
            requested_scope: requested_scope.clone(),
            exact_path,
            exact_outcome: exact_outcome.into(),
            candidate_roots: Vec::new(),
            candidate_count: 0,
            rejected: Vec::new(),
            evidence_problems: Vec::new(),
        })
    }

    fn rejected_summary(&self) -> String {
        let counts =
            self.rejected
                .iter()
                .fold(BTreeMap::<&str, usize>::new(), |mut counts, reason| {
                    *counts.entry(reason).or_default() += 1;
                    counts
                });
        if counts.is_empty() {
            return "none".to_string();
        }
        counts
            .into_iter()
            .map(|(reason, count)| format!("{reason}={count}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn recovery(&self) -> String {
        match self.kind {
            FailureKind::Missing => "run `gwtd daemon start` from the intended daemon owner worktree, or retry subscribe from a worktree with an exact endpoint".to_string(),
            FailureKind::Ambiguous => format!(
                "retry from exactly one owner worktree so its exact endpoint wins (owner worktree roots=[{}])",
                self.candidate_roots
                    .iter()
                    .map(|root| root.display().to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            FailureKind::InvalidEvidence => "inspect and repair the reported endpoint metadata before retrying; sibling fallback was not used".to_string(),
        }
    }
}

impl fmt::Display for ResolutionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "gwtd daemon subscribe: endpoint resolution failed kind={} requested_scope={}/{} exact_endpoint={} exact={} candidates={} rejected=[{}] evidence=[{}] recovery={}",
            self.kind.as_str(),
            self.requested_scope.repo_hash,
            self.requested_scope.worktree_hash,
            self.exact_path.display(),
            self.exact_outcome,
            self.candidate_count,
            self.rejected_summary(),
            if self.evidence_problems.is_empty() {
                "none".to_string()
            } else {
                self.evidence_problems.join(",")
            },
            self.recovery()
        )
    }
}

pub(super) async fn resolve<F>(
    gwt_home: &Path,
    requested_scope: &RuntimeScope,
    expected_protocol_version: u32,
    is_process_alive: F,
) -> Result<DaemonEndpoint, Box<ResolutionFailure>>
where
    F: Fn(u32) -> bool,
{
    match prepare_resolution(
        gwt_home,
        requested_scope,
        expected_protocol_version,
        is_process_alive,
    )? {
        PreparedResolution::Exact(endpoint) => Ok(endpoint),
        PreparedResolution::Siblings(scan) if !scan.evidence_problems.is_empty() => {
            finalize_scan(requested_scope, scan, Vec::new())
        }
        PreparedResolution::Siblings(scan) => {
            let mut outcomes = Vec::with_capacity(scan.candidates.len());
            for endpoint in &scan.candidates {
                outcomes.push(authenticated_probe(endpoint).await);
            }
            finalize_scan(requested_scope, scan, outcomes)
        }
    }
}

#[cfg(test)]
fn resolve_with_probe<F, P>(
    gwt_home: &Path,
    requested_scope: &RuntimeScope,
    expected_protocol_version: u32,
    is_process_alive: F,
    probe: P,
) -> Result<DaemonEndpoint, Box<ResolutionFailure>>
where
    F: Fn(u32) -> bool,
    P: Fn(&DaemonEndpoint) -> ProbeOutcome,
{
    match prepare_resolution(
        gwt_home,
        requested_scope,
        expected_protocol_version,
        is_process_alive,
    )? {
        PreparedResolution::Exact(endpoint) => Ok(endpoint),
        PreparedResolution::Siblings(scan) if !scan.evidence_problems.is_empty() => {
            finalize_scan(requested_scope, scan, Vec::new())
        }
        PreparedResolution::Siblings(scan) => {
            let outcomes = scan.candidates.iter().map(probe).collect();
            finalize_scan(requested_scope, scan, outcomes)
        }
    }
}

fn prepare_resolution<F>(
    gwt_home: &Path,
    requested_scope: &RuntimeScope,
    expected_protocol_version: u32,
    is_process_alive: F,
) -> Result<PreparedResolution, Box<ResolutionFailure>>
where
    F: Fn(u32) -> bool,
{
    let exact_path = requested_scope.endpoint_path(gwt_home);
    let exact_outcome = match fs::read(&exact_path) {
        Ok(payload) => {
            let endpoint: DaemonEndpoint = serde_json::from_slice(&payload).map_err(|_| {
                ResolutionFailure::invalid_evidence(
                    requested_scope,
                    exact_path.clone(),
                    "malformed",
                )
            })?;
            if !is_live_pid(endpoint.pid, &is_process_alive) {
                format!("dead(pid={})", endpoint.pid)
            } else {
                return validate_exact(
                    endpoint,
                    requested_scope,
                    expected_protocol_version,
                    exact_path,
                )
                .map(PreparedResolution::Exact);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "missing".to_string(),
        Err(error) => {
            return Err(ResolutionFailure::invalid_evidence(
                requested_scope,
                exact_path,
                format!("unreadable:{error}"),
            ));
        }
    };

    scan_siblings(
        gwt_home,
        requested_scope,
        expected_protocol_version,
        &is_process_alive,
        exact_path,
        exact_outcome,
    )
}

fn validate_exact(
    endpoint: DaemonEndpoint,
    requested_scope: &RuntimeScope,
    expected_protocol_version: u32,
    exact_path: PathBuf,
) -> Result<DaemonEndpoint, Box<ResolutionFailure>> {
    let problem = if endpoint.scope != *requested_scope {
        Some("scope_mismatch".to_string())
    } else if endpoint.protocol_version != expected_protocol_version {
        Some(format!(
            "protocol_mismatch(expected={expected_protocol_version},actual={})",
            endpoint.protocol_version
        ))
    } else if let Some(reason) = unix_socket_rejection(&endpoint.bind) {
        Some(reason.to_string())
    } else if endpoint.auth_token.trim().is_empty() {
        Some("missing_auth_token".to_string())
    } else {
        None
    };
    match problem {
        Some(problem) => Err(ResolutionFailure::invalid_evidence(
            requested_scope,
            exact_path,
            problem,
        )),
        None => Ok(endpoint),
    }
}

fn scan_siblings<F>(
    gwt_home: &Path,
    requested_scope: &RuntimeScope,
    expected_protocol_version: u32,
    is_process_alive: &F,
    exact_path: PathBuf,
    exact_outcome: String,
) -> Result<PreparedResolution, Box<ResolutionFailure>>
where
    F: Fn(u32) -> bool,
{
    let read_dir = match fs::read_dir(requested_scope.daemon_dir(gwt_home)) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PreparedResolution::Siblings(SiblingScan {
                exact_path,
                exact_outcome,
                candidates: Vec::new(),
                rejected: Vec::new(),
                evidence_problems: Vec::new(),
            }));
        }
        Err(_) => {
            return Err(failure(
                FailureKind::InvalidEvidence,
                requested_scope,
                exact_path,
                exact_outcome,
                Vec::new(),
                Vec::new(),
                vec!["daemon_directory_unreadable".to_string()],
            ));
        }
    };

    let mut paths = Vec::new();
    let mut evidence_problems = Vec::new();
    for entry in read_dir {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if path != exact_path
                    && path.extension().and_then(|value| value.to_str()) == Some("json")
                {
                    paths.push(path);
                }
            }
            Err(_) => evidence_problems.push("directory_entry_unreadable".to_string()),
        }
    }
    paths.sort();

    let mut candidates = Vec::new();
    let mut rejected = Vec::new();
    for path in paths {
        let payload = match fs::read(&path) {
            Ok(payload) => payload,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                rejected.push("vanished");
                continue;
            }
            Err(_) => {
                evidence_problems.push(format!("sibling_unreadable:{}", path.display()));
                continue;
            }
        };
        let endpoint: DaemonEndpoint = match serde_json::from_slice(&payload) {
            Ok(endpoint) => endpoint,
            Err(_) => {
                evidence_problems.push(format!("sibling_malformed:{}", path.display()));
                continue;
            }
        };
        match sibling_rejection(
            &path,
            &endpoint,
            requested_scope,
            expected_protocol_version,
            is_process_alive,
        ) {
            Some(reason) => rejected.push(reason),
            None => candidates.push(endpoint),
        }
    }

    Ok(PreparedResolution::Siblings(SiblingScan {
        exact_path,
        exact_outcome,
        candidates,
        rejected,
        evidence_problems,
    }))
}

fn finalize_scan(
    requested_scope: &RuntimeScope,
    mut scan: SiblingScan,
    outcomes: Vec<ProbeOutcome>,
) -> Result<DaemonEndpoint, Box<ResolutionFailure>> {
    if !scan.evidence_problems.is_empty() {
        return Err(failure(
            FailureKind::InvalidEvidence,
            requested_scope,
            scan.exact_path,
            scan.exact_outcome,
            scan.candidates,
            scan.rejected,
            scan.evidence_problems,
        ));
    }

    assert_eq!(scan.candidates.len(), outcomes.len());
    let mut reachable = Vec::new();
    let mut uncertain = Vec::new();
    for (endpoint, outcome) in scan.candidates.into_iter().zip(outcomes) {
        match outcome {
            ProbeOutcome::Reachable => reachable.push(endpoint),
            ProbeOutcome::DefinitelyUnavailable => scan.rejected.push("unreachable"),
            ProbeOutcome::Uncertain => uncertain.push(endpoint),
        }
    }
    if !uncertain.is_empty() {
        scan.evidence_problems
            .resize(uncertain.len(), "sibling_probe_uncertain".to_string());
        reachable.extend(uncertain);
        return Err(failure(
            FailureKind::InvalidEvidence,
            requested_scope,
            scan.exact_path,
            scan.exact_outcome,
            reachable,
            scan.rejected,
            scan.evidence_problems,
        ));
    }
    if reachable.len() == 1 {
        return Ok(reachable.pop().expect("one reachable candidate"));
    }
    let kind = if reachable.is_empty() {
        FailureKind::Missing
    } else {
        FailureKind::Ambiguous
    };
    Err(failure(
        kind,
        requested_scope,
        scan.exact_path,
        scan.exact_outcome,
        reachable,
        scan.rejected,
        scan.evidence_problems,
    ))
}

fn sibling_rejection<F>(
    path: &Path,
    endpoint: &DaemonEndpoint,
    requested_scope: &RuntimeScope,
    expected_protocol_version: u32,
    is_process_alive: &F,
) -> Option<&'static str>
where
    F: Fn(u32) -> bool,
{
    let expected_filename = format!("{}.json", endpoint.scope.worktree_hash);
    if path.file_name().and_then(|value| value.to_str()) != Some(&expected_filename) {
        Some("metadata_path_mismatch")
    } else if endpoint.scope.repo_hash != requested_scope.repo_hash {
        Some("repo_mismatch")
    } else if endpoint.scope.target != requested_scope.target {
        Some("target_mismatch")
    } else if !endpoint.scope.project_root.is_absolute() {
        Some("invalid_project_root")
    } else if endpoint.protocol_version != expected_protocol_version {
        Some("protocol_mismatch")
    } else if !is_live_pid(endpoint.pid, is_process_alive) {
        Some("dead")
    } else if let Some(reason) = unix_socket_rejection(&endpoint.bind) {
        Some(reason)
    } else if endpoint.auth_token.trim().is_empty() {
        Some("missing_auth_token")
    } else {
        None
    }
}

fn failure(
    kind: FailureKind,
    requested_scope: &RuntimeScope,
    exact_path: PathBuf,
    exact_outcome: String,
    candidates: Vec<DaemonEndpoint>,
    rejected: Vec<&'static str>,
    evidence_problems: Vec<String>,
) -> Box<ResolutionFailure> {
    Box::new(ResolutionFailure {
        kind,
        requested_scope: requested_scope.clone(),
        exact_path,
        exact_outcome,
        candidate_roots: candidates
            .iter()
            .map(|endpoint| endpoint.scope.project_root.clone())
            .collect(),
        candidate_count: candidates.len(),
        rejected,
        evidence_problems,
    })
}

fn unix_socket_rejection(bind: &str) -> Option<&'static str> {
    let bind = bind.trim();
    if bind.is_empty() || !Path::new(bind).is_absolute() {
        return Some("unsupported_transport");
    }
    match fs::metadata(bind) {
        Ok(metadata) if metadata.file_type().is_socket() => None,
        Ok(_) => Some("not_socket"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some("socket_missing"),
        Err(_) => Some("socket_unreadable"),
    }
}

fn is_live_pid<F>(pid: u32, is_process_alive: &F) -> bool
where
    F: Fn(u32) -> bool,
{
    pid > 0 && libc::pid_t::try_from(pid).is_ok() && is_process_alive(pid)
}

async fn authenticated_probe(endpoint: &DaemonEndpoint) -> ProbeOutcome {
    enum ProbeError {
        Connect(std::io::Error),
        Handshake,
    }

    let attempt = timeout(SIBLING_HANDSHAKE_TIMEOUT, async {
        let mut stream = UnixStream::connect(&endpoint.bind)
            .await
            .map_err(ProbeError::Connect)?;
        let request = IpcHandshakeRequest {
            protocol_version: endpoint.protocol_version,
            auth_token: endpoint.auth_token.clone(),
            scope: endpoint.scope.clone(),
        };
        let mut payload = serde_json::to_vec(&request).map_err(|_| ProbeError::Handshake)?;
        payload.push(b'\n');
        stream
            .write_all(&payload)
            .await
            .map_err(|_| ProbeError::Handshake)?;
        let response = read_bounded_handshake_response(&mut stream)
            .await
            .map_err(|_| ProbeError::Handshake)?;
        let response = serde_json::from_slice::<IpcHandshakeResponse>(&response)
            .map_err(|_| ProbeError::Handshake)?;
        validate_handshake(endpoint, &request, &response).map_err(|_| ProbeError::Handshake)
    })
    .await;

    match attempt {
        Ok(Ok(())) => ProbeOutcome::Reachable,
        Ok(Err(ProbeError::Connect(error)))
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            ProbeOutcome::DefinitelyUnavailable
        }
        Ok(Err(ProbeError::Connect(_))) | Ok(Err(ProbeError::Handshake)) | Err(_) => {
            ProbeOutcome::Uncertain
        }
    }
}

async fn read_bounded_handshake_response(stream: &mut UnixStream) -> Result<Vec<u8>, ()> {
    let mut response = Vec::with_capacity(512);
    let mut chunk = [0_u8; 512];
    loop {
        let read = stream.read(&mut chunk).await.map_err(|_| ())?;
        if read == 0 {
            return Err(());
        }
        let bytes = &chunk[..read];
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            if response.len() + newline > MAX_HANDSHAKE_RESPONSE_BYTES {
                return Err(());
            }
            response.extend_from_slice(&bytes[..newline]);
            return Ok(response);
        }
        if response.len() + bytes.len() > MAX_HANDSHAKE_RESPONSE_BYTES {
            return Err(());
        }
        response.extend_from_slice(bytes);
    }
}

#[cfg(test)]
mod tests {
    use gwt_core::daemon::{
        persist_endpoint, RuntimeScope, RuntimeTarget, DAEMON_PROTOCOL_VERSION,
    };
    use std::os::unix::net::UnixListener;
    use tempfile::TempDir;

    use super::*;

    struct Fixture {
        temp: TempDir,
        gwt_home: PathBuf,
        caller: RuntimeScope,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("tempdir");
            let gwt_home = temp.path().join(".gwt");
            let caller = scope(&temp, "caller");
            Self {
                temp,
                gwt_home,
                caller,
            }
        }

        fn endpoint(&self, worktree: &str, pid: u32) -> DaemonEndpoint {
            DaemonEndpoint::new(
                scope(&self.temp, worktree),
                pid,
                self.socket_path(worktree).display().to_string(),
                format!("token-{pid}"),
                "test-daemon".to_string(),
            )
        }

        fn socket_path(&self, name: &str) -> PathBuf {
            let path = self.temp.path().join(format!("{name}.sock"));
            let listener = UnixListener::bind(&path).expect("bind test socket");
            drop(listener);
            path
        }

        fn persist(&self, endpoint: &DaemonEndpoint) {
            persist_endpoint(&endpoint.scope.endpoint_path(&self.gwt_home), endpoint)
                .expect("persist endpoint");
        }
    }

    fn scope(temp: &TempDir, worktree: &str) -> RuntimeScope {
        let root = temp.path().join(worktree);
        fs::create_dir_all(&root).expect("project root");
        RuntimeScope::new("repo-1", worktree, root, RuntimeTarget::Host).expect("scope")
    }

    fn resolve_assuming_reachable<F>(
        gwt_home: &Path,
        requested_scope: &RuntimeScope,
        expected_protocol_version: u32,
        is_process_alive: F,
    ) -> Result<DaemonEndpoint, Box<ResolutionFailure>>
    where
        F: Fn(u32) -> bool,
    {
        resolve_with_probe(
            gwt_home,
            requested_scope,
            expected_protocol_version,
            is_process_alive,
            |_| ProbeOutcome::Reachable,
        )
    }

    #[test]
    fn exact_endpoint_wins_over_live_sibling() {
        let fixture = Fixture::new();
        let exact = DaemonEndpoint::new(
            fixture.caller.clone(),
            11,
            fixture.socket_path("exact").display().to_string(),
            "exact-token".to_string(),
            "test-daemon".to_string(),
        );
        fixture.persist(&exact);
        fixture.persist(&fixture.endpoint("sibling", 12));

        assert_eq!(
            resolve_assuming_reachable(
                &fixture.gwt_home,
                &fixture.caller,
                DAEMON_PROTOCOL_VERSION,
                |_| true
            )
            .expect("exact endpoint"),
            exact
        );
    }

    #[test]
    fn exact_endpoint_does_not_scan_malformed_sibling_evidence() {
        let fixture = Fixture::new();
        let exact = DaemonEndpoint::new(
            fixture.caller.clone(),
            13,
            fixture.socket_path("exact").display().to_string(),
            "exact-token".to_string(),
            "test-daemon".to_string(),
        );
        fixture.persist(&exact);
        fs::write(
            fixture
                .caller
                .daemon_dir(&fixture.gwt_home)
                .join("broken.json"),
            b"not-json",
        )
        .expect("malformed sibling");

        assert_eq!(
            resolve_assuming_reachable(
                &fixture.gwt_home,
                &fixture.caller,
                DAEMON_PROTOCOL_VERSION,
                |_| true
            )
            .expect("exact endpoint"),
            exact
        );
    }

    #[test]
    fn dead_exact_falls_back_without_deleting_evidence() {
        let fixture = Fixture::new();
        let exact = DaemonEndpoint::new(
            fixture.caller.clone(),
            21,
            fixture.socket_path("exact").display().to_string(),
            "exact-token".to_string(),
            "test-daemon".to_string(),
        );
        let sibling = fixture.endpoint("sibling", 22);
        fixture.persist(&exact);
        fixture.persist(&sibling);

        assert_eq!(
            resolve_assuming_reachable(
                &fixture.gwt_home,
                &fixture.caller,
                DAEMON_PROTOCOL_VERSION,
                |pid| pid == sibling.pid,
            )
            .expect("unique sibling"),
            sibling
        );
        assert!(fixture.caller.endpoint_path(&fixture.gwt_home).exists());
    }

    #[test]
    fn multiple_live_siblings_fail_closed_with_recovery() {
        let fixture = Fixture::new();
        fixture.persist(&fixture.endpoint("sibling-a", 31));
        fixture.persist(&fixture.endpoint("sibling-b", 32));

        let failure = resolve_assuming_reachable(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |_| true,
        )
        .expect_err("ambiguity");
        assert_eq!(failure.kind, FailureKind::Ambiguous);
        assert_eq!(failure.candidate_count, 2);
        let diagnostic = failure.to_string();
        assert!(diagnostic.contains("owner worktree"));
        assert!(!diagnostic.contains("token-31"));
        assert!(!diagnostic.contains("token-32"));
    }

    #[test]
    fn malformed_exact_is_preserved_and_blocks_fallback() {
        let fixture = Fixture::new();
        let exact_path = fixture.caller.endpoint_path(&fixture.gwt_home);
        fs::create_dir_all(exact_path.parent().expect("parent")).expect("parent");
        fs::write(&exact_path, b"{not-json").expect("malformed exact");
        fixture.persist(&fixture.endpoint("sibling", 42));

        let failure = resolve_assuming_reachable(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |_| true,
        )
        .expect_err("invalid evidence");
        assert_eq!(failure.kind, FailureKind::InvalidEvidence);
        assert_eq!(fs::read(&exact_path).expect("evidence"), b"{not-json");
    }

    #[test]
    fn live_scope_mismatch_blocks_fallback() {
        let fixture = Fixture::new();
        let wrong = fixture.endpoint("other", 51);
        persist_endpoint(&fixture.caller.endpoint_path(&fixture.gwt_home), &wrong)
            .expect("wrong exact");

        let failure = resolve_assuming_reachable(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |_| true,
        )
        .expect_err("scope mismatch");
        assert_eq!(failure.kind, FailureKind::InvalidEvidence);
        assert!(failure.to_string().contains("scope_mismatch"));
    }

    #[test]
    fn live_protocol_mismatch_blocks_fallback() {
        let fixture = Fixture::new();
        let mut exact = DaemonEndpoint::new(
            fixture.caller.clone(),
            52,
            fixture.socket_path("exact").display().to_string(),
            "exact-token".to_string(),
            "test-daemon".to_string(),
        );
        exact.protocol_version += 1;
        fixture.persist(&exact);
        fixture.persist(&fixture.endpoint("sibling", 53));

        let failure = resolve_assuming_reachable(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |_| true,
        )
        .expect_err("protocol mismatch");
        assert_eq!(failure.kind, FailureKind::InvalidEvidence);
        assert!(failure.to_string().contains("protocol_mismatch"));
    }

    #[test]
    fn live_exact_with_missing_socket_blocks_sibling_fallback() {
        let fixture = Fixture::new();
        let exact = DaemonEndpoint::new(
            fixture.caller.clone(),
            54,
            fixture
                .temp
                .path()
                .join("missing.sock")
                .display()
                .to_string(),
            "exact-token".to_string(),
            "test-daemon".to_string(),
        );
        fixture.persist(&exact);
        fixture.persist(&fixture.endpoint("sibling", 55));

        let failure = resolve_assuming_reachable(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |_| true,
        )
        .expect_err("missing exact socket");
        assert_eq!(failure.kind, FailureKind::InvalidEvidence);
        assert!(failure.to_string().contains("socket_missing"));
    }

    #[test]
    fn malformed_sibling_is_preserved_and_fails_closed() {
        let fixture = Fixture::new();
        fixture.persist(&fixture.endpoint("valid", 58));
        let daemon_dir = fixture.caller.daemon_dir(&fixture.gwt_home);
        fs::create_dir_all(&daemon_dir).expect("daemon dir");
        let malformed = daemon_dir.join("broken.json");
        fs::write(&malformed, b"not-json").expect("malformed sibling");

        let failure = resolve_assuming_reachable(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |_| true,
        )
        .expect_err("invalid sibling evidence");
        assert_eq!(failure.kind, FailureKind::InvalidEvidence);
        assert_eq!(failure.exact_outcome, "missing");
        assert_eq!(failure.candidate_count, 1);
        let diagnostic = failure.to_string();
        assert!(diagnostic.contains("sibling_malformed"));
        assert!(!diagnostic.contains("token-58"));
        assert!(malformed.exists());
    }

    #[test]
    fn invalid_pid_range_and_missing_socket_are_not_live_candidates() {
        let fixture = Fixture::new();
        fixture.persist(&fixture.endpoint("invalid-pid", u32::MAX));
        let missing_socket = DaemonEndpoint::new(
            scope(&fixture.temp, "missing-socket"),
            59,
            fixture
                .temp
                .path()
                .join("absent.sock")
                .display()
                .to_string(),
            "token-59".to_string(),
            "test-daemon".to_string(),
        );
        fixture.persist(&missing_socket);

        let failure = resolve_assuming_reachable(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |pid| {
                assert_ne!(pid, u32::MAX, "out-of-range pid must not reach OS probe");
                true
            },
        )
        .expect_err("invalid candidates");
        assert_eq!(failure.kind, FailureKind::Missing);
        let diagnostic = failure.to_string();
        assert!(diagnostic.contains("dead=1"));
        assert!(diagnostic.contains("socket_missing=1"));
    }

    #[test]
    fn sibling_scan_rejects_foreign_target_protocol_and_metadata_path() {
        let fixture = Fixture::new();
        let daemon_dir = fixture.caller.daemon_dir(&fixture.gwt_home);
        fs::create_dir_all(&daemon_dir).expect("daemon dir");

        let foreign_root = fixture.temp.path().join("foreign");
        fs::create_dir_all(&foreign_root).expect("foreign root");
        let foreign = DaemonEndpoint::new(
            RuntimeScope::new("repo-2", "foreign", foreign_root, RuntimeTarget::Host)
                .expect("foreign scope"),
            71,
            fixture.socket_path("foreign").display().to_string(),
            "token-71".to_string(),
            "test-daemon".to_string(),
        );
        persist_endpoint(&daemon_dir.join("foreign.json"), &foreign).expect("foreign endpoint");

        let docker_root = fixture.temp.path().join("docker");
        fs::create_dir_all(&docker_root).expect("docker root");
        let docker = DaemonEndpoint::new(
            RuntimeScope::new("repo-1", "docker", docker_root, RuntimeTarget::Docker)
                .expect("docker scope"),
            72,
            fixture.socket_path("docker").display().to_string(),
            "token-72".to_string(),
            "test-daemon".to_string(),
        );
        fixture.persist(&docker);

        let mut old_protocol = fixture.endpoint("old-protocol", 73);
        old_protocol.protocol_version += 1;
        fixture.persist(&old_protocol);

        let wrong_path = fixture.endpoint("actual-worktree", 74);
        persist_endpoint(&daemon_dir.join("wrong-name.json"), &wrong_path)
            .expect("wrong path endpoint");

        let failure = resolve_assuming_reachable(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |_| true,
        )
        .expect_err("no compatible sibling");
        assert_eq!(failure.kind, FailureKind::Missing);
        let diagnostic = failure.to_string();
        assert!(diagnostic.contains("repo_mismatch=1"));
        assert!(diagnostic.contains("target_mismatch=1"));
        assert!(diagnostic.contains("protocol_mismatch=1"));
        assert!(diagnostic.contains("metadata_path_mismatch=1"));
    }

    /// Issue #2338 stopped the GUI front door from writing this sentinel, but
    /// gwt homes upgraded from an older build can still hold one on disk, so
    /// the resolver must keep rejecting it instead of treating it as reachable.
    #[test]
    fn internal_front_door_is_not_a_candidate() {
        let fixture = Fixture::new();
        let mut front_door = fixture.endpoint("front-door", 61);
        front_door.bind = "internal://gwt-front-door".to_string();
        fixture.persist(&front_door);

        let failure = resolve_assuming_reachable(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |_| true,
        )
        .expect_err("no daemon socket");
        assert_eq!(failure.kind, FailureKind::Missing);
        assert_eq!(failure.candidate_count, 0);
        assert!(failure.to_string().contains("unsupported_transport=1"));
    }

    #[test]
    fn missing_daemon_reports_exact_path_and_start_recovery() {
        let fixture = Fixture::new();
        let failure = resolve_assuming_reachable(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |_| true,
        )
        .expect_err("missing daemon");
        let diagnostic = failure.to_string();

        assert_eq!(failure.kind, FailureKind::Missing);
        assert!(diagnostic.contains(
            &fixture
                .caller
                .endpoint_path(&fixture.gwt_home)
                .display()
                .to_string()
        ));
        assert!(diagnostic.contains("gwtd daemon start"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_socket_and_reused_pid_do_not_create_false_ambiguity() {
        use std::time::Duration;

        use super::super::{broadcast::BroadcastHub, server};

        let fixture = Fixture::new();
        let live_socket = fixture.temp.path().join("live-daemon.sock");
        let live_scope = scope(&fixture.temp, "live");
        let live = DaemonEndpoint::new(
            live_scope.clone(),
            std::process::id(),
            live_socket.display().to_string(),
            "live-token".to_string(),
            "test-daemon".to_string(),
        );
        let stale = fixture.endpoint("stale", std::process::id());
        fixture.persist(&live);
        fixture.persist(&stale);

        let server_endpoint = live.clone();
        let server_socket = live_socket.clone();
        let server_endpoint_path = live_scope.endpoint_path(&fixture.gwt_home);
        let server_handle = tokio::spawn(async move {
            server::run_server(
                server_endpoint,
                server_socket,
                server_endpoint_path,
                BroadcastHub::new(),
            )
            .await
        });
        for _ in 0..50 {
            if live_socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(live_socket.exists(), "daemon socket did not appear");

        let resolved = resolve(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |pid| pid == std::process::id(),
        )
        .await
        .expect("stale socket must not create ambiguity");
        assert_eq!(resolved, live);

        server_handle.abort();
        let _ = server_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slow_live_and_healthy_live_authorities_fail_closed() {
        use std::{
            io::{BufRead, BufReader},
            sync::mpsc,
            thread,
            time::Duration,
        };

        use super::super::{broadcast::BroadcastHub, server};

        let fixture = Fixture::new();
        let healthy_socket = fixture.temp.path().join("healthy.sock");
        let healthy_scope = scope(&fixture.temp, "healthy");
        let healthy = DaemonEndpoint::new(
            healthy_scope.clone(),
            std::process::id(),
            healthy_socket.display().to_string(),
            "healthy-token".to_string(),
            "test-daemon".to_string(),
        );
        let slow_socket = fixture.temp.path().join("slow.sock");
        let slow_listener = UnixListener::bind(&slow_socket).expect("bind slow socket");
        let slow = DaemonEndpoint::new(
            scope(&fixture.temp, "slow"),
            std::process::id(),
            slow_socket.display().to_string(),
            "slow-token".to_string(),
            "test-daemon".to_string(),
        );
        fixture.persist(&healthy);
        fixture.persist(&slow);

        let healthy_endpoint = healthy.clone();
        let server_socket = healthy_socket.clone();
        let server_endpoint_path = healthy_scope.endpoint_path(&fixture.gwt_home);
        let healthy_server = tokio::spawn(async move {
            server::run_server(
                healthy_endpoint,
                server_socket,
                server_endpoint_path,
                BroadcastHub::new(),
            )
            .await
        });
        for _ in 0..50 {
            if healthy_socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            healthy_socket.exists(),
            "healthy daemon socket did not appear"
        );

        let (release_slow, await_resolver) = mpsc::channel();
        let slow_server = thread::spawn(move || {
            let (stream, _) = slow_listener.accept().expect("accept slow probe");
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            reader.read_line(&mut request).expect("read slow handshake");
            assert!(!request.is_empty());
            await_resolver
                .recv_timeout(Duration::from_secs(2))
                .expect("resolver must fail closed before slow server exits");
        });

        let failure = resolve(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |pid| pid == std::process::id(),
        )
        .await
        .expect_err("slow live authority must block unique routing");
        assert_eq!(failure.kind, FailureKind::InvalidEvidence);
        assert_eq!(failure.candidate_count, 2);
        assert!(failure.to_string().contains("sibling_probe_uncertain"));

        release_slow.send(()).expect("release slow server");
        slow_server.join().expect("slow server");
        healthy_server.abort();
        let _ = healthy_server.await;
    }

    #[tokio::test]
    async fn handshake_response_reader_rejects_oversized_frame() {
        use tokio::io::AsyncWriteExt;

        let (mut reader, mut writer) = UnixStream::pair().expect("socket pair");
        let oversized = vec![b'x'; MAX_HANDSHAKE_RESPONSE_BYTES + 1];
        let server = tokio::spawn(async move {
            let _ = writer.write_all(&oversized).await;
            let _ = writer.write_all(b"\n").await;
        });

        assert!(read_bounded_handshake_response(&mut reader).await.is_err());
        let _ = server.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resolved_sibling_scope_and_token_complete_subscribe_handshake() {
        use std::time::Duration;

        use gwt_core::daemon::{ClientFrame, DaemonFrame};

        use super::super::{broadcast::BroadcastHub, client::DaemonClient, server};

        let fixture = Fixture::new();
        let socket_path = fixture.temp.path().join("daemon.sock");
        let sibling_scope = scope(&fixture.temp, "sibling");
        let endpoint = DaemonEndpoint::new(
            sibling_scope.clone(),
            std::process::id(),
            socket_path.display().to_string(),
            "sibling-token".to_string(),
            "test-daemon".to_string(),
        );
        fixture.persist(&endpoint);

        let server_endpoint = endpoint.clone();
        let server_socket = socket_path.clone();
        let server_endpoint_path = sibling_scope.endpoint_path(&fixture.gwt_home);
        let server_handle = tokio::spawn(async move {
            server::run_server(
                server_endpoint,
                server_socket,
                server_endpoint_path,
                BroadcastHub::new(),
            )
            .await
        });
        for _ in 0..50 {
            if socket_path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(socket_path.exists(), "daemon socket did not appear");

        let resolved = resolve(
            &fixture.gwt_home,
            &fixture.caller,
            DAEMON_PROTOCOL_VERSION,
            |pid| pid == std::process::id(),
        )
        .await
        .expect("unique sibling");
        let mut client = DaemonClient::connect(&resolved)
            .await
            .expect("sibling handshake");
        client
            .send_frame(&ClientFrame::Subscribe {
                channels: vec!["issue-monitor".to_string()],
            })
            .await
            .expect("subscribe frame");
        assert_eq!(
            client.read_frame::<DaemonFrame>().await.expect("ack"),
            DaemonFrame::Ack
        );

        drop(client);
        server_handle.abort();
        let _ = server_handle.await;
    }
}
