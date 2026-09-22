//! Host contract preflight (SPEC #3248 FR-242 / AS-220, Issue #4546).
//!
//! A generation is authority. Minting one against a Host that cannot carry it
//! leaves the caller holding a record nobody honours: the launch materializes
//! an Execution Control Record, a Session binding, a Work projection, a
//! capability and a set of obligations, and only then discovers that the Host
//! serving them speaks an older contract. Everything written by that point is
//! real, and none of it can be honestly settled.
//!
//! The preflight moves that discovery in front of the first byte. It asks the
//! running Host one question — *which execution contract do you serve?* — over
//! the authenticated agent bridge, and refuses the generation when the answer
//! is anything but a match.
//!
//! Three properties make it usable as a gate:
//!
//! - **It is side-effect free.** The Host answers from constants and the
//!   presented capability, so a refused preflight leaves the generation
//!   ledger, the Execution Control Record, the Session, the Work projection,
//!   the obligations, the verification state and Git byte-identical.
//! - **It has no local fallback.** A Host that cannot be reached, cannot be
//!   parsed, or disagrees is not replaced by the client's own authority. The
//!   caller is returned to Inspection with the mismatch named.
//! - **It names the next operation.** Every refusal carries the operation the
//!   agent runs next, because a diagnostic an agent cannot act on stalls the
//!   same way the missing contract would have.

use std::io;

use crate::{
    AgentHostContractReceipt, AgentHostContractRequest, AGENT_HOST_CONTRACT_SCHEMA_VERSION,
    EXECUTION_GENERATION_CONTRACT_VERSION,
};

/// The execution-generation contract this build requires of a Host.
pub const REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION: u32 =
    EXECUTION_GENERATION_CONTRACT_VERSION;

/// Whether this process is itself a Host.
///
/// A Host is its own execution authority, so asking it to prove a contract
/// against some *other* Host is both meaningless and actively wrong: a `gwt`
/// launched from inside an agent pane inherits that pane's bridge environment,
/// and without this marker it would preflight itself against the pane's
/// parent Host and refuse every launch it serves.
static HOST_PROCESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Record that this process serves the Host role. Called once as the embedded
/// server comes up, before it can materialize anything.
pub fn mark_host_process() {
    HOST_PROCESS.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[must_use]
pub fn is_host_process() -> bool {
    HOST_PROCESS.load(std::sync::atomic::Ordering::SeqCst)
}

/// The raw result of asking a Host for its contract, before it is judged.
///
/// Separating the transport outcome from the verdict is what makes every
/// AS-220 case reachable in a test without standing up seven different Hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostContractProbe {
    /// The Host returned a body that parsed as a receipt.
    Answered(Box<AgentHostContractReceipt>),
    /// The Host could not be reached at all.
    Unreachable { detail: String },
    /// The Host answered with an HTTP status instead of a receipt.
    Rejected { http_status: u16, detail: String },
    /// A response arrived but was not a receipt.
    Malformed { detail: String },
}

/// What the Host proved about its contract (AS-220).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostContractOutcome {
    /// The Host serves the required execution-generation contract.
    Compatible,
    /// No Host answered: it is down, restarting, or behind a dead socket.
    Unavailable { detail: String },
    /// The Host answered, but has no contract route — it predates it.
    RouteMissing { http_status: u16 },
    /// The Host refused the question because its own state conflicts with the
    /// caller's (HTTP 409).
    Conflict { detail: String },
    /// Something answered, but not with a receipt this client can trust.
    InvalidReceipt { detail: String },
    /// A Host answered for a different authority than the caller's.
    MismatchedAuthority { mismatched_fields: Vec<String> },
    /// The Host serves an older execution-generation contract.
    OldSchema { actual: u32 },
}

impl HostContractOutcome {
    /// Stable machine token, for diagnostics and tests.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Compatible => "compatible",
            Self::Unavailable { .. } => "unavailable",
            Self::RouteMissing { .. } => "route_missing",
            Self::Conflict { .. } => "conflict",
            Self::InvalidReceipt { .. } => "invalid_receipt",
            Self::MismatchedAuthority { .. } => "mismatched_authority",
            Self::OldSchema { .. } => "old_schema",
        }
    }
}

/// The preflight verdict: expected versus actual authority, plus the route out
/// (FR-242, AC-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostContractDiagnosis {
    pub outcome: HostContractOutcome,
    pub expected_contract_version: u32,
    pub actual_contract_version: Option<u32>,
    pub host_instance_id: Option<String>,
    pub host_version: Option<String>,
}

impl HostContractDiagnosis {
    #[must_use]
    pub fn is_compatible(&self) -> bool {
        self.outcome == HostContractOutcome::Compatible
    }

    /// The JSON operation the agent runs next.
    ///
    /// Every variant names one. A Host that must be updated still leaves the
    /// agent something to run once it has been: re-reading `execution.status`
    /// is how it learns whether the new Host carries the authority forward.
    #[must_use]
    pub fn next_operation(&self) -> &'static str {
        match self.outcome {
            HostContractOutcome::Compatible => "",
            HostContractOutcome::MismatchedAuthority { .. } => "execution.adopt",
            HostContractOutcome::Conflict { .. } => "workspace.ensure",
            _ => "execution.status",
        }
    }

    /// What a human must do before that operation can succeed.
    #[must_use]
    pub fn recovery_action(&self) -> String {
        match &self.outcome {
            HostContractOutcome::Compatible => String::new(),
            HostContractOutcome::Unavailable { .. } => {
                "start or restart the gwt Host that owns this worktree".to_string()
            }
            HostContractOutcome::RouteMissing { .. } | HostContractOutcome::OldSchema { .. } => {
                format!(
                    "update the running gwt Host{} and restart it",
                    self.host_version
                        .as_deref()
                        .map(|version| format!(" (currently {version})"))
                        .unwrap_or_default()
                )
            }
            HostContractOutcome::Conflict { .. } => {
                "reconcile this Session's canonical assignment with the Host".to_string()
            }
            HostContractOutcome::InvalidReceipt { .. } => {
                "restart the gwt Host so it reissues a trustworthy capability".to_string()
            }
            HostContractOutcome::MismatchedAuthority { .. } => {
                "relaunch this Session against the Host that holds its authority".to_string()
            }
        }
    }

    #[must_use]
    pub fn describe(&self) -> String {
        if self.is_compatible() {
            return format!(
                "host contract {} is compatible",
                self.expected_contract_version
            );
        }
        let detail = match &self.outcome {
            HostContractOutcome::Compatible => String::new(),
            HostContractOutcome::Unavailable { detail } => {
                format!("no Host answered the contract preflight: {detail}")
            }
            HostContractOutcome::RouteMissing { http_status } => format!(
                "the running Host has no execution contract route (http_status={http_status})"
            ),
            HostContractOutcome::Conflict { detail } => {
                format!("the running Host refused the contract preflight: {detail}")
            }
            HostContractOutcome::InvalidReceipt { detail } => {
                format!("the contract receipt could not be trusted: {detail}")
            }
            HostContractOutcome::MismatchedAuthority { mismatched_fields } => format!(
                "the contract receipt names another authority (mismatched_fields={})",
                mismatched_fields.join(",")
            ),
            HostContractOutcome::OldSchema { actual } => format!(
                "the running Host serves execution contract {actual}, older than the required {}",
                self.expected_contract_version
            ),
        };
        let actual = self
            .actual_contract_version
            .map_or_else(|| "unknown".to_string(), |version| version.to_string());
        format!(
            "[{}] {detail}; expected execution contract {}, actual {actual}{}; {}, then run JSON operation `{}`. No local authority fallback was attempted.",
            self.outcome.code(),
            self.expected_contract_version,
            self.host_instance_id
                .as_deref()
                .map(|id| format!(" (host_instance_id={id})"))
                .unwrap_or_default(),
            self.recovery_action(),
            self.next_operation(),
        )
    }
}

fn diagnosis(outcome: HostContractOutcome) -> HostContractDiagnosis {
    HostContractDiagnosis {
        outcome,
        expected_contract_version: REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION,
        actual_contract_version: None,
        host_instance_id: None,
        host_version: None,
    }
}

/// HTTP statuses that mean the Host is not there *right now* rather than that
/// it disagrees. They are the shapes a Host emits while restarting or behind a
/// proxy, so they classify as Unavailable and not as a contract verdict.
fn is_transport_status(http_status: u16) -> bool {
    matches!(http_status, 408 | 429 | 500 | 502 | 503 | 504)
}

/// Judge one probe against this client's required contract (AC-2).
///
/// The order matters: envelope integrity is checked before authority, and
/// authority before version, so a forged or cross-wired receipt can never
/// present itself as a merely outdated Host.
#[must_use]
pub fn classify(
    probe: HostContractProbe,
    expected: &AgentHostContractRequest,
) -> HostContractDiagnosis {
    match probe {
        HostContractProbe::Unreachable { detail } => {
            diagnosis(HostContractOutcome::Unavailable { detail })
        }
        HostContractProbe::Malformed { detail } => {
            diagnosis(HostContractOutcome::InvalidReceipt { detail })
        }
        HostContractProbe::Rejected {
            http_status,
            detail,
        } => match http_status {
            404 | 405 => diagnosis(HostContractOutcome::RouteMissing { http_status }),
            409 => diagnosis(HostContractOutcome::Conflict { detail }),
            401 | 403 => diagnosis(HostContractOutcome::MismatchedAuthority {
                mismatched_fields: vec!["capability".to_string()],
            }),
            status if is_transport_status(status) => {
                diagnosis(HostContractOutcome::Unavailable { detail })
            }
            status => diagnosis(HostContractOutcome::InvalidReceipt {
                detail: format!("unexpected http_status={status}: {detail}"),
            }),
        },
        HostContractProbe::Answered(receipt) => {
            let mut verdict = diagnosis(HostContractOutcome::Compatible);
            verdict.actual_contract_version = Some(receipt.execution_generation_contract_version);
            verdict.host_instance_id = Some(receipt.host_instance_id.clone());
            verdict.host_version = Some(receipt.host_version.clone());

            if receipt.schema_version != AGENT_HOST_CONTRACT_SCHEMA_VERSION {
                verdict.outcome = HostContractOutcome::InvalidReceipt {
                    detail: format!(
                        "receipt schema {} is not the expected {AGENT_HOST_CONTRACT_SCHEMA_VERSION}",
                        receipt.schema_version
                    ),
                };
                return verdict;
            }
            if receipt.operation_id != expected.operation_id || receipt.nonce != expected.nonce {
                verdict.outcome = HostContractOutcome::InvalidReceipt {
                    detail: "the receipt does not echo this preflight's operation id and nonce"
                        .to_string(),
                };
                return verdict;
            }
            // A zero capability generation is *not* a defect here: the
            // preflight runs before a generation exists, so an Inspection
            // principal legitimately carries none. Only the Host's own
            // identity has to be present, because that is what the update
            // instruction names.
            if receipt.host_instance_id.trim().is_empty() || receipt.host_version.trim().is_empty()
            {
                verdict.outcome = HostContractOutcome::InvalidReceipt {
                    detail: "the receipt omits the Host identity".to_string(),
                };
                return verdict;
            }
            if receipt.execution_generation_contract_version
                < REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION
            {
                verdict.outcome = HostContractOutcome::OldSchema {
                    actual: receipt.execution_generation_contract_version,
                };
            }
            verdict
        }
    }
}

/// Judge a probe whose answer must also belong to `expected_session_id`.
///
/// Split from [`classify`] because the caller does not always have a Session:
/// a genesis preflight runs before the binding exists, and demanding one there
/// would refuse the very launch the contract is meant to protect.
#[must_use]
pub fn classify_for_session(
    probe: HostContractProbe,
    expected: &AgentHostContractRequest,
    expected_session_id: &str,
) -> HostContractDiagnosis {
    let session_id = match &probe {
        HostContractProbe::Answered(receipt) => Some(receipt.session_id.clone()),
        _ => None,
    };
    let mut verdict = classify(probe, expected);
    if verdict.is_compatible() {
        if let Some(session_id) = session_id {
            if session_id != expected_session_id {
                verdict.outcome = HostContractOutcome::MismatchedAuthority {
                    mismatched_fields: vec!["session_id".to_string()],
                };
            }
        }
    }
    verdict
}

/// Build one preflight request with a fresh nonce.
#[must_use]
pub fn request_for(stage: &str) -> AgentHostContractRequest {
    AgentHostContractRequest {
        schema_version: AGENT_HOST_CONTRACT_SCHEMA_VERSION,
        operation_id: format!("host-contract:{stage}"),
        nonce: uuid::Uuid::new_v4().to_string(),
    }
}

/// Why a preflight did not have to ask anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightScope {
    /// This process is the Host: it is its own execution authority.
    HostProcess,
    /// No agent bridge is configured, so the caller holds no Host-issued
    /// authority and there is no Host contract to prove. This is scope, not a
    /// fallback: nothing about a Host is being assumed.
    Unbridged,
}

/// Either the question did not apply, or the Host proved its contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightOutcome {
    NotApplicable(PreflightScope),
    Proven(HostContractDiagnosis),
}

/// The mandatory preflight, run before a bridged client issues a generation,
/// binds a Session, publishes Work, takes a capability, or creates an
/// obligation (FR-242, AC-1).
pub fn preflight(stage: &str) -> Result<PreflightOutcome, HostContractDiagnosis> {
    if is_host_process() {
        return Ok(PreflightOutcome::NotApplicable(PreflightScope::HostProcess));
    }
    let target = match crate::daemon_runtime::HookForwardTarget::from_env_strict() {
        Ok(Some(target)) => target,
        Ok(None) => return Ok(PreflightOutcome::NotApplicable(PreflightScope::Unbridged)),
        Err(detail) => {
            return Err(diagnosis(HostContractOutcome::InvalidReceipt {
                detail: format!("the agent bridge is misconfigured: {detail}"),
            }))
        }
    };
    let request = request_for(stage);
    let probe = crate::daemon_runtime::fetch_host_contract_via_agent_bridge(&target, &request);
    let verdict = match std::env::var(gwt_agent::GWT_SESSION_ID_ENV) {
        Ok(session_id) if !session_id.trim().is_empty() => {
            classify_for_session(probe, &request, session_id.trim())
        }
        _ => classify(probe, &request),
    };
    if verdict.is_compatible() {
        Ok(PreflightOutcome::Proven(verdict))
    } else {
        Err(verdict)
    }
}

/// Whether a Host that predates the contract route is refused yet.
///
/// The client and the Host ship in the same binary, so the first release to
/// carry this contract necessarily faces Hosts that do not serve the route.
/// Refusing them immediately would strand every already-running agent — the
/// bridged operations gated here include `execution.continue`, the recovery
/// path a wedged agent uses to get unstuck — behind an app restart it cannot
/// perform for itself.
///
/// So this one outcome is *staged*, the way the launch packet's rejection was
/// staged behind T-276: the mismatch is still probed, classified and reported
/// exactly like every other, and nothing local substitutes for the Host's
/// authority. Only the refusal waits.
///
/// Flip this to `true` once a Host serving `/internal/host-contract` has
/// shipped and the fleet has rolled forward; nothing else has to change.
pub const REFUSE_HOSTS_PREDATING_THE_CONTRACT_ROUTE: bool = false;

impl HostContractDiagnosis {
    /// Whether this verdict stops the operation.
    ///
    /// Compatible never does. Every incompatible verdict does, except a Host
    /// that predates the route while [`REFUSE_HOSTS_PREDATING_THE_CONTRACT_ROUTE`]
    /// is still staged.
    #[must_use]
    pub fn refuses(&self) -> bool {
        match self.outcome {
            HostContractOutcome::Compatible => false,
            HostContractOutcome::RouteMissing { .. } => REFUSE_HOSTS_PREDATING_THE_CONTRACT_ROUTE,
            _ => true,
        }
    }
}

/// The preflight as a gate: prove the Host contract, or fail with the whole
/// structured diagnosis (AC-1, AC-4).
///
/// Callers are the bridged senders that ask a Host to mint or bind execution
/// authority. The *local* materialization functions deliberately do not call
/// this: a worktree's own generation ledger is written by the process that
/// will serve it, so there is no second party whose contract could differ.
/// Gating them would also have meant gating every `cargo test` and build that
/// inherits an agent pane's ambient bridge environment.
pub fn require(stage: &str) -> io::Result<()> {
    match preflight(stage) {
        Ok(_) => Ok(()),
        Err(verdict) if !verdict.refuses() => Ok(()),
        Err(verdict) => Err(io::Error::other(format!(
            "host contract preflight refused `{stage}`: {}",
            verdict.describe()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    fn request() -> AgentHostContractRequest {
        AgentHostContractRequest {
            schema_version: AGENT_HOST_CONTRACT_SCHEMA_VERSION,
            operation_id: "host-contract:test".to_string(),
            nonce: "nonce-1".to_string(),
        }
    }

    fn receipt(request: &AgentHostContractRequest) -> AgentHostContractReceipt {
        AgentHostContractReceipt {
            schema_version: AGENT_HOST_CONTRACT_SCHEMA_VERSION,
            operation_id: request.operation_id.clone(),
            nonce: request.nonce.clone(),
            host_instance_id: "host-instance-1".to_string(),
            host_version: "9.101.1".to_string(),
            execution_generation_contract_version: REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION,
            session_id: "session-1".to_string(),
            capability_generation: 1,
        }
    }

    fn answered(receipt: AgentHostContractReceipt) -> HostContractProbe {
        HostContractProbe::Answered(Box::new(receipt))
    }

    // AC-2: the seven outcomes AS-220 has to tell apart, each reached from the
    // shape a Host actually produces. A collapse of any two of these is the
    // defect the preflight exists to prevent — "no Host" and "wrong Host" call
    // for opposite actions.
    #[test]
    fn every_host_contract_outcome_is_classified_distinctly() {
        let request = request();

        assert_eq!(
            classify(answered(receipt(&request)), &request).outcome,
            HostContractOutcome::Compatible
        );

        assert_eq!(
            classify(
                HostContractProbe::Unreachable {
                    detail: "connection refused".to_string()
                },
                &request
            )
            .outcome
            .code(),
            "unavailable"
        );

        for status in [404, 405] {
            assert_eq!(
                classify(
                    HostContractProbe::Rejected {
                        http_status: status,
                        detail: "no route".to_string()
                    },
                    &request
                )
                .outcome,
                HostContractOutcome::RouteMissing {
                    http_status: status
                },
                "http_status={status}"
            );
        }

        assert_eq!(
            classify(
                HostContractProbe::Rejected {
                    http_status: 409,
                    detail: "workspace_ensure_required".to_string()
                },
                &request
            )
            .outcome
            .code(),
            "conflict"
        );

        assert_eq!(
            classify(
                HostContractProbe::Malformed {
                    detail: "expected value".to_string()
                },
                &request
            )
            .outcome
            .code(),
            "invalid_receipt"
        );

        for status in [401, 403] {
            assert_eq!(
                classify(
                    HostContractProbe::Rejected {
                        http_status: status,
                        detail: "capability rejected".to_string()
                    },
                    &request
                )
                .outcome
                .code(),
                "mismatched_authority",
                "http_status={status}"
            );
        }

        let mut old = receipt(&request);
        old.execution_generation_contract_version =
            REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION - 1;
        let verdict = classify(answered(old), &request);
        assert_eq!(
            verdict.outcome,
            HostContractOutcome::OldSchema {
                actual: REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION - 1
            }
        );
        // AC-2: expected *and* actual, not just "incompatible".
        assert_eq!(
            verdict.expected_contract_version,
            REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION
        );
        assert_eq!(
            verdict.actual_contract_version,
            Some(REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION - 1)
        );
        assert_eq!(verdict.host_version.as_deref(), Some("9.101.1"));
    }

    // AC-2: a receipt that does not echo this preflight's own operation id and
    // nonce is a replay or a cross-wired answer. Accepting it would let a
    // stale Host vouch for a contract it was never asked about.
    #[test]
    fn a_replayed_or_reshaped_receipt_is_never_compatible() {
        let request = request();

        let mut replayed = receipt(&request);
        replayed.nonce = "someone-elses-nonce".to_string();
        assert_eq!(
            classify(answered(replayed), &request).outcome.code(),
            "invalid_receipt"
        );

        let mut wrong_operation = receipt(&request);
        wrong_operation.operation_id = "host-contract:other".to_string();
        assert_eq!(
            classify(answered(wrong_operation), &request).outcome.code(),
            "invalid_receipt"
        );

        let mut wrong_schema = receipt(&request);
        wrong_schema.schema_version = AGENT_HOST_CONTRACT_SCHEMA_VERSION + 1;
        assert_eq!(
            classify(answered(wrong_schema), &request).outcome.code(),
            "invalid_receipt"
        );

        let mut anonymous = receipt(&request);
        anonymous.host_instance_id = "  ".to_string();
        assert_eq!(
            classify(answered(anonymous), &request).outcome.code(),
            "invalid_receipt"
        );
    }

    // AC-2: a Host that answers for another Session holds another authority.
    // Envelope integrity is checked first, so a forged receipt cannot present
    // itself as merely belonging to someone else.
    #[test]
    fn a_receipt_for_another_session_is_mismatched_authority() {
        let request = request();
        assert_eq!(
            classify_for_session(answered(receipt(&request)), &request, "session-1")
                .outcome
                .code(),
            "compatible"
        );
        assert_eq!(
            classify_for_session(answered(receipt(&request)), &request, "session-2")
                .outcome
                .code(),
            "mismatched_authority"
        );
    }

    // AC-2: a preflight has zero local authority to fall back on, and says so.
    // AC-4: every refusal names the JSON operation the agent runs next plus
    // the human action that makes it succeed.
    #[test]
    fn every_refusal_names_the_next_operation_and_refuses_local_fallback() {
        let request = request();
        let mut old = receipt(&request);
        old.execution_generation_contract_version =
            REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION - 1;

        let refusals = [
            classify(
                HostContractProbe::Unreachable {
                    detail: "connection refused".to_string(),
                },
                &request,
            ),
            classify(
                HostContractProbe::Rejected {
                    http_status: 404,
                    detail: "no route".to_string(),
                },
                &request,
            ),
            classify(
                HostContractProbe::Rejected {
                    http_status: 409,
                    detail: "conflict".to_string(),
                },
                &request,
            ),
            classify(
                HostContractProbe::Rejected {
                    http_status: 401,
                    detail: "unauthorized".to_string(),
                },
                &request,
            ),
            classify(
                HostContractProbe::Malformed {
                    detail: "expected value".to_string(),
                },
                &request,
            ),
            classify(answered(old), &request),
        ];

        for verdict in refusals {
            assert!(!verdict.is_compatible(), "{verdict:?}");
            let described = verdict.describe();
            assert!(
                !verdict.next_operation().is_empty(),
                "{} names no next operation",
                verdict.outcome.code()
            );
            assert!(
                described.contains(verdict.next_operation()),
                "{described} omits its own next operation"
            );
            assert!(
                !verdict.recovery_action().is_empty(),
                "{} names no recovery action",
                verdict.outcome.code()
            );
            assert!(
                described.contains("No local authority fallback was attempted"),
                "{described} does not disclaim a local fallback"
            );
            assert!(
                described.contains(verdict.outcome.code()),
                "{described} omits its own outcome code"
            );
        }
    }

    // AC-2: a Host that is restarting is not a Host that disagrees. These
    // statuses must stay Unavailable, because the recovery is to wait or
    // restart rather than to update a binary that is already current.
    #[test]
    fn transient_host_statuses_stay_unavailable() {
        let request = request();
        for status in [408, 429, 500, 502, 503, 504] {
            assert_eq!(
                classify(
                    HostContractProbe::Rejected {
                        http_status: status,
                        detail: "restarting".to_string()
                    },
                    &request
                )
                .outcome
                .code(),
                "unavailable",
                "http_status={status}"
            );
        }
    }

    // AC-1: the Host is its own execution authority. Without this, a `gwt`
    // started inside an agent pane inherits that pane's bridge environment and
    // preflights itself against the parent Host — refusing every launch it is
    // itself responsible for serving.
    #[test]
    fn a_host_process_never_preflights_itself_against_another_host() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let restore = is_host_process();
        let _url = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
            "http://127.0.0.1:1/internal/hook-live",
        );
        let _token = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "token");

        mark_host_process();
        let outcome = preflight("test").expect("a Host proves nothing to another Host");
        assert_eq!(
            outcome,
            PreflightOutcome::NotApplicable(PreflightScope::HostProcess)
        );
        HOST_PROCESS.store(restore, std::sync::atomic::Ordering::SeqCst);
    }

    // AC-1: an unbridged caller holds no Host-issued authority, so there is no
    // Host contract to prove. This is scope, not a fallback — nothing about a
    // Host is assumed, because no Host is involved.
    #[test]
    fn an_unbridged_caller_has_no_host_contract_to_prove() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let restore = is_host_process();
        HOST_PROCESS.store(false, std::sync::atomic::Ordering::SeqCst);
        let _url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        let _token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);

        assert_eq!(
            preflight("test").expect("an unbridged caller is in scope"),
            PreflightOutcome::NotApplicable(PreflightScope::Unbridged)
        );
        HOST_PROCESS.store(restore, std::sync::atomic::Ordering::SeqCst);
    }

    // The staged-rollout clause, stated as a test so it cannot drift silently.
    //
    // A Host that predates the contract route is still probed, classified and
    // reported as incompatible — the diagnosis is identical either way. Only
    // the refusal is staged, because the first release to carry this contract
    // necessarily faces Hosts that predate it, and refusing them would strand
    // every running agent behind an app restart it cannot perform for itself.
    #[test]
    fn only_the_refusal_of_a_pre_contract_host_is_staged() {
        let request = request();
        let route_missing = classify(
            HostContractProbe::Rejected {
                http_status: 404,
                detail: "no route".to_string(),
            },
            &request,
        );
        assert!(
            !route_missing.is_compatible(),
            "a 404 Host is not compatible"
        );
        assert_eq!(
            route_missing.refuses(),
            REFUSE_HOSTS_PREDATING_THE_CONTRACT_ROUTE,
            "the pre-contract Host refusal must follow its staging flag alone"
        );

        // Every other incompatible verdict refuses now, staged flag or not.
        let mut old = receipt(&request);
        old.execution_generation_contract_version =
            REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION - 1;
        for verdict in [
            classify(
                HostContractProbe::Unreachable {
                    detail: "refused".to_string(),
                },
                &request,
            ),
            classify(
                HostContractProbe::Rejected {
                    http_status: 409,
                    detail: "conflict".to_string(),
                },
                &request,
            ),
            classify(
                HostContractProbe::Rejected {
                    http_status: 401,
                    detail: "unauthorized".to_string(),
                },
                &request,
            ),
            classify(
                HostContractProbe::Malformed {
                    detail: "garbled".to_string(),
                },
                &request,
            ),
            classify(answered(old), &request),
        ] {
            assert!(
                verdict.refuses(),
                "{} must refuse regardless of staging",
                verdict.outcome.code()
            );
        }

        assert!(
            !classify(answered(receipt(&request)), &request).refuses(),
            "a compatible Host never refuses"
        );
    }

    // AC-1: every bridged request that asks a Host to mint or bind execution
    // authority proves the Host contract first, and proves it before the
    // request is built.
    //
    // This reads the source because the property is structural: a future
    // bridged authority sender that forgets the preflight is exactly the
    // regression the AC forbids, and no behavioural test can fail for a call
    // site nobody has written yet.
    //
    // The local materialization functions are deliberately absent. A
    // worktree's own generation ledger is written by the process that will
    // serve it, so there is no second party whose contract could differ —
    // and gating them would gate every `cargo test` and build that inherits
    // an agent pane's ambient bridge environment.
    #[test]
    fn every_bridged_authority_request_preflights_the_host_contract() {
        let bridge = include_str!("../daemon_runtime.rs");
        for (sender, stage) in [
            (
                "fn send_execution_continuation_via_agent_bridge_detailed(",
                "execution-continuation",
            ),
            (
                "pub fn send_execution_adoption_via_agent_bridge(",
                "execution-adoption",
            ),
        ] {
            let start = bridge
                .find(sender)
                .unwrap_or_else(|| panic!("{sender} no longer exists in daemon_runtime.rs"));
            let body = &bridge[start..];
            let guard = body
                .find("crate::cli::host_contract::require(")
                .unwrap_or_else(|| panic!("{sender} does not preflight the Host contract"));
            assert!(
                body[guard..]
                    .starts_with(&format!("crate::cli::host_contract::require(\"{stage}\")")),
                "{sender} preflights with the wrong stage"
            );
            // The guard has to precede the request itself.
            let request = body
                .find(".bearer_auth(")
                .expect("a bridged sender authenticates its request");
            assert!(
                guard < request,
                "{sender} sends its request before the Host contract preflight"
            );
        }

        // The local mint path stays ungated on purpose; pin that so it is not
        // "fixed" back into a fleet-wide outage.
        let execution_state = include_str!("execution_state.rs");
        assert!(
            !execution_state.contains("host_contract::require("),
            "local generation materialization must not preflight a remote Host"
        );
    }

    /// Serve `/internal/host-contract` from a real HTTP listener and return
    /// its base URL. `answer` decides what that Host is.
    ///
    /// Classification is unit-tested above; this exists to prove the *wire*:
    /// that the receipt the Host writes is the receipt this client reads.
    fn spawn_host(
        answer: impl Fn(AgentHostContractRequest) -> (u16, String) + Send + Sync + 'static,
    ) -> (String, std::sync::mpsc::Sender<()>) {
        use axum::{extract::Json as AxumJson, response::IntoResponse, routing::post, Router};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fake Host");
        let port = listener.local_addr().expect("fake Host address").port();
        let (shutdown, shutdown_rx) = std::sync::mpsc::channel::<()>();
        let answer = std::sync::Arc::new(answer);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("fake Host runtime");
            runtime.block_on(async move {
                let app = Router::new().route(
                    "/internal/host-contract",
                    post(
                        move |AxumJson(request): AxumJson<AgentHostContractRequest>| {
                            let answer = answer.clone();
                            async move {
                                let (status, body) = answer(request);
                                (
                                    axum::http::StatusCode::from_u16(status)
                                        .expect("fake Host status"),
                                    body,
                                )
                                    .into_response()
                            }
                        },
                    ),
                );
                listener
                    .set_nonblocking(true)
                    .expect("nonblocking listener");
                let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");
                axum::serve(listener, app)
                    .with_graceful_shutdown(async move {
                        let _ = tokio::task::spawn_blocking(move || shutdown_rx.recv()).await;
                    })
                    .await
                    .expect("fake Host serve");
            });
        });
        (
            format!("http://127.0.0.1:{port}/internal/hook-live"),
            shutdown,
        )
    }

    fn preflight_against(url: &str) -> Result<PreflightOutcome, HostContractDiagnosis> {
        let _url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, url);
        let _token = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "capability-token");
        preflight("execution-generation-genesis")
    }

    // AC-2 / AC-5: the compatible case, proven over the real wire against the
    // Host's own receipt builder. A classifier that agrees with a receipt this
    // code also invented would prove nothing about the two sides matching.
    #[test]
    fn a_current_host_proves_its_contract_over_the_wire() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let restore = is_host_process();
        HOST_PROCESS.store(false, std::sync::atomic::Ordering::SeqCst);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "session-wire");

        let (compatible, _compatible_shutdown) = spawn_host(|request| {
            let receipt = crate::describe_authenticated_host_contract(
                &request,
                "session-wire",
                "host-instance-wire",
                3,
            )
            .expect("the Host answers its own contract");
            (200, serde_json::to_string(&receipt).expect("receipt json"))
        });
        let outcome = preflight_against(&compatible).expect("a current Host proves its contract");
        let PreflightOutcome::Proven(verdict) = outcome else {
            panic!("a reachable current Host is not out of scope: {outcome:?}");
        };
        assert_eq!(verdict.outcome, HostContractOutcome::Compatible);
        assert_eq!(
            verdict.actual_contract_version,
            Some(REQUIRED_EXECUTION_GENERATION_CONTRACT_VERSION)
        );
        assert_eq!(
            verdict.host_instance_id.as_deref(),
            Some("host-instance-wire")
        );

        // AS-220: the same wire, an older Host. 404 is what a Host that
        // predates the route actually returns, and it must read as "update
        // the Host", never as a reason to mint the generation anyway.
        let (old, _old_shutdown) = spawn_host(|_| (404, String::new()));
        let verdict = preflight_against(&old).expect_err("an old Host cannot prove the contract");
        assert_eq!(
            verdict.outcome,
            HostContractOutcome::RouteMissing { http_status: 404 }
        );
        assert!(verdict.describe().contains("update the running gwt Host"));

        // AS-220: a Host that answers with something that is not a receipt is
        // not a Host this client may trust.
        let (garbled, _garbled_shutdown) =
            spawn_host(|_| (200, "{\"not\":\"a receipt\"}".to_string()));
        assert_eq!(
            preflight_against(&garbled)
                .expect_err("a garbled receipt cannot prove the contract")
                .outcome
                .code(),
            "invalid_receipt"
        );

        HOST_PROCESS.store(restore, std::sync::atomic::Ordering::SeqCst);
    }

    /// Every byte under `root`, keyed by relative path.
    ///
    /// Snapshotting whole trees rather than named files is deliberate: the
    /// zero-side-effect claim covers stores this test does not know the layout
    /// of, and a file the preflight creates somewhere unexpected is exactly
    /// the defect worth catching.
    fn tree_digest(root: &Path) -> Vec<(String, String)> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else if let Ok(bytes) = std::fs::read(&path) {
                    let relative = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.push((
                        relative,
                        format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes)),
                    ));
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort();
        out
    }

    // AC-3: a refused preflight is a no-op everywhere. The fixture carries a
    // generation ledger, an Execution Control Record, a Session, an obligation
    // and a verification plan, and both the worktree and the whole GWT home
    // are compared byte for byte across every refusal shape.
    //
    // This is the property the whole gate rests on: moving the Host check in
    // front of generation issuance is only safe if failing it writes nothing.
    #[test]
    fn a_refused_preflight_leaves_every_authority_store_byte_identical() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let restore = is_host_process();
        HOST_PROCESS.store(false, std::sync::atomic::Ordering::SeqCst);

        let home = tempfile::tempdir().expect("home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let session_id = "host-contract-snapshot-session";
        let _session_env = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session_id);

        let worktree = tempfile::tempdir().expect("worktree");
        let worktree = worktree.path();
        crate::cli::trusted_store::init_git_repo_with_origin(worktree);

        // Materialize the authority stores while nothing is bridged, so the
        // fixture itself passes the very gate under test.
        {
            let _url = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
            let _token = ScopedEnvVar::unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
            let owner = crate::cli::execution_state::ExecutionOwnerKey {
                kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
                number: 4546,
            };
            crate::cli::execution_state::materialize_at_launch(
                worktree,
                owner.kind,
                owner.number,
                session_id,
                "gwt-execute",
                false,
            )
            .expect("materialize the execution control record");
            crate::cli::execution_state::ensure_generation_ledger(
                worktree,
                owner,
                crate::cli::execution_state::LegacyActiveDisposition::Live,
            )
            .expect("materialize the generation ledger");
            let mut session =
                gwt_agent::Session::new(worktree, "work/issue-4546", gwt_agent::AgentId::Codex);
            session.id = session_id.to_string();
            session.linked_issue_number = Some(4546);
            session
                .save(&gwt_core::paths::gwt_sessions_dir())
                .expect("save the Session projection");
            crate::cli::action_obligation::mark_from_prompt(
                worktree,
                session_id,
                "$gwt-execute #4546",
            )
            .expect("arm an action obligation");
        }

        let before = (tree_digest(worktree), tree_digest(home.path()));
        // A byte-identity assertion over empty trees proves nothing, so pin
        // that the stores the AC names are actually present to be disturbed.
        let stored = |snapshot: &[(String, String)], needle: &str| {
            snapshot.iter().any(|(path, _)| path.contains(needle))
        };
        assert!(
            stored(&before.0, "execution-control.json"),
            "fixture has no Execution Control Record: {:?}",
            before.0
        );
        assert!(
            stored(&before.1, "generation") || stored(&before.0, "generation"),
            "fixture has no generation ledger: {:?}",
            before.1
        );
        assert!(
            stored(&before.1, "sessions"),
            "fixture has no Session projection: {:?}",
            before.1
        );
        assert!(
            stored(&before.0, ".git/HEAD"),
            "fixture has no Git bytes: {:?}",
            before.0
        );

        // Every refusal shape a bridged caller can hit: an unreachable Host,
        // and a misconfigured bridge that never leaves the process.
        for (url, token) in [
            ("http://127.0.0.1:1/internal/hook-live", "token"),
            ("http://127.0.0.1:1/internal/hook-live", ""),
        ] {
            let _url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, url);
            let _token = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, token);

            require("execution-continuation")
                .expect_err("a bridged caller with no Host must be refused");
            preflight("execution-continuation")
                .expect_err("an unreachable Host cannot prove a contract");

            let after = (tree_digest(worktree), tree_digest(home.path()));
            assert_eq!(
                before.0, after.0,
                "the worktree changed under a refused preflight"
            );
            assert_eq!(
                before.1, after.1,
                "the GWT home changed under a refused preflight"
            );
        }

        HOST_PROCESS.store(restore, std::sync::atomic::Ordering::SeqCst);
    }

    // AC-1 / AC-4: a bridged caller whose Host is not there is refused, and the
    // refusal carries the whole structured diagnosis rather than a bare error.
    #[test]
    fn a_bridged_caller_is_refused_when_no_host_answers() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let restore = is_host_process();
        HOST_PROCESS.store(false, std::sync::atomic::Ordering::SeqCst);
        // Port 1 is privileged and unbound: the connection is refused rather
        // than answered, which is exactly the Unavailable shape.
        let _url = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
            "http://127.0.0.1:1/internal/hook-live",
        );
        let _token = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "token");

        let verdict = preflight("execution-continuation")
            .expect_err("an unreachable Host cannot prove a contract");
        assert_eq!(verdict.outcome.code(), "unavailable");

        let error = require("execution-generation-genesis")
            .expect_err("the gate refuses what the preflight refused");
        let message = error.to_string();
        assert!(
            message.contains("execution-generation-genesis"),
            "{message}"
        );
        assert!(message.contains("execution.status"), "{message}");
        assert!(
            message.contains("No local authority fallback was attempted"),
            "{message}"
        );
        HOST_PROCESS.store(restore, std::sync::atomic::Ordering::SeqCst);
    }
}
