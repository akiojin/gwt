//! Machine-readable GitHub Issue comment claims for the auto-improve monitor.

use serde::{Deserialize, Serialize};

use crate::{
    client::{OwnerMutationError, OwnerMutationResult},
    ApiError, CommentId, CommentSnapshot, FetchResult, IssueClient, IssueNumber,
};

const CLAIM_BEGIN: &str = "<!-- gwt-auto-improve-claim v1 -->";
const CLAIM_END: &str = "<!-- /gwt-auto-improve-claim -->";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    Active,
    Released,
    Completed,
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimComment {
    pub comment_id: Option<CommentId>,
    pub claim_id: String,
    pub owner: String,
    pub issue_number: u64,
    pub status: ClaimStatus,
    pub heartbeat_at: String,
    pub expires_at: String,
    pub launched_work_id: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClaimParseError {
    #[error("claim marker not found")]
    MissingMarker,
    #[error("claim payload is invalid: {0}")]
    InvalidPayload(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ClaimPayload {
    claim_id: String,
    owner: String,
    issue_number: u64,
    status: ClaimStatus,
    heartbeat_at: String,
    expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    launched_work_id: Option<String>,
}

impl From<&ClaimComment> for ClaimPayload {
    fn from(value: &ClaimComment) -> Self {
        Self {
            claim_id: value.claim_id.clone(),
            owner: value.owner.clone(),
            issue_number: value.issue_number,
            status: value.status.clone(),
            heartbeat_at: value.heartbeat_at.clone(),
            expires_at: value.expires_at.clone(),
            launched_work_id: value.launched_work_id.clone(),
        }
    }
}

impl ClaimPayload {
    fn into_claim(self, comment_id: Option<CommentId>) -> ClaimComment {
        ClaimComment {
            comment_id,
            claim_id: self.claim_id,
            owner: self.owner,
            issue_number: self.issue_number,
            status: self.status,
            heartbeat_at: self.heartbeat_at,
            expires_at: self.expires_at,
            launched_work_id: self.launched_work_id,
        }
    }
}

pub fn render_claim_comment(claim: &ClaimComment) -> String {
    let payload =
        serde_json::to_string_pretty(&ClaimPayload::from(claim)).expect("claim payload serializes");
    format!("{CLAIM_BEGIN}\n```json\n{payload}\n```\n{CLAIM_END}\n\nManaged by gwt Issue Monitor.")
}

pub fn parse_claim_comment(
    comment_id: Option<CommentId>,
    body: &str,
) -> Result<ClaimComment, ClaimParseError> {
    let Some(start) = body.find(CLAIM_BEGIN) else {
        return Err(ClaimParseError::MissingMarker);
    };
    let after_begin = &body[start + CLAIM_BEGIN.len()..];
    let Some(end) = after_begin.find(CLAIM_END) else {
        return Err(ClaimParseError::MissingMarker);
    };
    let mut payload = after_begin[..end].trim();
    if let Some(stripped) = payload.strip_prefix("```json") {
        payload = stripped.trim();
    }
    if let Some(stripped) = payload.strip_suffix("```") {
        payload = stripped.trim();
    }
    serde_json::from_str::<ClaimPayload>(payload)
        .map(|payload| payload.into_claim(comment_id))
        .map_err(|err| ClaimParseError::InvalidPayload(err.to_string()))
}

pub fn claim_is_active(claim: &ClaimComment, now: &str) -> bool {
    claim.status == ClaimStatus::Active && claim.expires_at.as_str() > now
}

pub fn select_winning_claim<'a>(claims: &'a [ClaimComment], now: &str) -> Option<&'a ClaimComment> {
    claims
        .iter()
        .filter(|claim| claim_is_active(claim, now))
        .min_by(|left, right| {
            left.heartbeat_at
                .cmp(&right.heartbeat_at)
                .then_with(|| left.claim_id.cmp(&right.claim_id))
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimAcquireOutcome {
    Acquired(ClaimComment),
    Blocked(ClaimComment),
    Lost {
        own_claim: ClaimComment,
        winning_claim: ClaimComment,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimReleaseOutcome {
    Released(ClaimComment),
    AlreadyReleased(Option<ClaimComment>),
}

pub fn extract_claim_comments(comments: &[CommentSnapshot]) -> Vec<ClaimComment> {
    comments
        .iter()
        .filter_map(|comment| parse_claim_comment(Some(comment.id), &comment.body).ok())
        .collect()
}

pub fn acquire_claim<C: IssueClient + ?Sized>(
    client: &C,
    issue_number: IssueNumber,
    claim: ClaimComment,
    now: &str,
) -> Result<ClaimAcquireOutcome, ApiError> {
    if claim.issue_number != issue_number.0 {
        return Err(ApiError::Unexpected(format!(
            "claim targets issue #{} but was submitted for {issue_number}",
            claim.issue_number
        )));
    }
    let claims = fetch_claims(client, issue_number)?;
    if !matches!(
        classify_claim_resolution(&claims, &claim, issue_number, now),
        ClaimResolution::NoWinner
    ) {
        return resolve_claim_snapshot(client, issue_number, &claims, &claim, now);
    }
    if let Some(existing) = claims
        .iter()
        .find(|existing| claim_identity_matches(existing, &claim, issue_number))
    {
        let mut refreshed = claim;
        refreshed.comment_id = existing.comment_id;
        refreshed.status = ClaimStatus::Active;
        if let Some(comment_id) = existing.comment_id {
            client.patch_comment(comment_id, &render_claim_comment(&refreshed))?;
        }
        return resolve_claim_after_submission(client, issue_number, refreshed, now);
    }

    let created = client.create_comment(issue_number, &render_claim_comment(&claim))?;
    let mut own_claim = claim;
    own_claim.comment_id = Some(created.id);

    resolve_claim_after_submission(client, issue_number, own_claim, now)
}

/// Acquire a stable logical claim while preserving whether a mutation was
/// definitely not submitted or may have reached GitHub. An unknown outcome is
/// intentionally left to the durable executor for authoritative replay.
pub fn acquire_claim_mutation<C: IssueClient + ?Sized>(
    client: &C,
    issue_number: IssueNumber,
    claim: ClaimComment,
    now: &str,
) -> OwnerMutationResult<ClaimAcquireOutcome> {
    if claim.issue_number != issue_number.0 {
        return Err(OwnerMutationError::PreSubmit(ApiError::Unexpected(
            format!(
                "claim targets issue #{} but was submitted for {issue_number}",
                claim.issue_number
            ),
        )));
    }
    let claims = fetch_claims(client, issue_number).map_err(OwnerMutationError::PreSubmit)?;
    if !matches!(
        classify_claim_resolution(&claims, &claim, issue_number, now),
        ClaimResolution::NoWinner
    ) {
        return resolve_claim_snapshot_mutation(client, issue_number, &claims, &claim, now, false);
    }
    if let Some(existing) = claims
        .iter()
        .find(|existing| claim_identity_matches(existing, &claim, issue_number))
    {
        let mut refreshed = claim;
        refreshed.comment_id = existing.comment_id;
        refreshed.status = ClaimStatus::Active;
        if let Some(comment_id) = existing.comment_id {
            client.patch_comment_mutation(comment_id, &render_claim_comment(&refreshed))?;
        }
        return resolve_claim_after_mutation(client, issue_number, refreshed, now);
    }

    let created = client.create_comment_mutation(issue_number, &render_claim_comment(&claim))?;
    let mut own_claim = claim;
    own_claim.comment_id = Some(created.id);
    resolve_claim_after_mutation(client, issue_number, own_claim, now)
}

fn resolve_claim_after_mutation<C: IssueClient + ?Sized>(
    client: &C,
    issue_number: IssueNumber,
    own_claim: ClaimComment,
    now: &str,
) -> OwnerMutationResult<ClaimAcquireOutcome> {
    let claims =
        fetch_claims(client, issue_number).map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
    ensure_submitted_claim_read_back(&claims, &own_claim, issue_number)
        .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
    resolve_claim_snapshot_mutation(client, issue_number, &claims, &own_claim, now, true)
}

fn resolve_claim_after_submission<C: IssueClient + ?Sized>(
    client: &C,
    issue_number: IssueNumber,
    own_claim: ClaimComment,
    now: &str,
) -> Result<ClaimAcquireOutcome, ApiError> {
    let claims = fetch_claims(client, issue_number)?;
    ensure_submitted_claim_read_back(&claims, &own_claim, issue_number)?;
    resolve_claim_snapshot(client, issue_number, &claims, &own_claim, now)
}

fn ensure_submitted_claim_read_back(
    claims: &[ClaimComment],
    submitted: &ClaimComment,
    issue_number: IssueNumber,
) -> Result<(), ApiError> {
    let submitted_comment_id = submitted.comment_id.ok_or_else(|| {
        ApiError::Unexpected("submitted exact claim has no known comment id".to_string())
    })?;
    if claims.iter().any(|candidate| {
        candidate.comment_id == Some(submitted_comment_id)
            && claim_identity_matches(candidate, submitted, issue_number)
    }) {
        return Ok(());
    }
    Err(ApiError::Unexpected(
        "claim readback does not contain submitted exact claim comment".to_string(),
    ))
}

enum ClaimResolution {
    Acquired(ClaimComment),
    Blocked(ClaimComment),
    Lost(ClaimComment),
    NoWinner,
}

fn classify_claim_resolution(
    claims: &[ClaimComment],
    requested: &ClaimComment,
    issue_number: IssueNumber,
    now: &str,
) -> ClaimResolution {
    let active_own_exists = claims.iter().any(|existing| {
        claim_identity_matches(existing, requested, issue_number) && claim_is_active(existing, now)
    });
    if let Some(collision) = claims.iter().find(|existing| {
        existing.claim_id == requested.claim_id
            && !claim_identity_matches(existing, requested, issue_number)
    }) {
        return if active_own_exists {
            ClaimResolution::Lost(collision.clone())
        } else {
            ClaimResolution::Blocked(collision.clone())
        };
    }

    match select_winning_claim(claims, now) {
        Some(winner) if claim_identity_matches(winner, requested, issue_number) => {
            ClaimResolution::Acquired(winner.clone())
        }
        Some(winner) if active_own_exists => ClaimResolution::Lost(winner.clone()),
        Some(winner) => ClaimResolution::Blocked(winner.clone()),
        None => ClaimResolution::NoWinner,
    }
}

fn active_own_claims_except(
    claims: &[ClaimComment],
    requested: &ClaimComment,
    issue_number: IssueNumber,
    now: &str,
    keep_comment_id: Option<CommentId>,
) -> Vec<ClaimComment> {
    claims
        .iter()
        .filter(|existing| {
            claim_identity_matches(existing, requested, issue_number)
                && claim_is_active(existing, now)
                && existing.comment_id != keep_comment_id
        })
        .cloned()
        .collect()
}

fn terminalize_claims<C: IssueClient + ?Sized>(
    client: &C,
    claims: Vec<ClaimComment>,
    status: ClaimStatus,
) -> Result<Option<ClaimComment>, ApiError> {
    let mut terminalized = None;
    for mut claim in claims {
        let Some(comment_id) = claim.comment_id else {
            continue;
        };
        claim.status = status.clone();
        client.patch_comment(comment_id, &render_claim_comment(&claim))?;
        if terminalized.is_none() {
            terminalized = Some(claim);
        }
    }
    Ok(terminalized)
}

fn promote_mutation_error_after_submission(
    error: OwnerMutationError,
    submitted: bool,
) -> OwnerMutationError {
    if !submitted {
        return error;
    }
    match error {
        OwnerMutationError::PreSubmit(source) => OwnerMutationError::RemoteOutcomeUnknown(source),
        error => error,
    }
}

fn terminalize_claims_mutation<C: IssueClient + ?Sized>(
    client: &C,
    claims: Vec<ClaimComment>,
    status: ClaimStatus,
    prior_submission: bool,
) -> OwnerMutationResult<Option<ClaimComment>> {
    let mut submitted = prior_submission;
    let mut terminalized = None;
    for mut claim in claims {
        let Some(comment_id) = claim.comment_id else {
            continue;
        };
        claim.status = status.clone();
        if let Err(error) = client.patch_comment_mutation(comment_id, &render_claim_comment(&claim))
        {
            return Err(promote_mutation_error_after_submission(error, submitted));
        }
        submitted = true;
        if terminalized.is_none() {
            terminalized = Some(claim);
        }
    }
    Ok(terminalized)
}

fn resolve_claim_snapshot<C: IssueClient + ?Sized>(
    client: &C,
    issue_number: IssueNumber,
    claims: &[ClaimComment],
    requested: &ClaimComment,
    now: &str,
) -> Result<ClaimAcquireOutcome, ApiError> {
    match classify_claim_resolution(claims, requested, issue_number, now) {
        ClaimResolution::Acquired(winner) => {
            terminalize_claims(
                client,
                active_own_claims_except(claims, requested, issue_number, now, winner.comment_id),
                ClaimStatus::Lost,
            )?;
            Ok(ClaimAcquireOutcome::Acquired(winner))
        }
        ClaimResolution::Blocked(winner) => Ok(ClaimAcquireOutcome::Blocked(winner)),
        ClaimResolution::Lost(winning_claim) => {
            let own_claim = terminalize_claims(
                client,
                active_own_claims_except(claims, requested, issue_number, now, None),
                ClaimStatus::Lost,
            )?
            .ok_or_else(|| {
                ApiError::Unexpected("claim loss has no active own claim".to_string())
            })?;
            Ok(ClaimAcquireOutcome::Lost {
                own_claim,
                winning_claim,
            })
        }
        ClaimResolution::NoWinner => Err(ApiError::Unexpected(
            "claim readback has no active winner".to_string(),
        )),
    }
}

fn resolve_claim_snapshot_mutation<C: IssueClient + ?Sized>(
    client: &C,
    issue_number: IssueNumber,
    claims: &[ClaimComment],
    requested: &ClaimComment,
    now: &str,
    prior_submission: bool,
) -> OwnerMutationResult<ClaimAcquireOutcome> {
    match classify_claim_resolution(claims, requested, issue_number, now) {
        ClaimResolution::Acquired(winner) => {
            terminalize_claims_mutation(
                client,
                active_own_claims_except(claims, requested, issue_number, now, winner.comment_id),
                ClaimStatus::Lost,
                prior_submission,
            )?;
            Ok(ClaimAcquireOutcome::Acquired(winner))
        }
        ClaimResolution::Blocked(winner) => Ok(ClaimAcquireOutcome::Blocked(winner)),
        ClaimResolution::Lost(winning_claim) => {
            let own_claim = terminalize_claims_mutation(
                client,
                active_own_claims_except(claims, requested, issue_number, now, None),
                ClaimStatus::Lost,
                prior_submission,
            )?
            .ok_or_else(|| {
                let error = OwnerMutationError::PreSubmit(ApiError::Unexpected(
                    "claim loss has no active own claim".to_string(),
                ));
                promote_mutation_error_after_submission(error, prior_submission)
            })?;
            Ok(ClaimAcquireOutcome::Lost {
                own_claim,
                winning_claim,
            })
        }
        ClaimResolution::NoWinner => {
            let error = OwnerMutationError::PreSubmit(ApiError::Unexpected(
                "claim mutation readback has no active winner".to_string(),
            ));
            Err(promote_mutation_error_after_submission(
                error,
                prior_submission,
            ))
        }
    }
}

/// Release the claim identified by its exact Issue, logical id, and owner.
///
/// Replaying a release after a daemon restart is idempotent: an absent claim,
/// or a claim already in a terminal state, is treated as the target state and
/// does not issue another patch.
pub fn release_claim<C: IssueClient + ?Sized>(
    client: &C,
    issue_number: IssueNumber,
    claim_id: &str,
    owner: &str,
) -> Result<ClaimReleaseOutcome, ApiError> {
    if owner.trim().is_empty() {
        return Err(ApiError::Unexpected(
            "release claim owner identity is missing".to_string(),
        ));
    }
    let claims = fetch_claims(client, issue_number)?
        .into_iter()
        .filter(|claim| {
            claim.claim_id == claim_id
                && claim.owner == owner
                && claim.issue_number == issue_number.0
        })
        .collect::<Vec<_>>();
    let observed = claims.first().cloned();
    let released = terminalize_claims(
        client,
        claims
            .into_iter()
            .filter(|claim| claim.status == ClaimStatus::Active)
            .collect(),
        ClaimStatus::Released,
    )?;
    let Some(released) = released else {
        return Ok(ClaimReleaseOutcome::AlreadyReleased(observed));
    };
    Ok(ClaimReleaseOutcome::Released(released))
}

/// Mutation-aware release used by the durable side-effect executor.
pub fn release_claim_mutation<C: IssueClient + ?Sized>(
    client: &C,
    issue_number: IssueNumber,
    claim_id: &str,
    owner: &str,
) -> OwnerMutationResult<ClaimReleaseOutcome> {
    if owner.trim().is_empty() {
        return Err(OwnerMutationError::PreSubmit(ApiError::Unexpected(
            "release claim owner identity is missing".to_string(),
        )));
    }
    let claims = fetch_claims(client, issue_number)
        .map_err(OwnerMutationError::PreSubmit)?
        .into_iter()
        .filter(|claim| {
            claim.claim_id == claim_id
                && claim.owner == owner
                && claim.issue_number == issue_number.0
        })
        .collect::<Vec<_>>();
    let observed = claims.first().cloned();
    let released = terminalize_claims_mutation(
        client,
        claims
            .into_iter()
            .filter(|claim| claim.status == ClaimStatus::Active)
            .collect(),
        ClaimStatus::Released,
        false,
    )?;
    let Some(released) = released else {
        return Ok(ClaimReleaseOutcome::AlreadyReleased(observed));
    };
    Ok(ClaimReleaseOutcome::Released(released))
}

fn claim_identity_matches(
    candidate: &ClaimComment,
    requested: &ClaimComment,
    issue_number: IssueNumber,
) -> bool {
    candidate.claim_id == requested.claim_id
        && candidate.owner == requested.owner
        && candidate.issue_number == issue_number.0
        && requested.issue_number == issue_number.0
}

fn fetch_claims<C: IssueClient + ?Sized>(
    client: &C,
    issue_number: IssueNumber,
) -> Result<Vec<ClaimComment>, ApiError> {
    match client.fetch(issue_number, None)? {
        FetchResult::Updated(snapshot) => Ok(extract_claim_comments(&snapshot.comments)),
        FetchResult::NotModified => Ok(Vec::new()),
    }
}
