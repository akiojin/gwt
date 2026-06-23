//! Machine-readable GitHub Issue comment claims for the auto-improve monitor.

use serde::{Deserialize, Serialize};

use crate::{ApiError, CommentId, CommentSnapshot, FetchResult, IssueClient, IssueNumber};

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

pub fn extract_claim_comments(comments: &[CommentSnapshot]) -> Vec<ClaimComment> {
    comments
        .iter()
        .filter_map(|comment| parse_claim_comment(Some(comment.id), &comment.body).ok())
        .collect()
}

pub fn acquire_claim<C: IssueClient>(
    client: &C,
    issue_number: IssueNumber,
    claim: ClaimComment,
    now: &str,
) -> Result<ClaimAcquireOutcome, ApiError> {
    let claims = fetch_claims(client, issue_number)?;
    if let Some(winner) = select_winning_claim(&claims, now) {
        if winner.claim_id == claim.claim_id {
            return Ok(ClaimAcquireOutcome::Acquired(winner.clone()));
        }
        if winner.owner == claim.owner {
            if let Some(comment_id) = winner.comment_id {
                let mut refreshed = claim;
                refreshed.comment_id = Some(comment_id);
                client.patch_comment(comment_id, &render_claim_comment(&refreshed))?;
                return Ok(ClaimAcquireOutcome::Acquired(refreshed));
            }
        }
        return Ok(ClaimAcquireOutcome::Blocked(winner.clone()));
    }

    let created = client.create_comment(issue_number, &render_claim_comment(&claim))?;
    let mut own_claim = claim;
    own_claim.comment_id = Some(created.id);

    let claims = fetch_claims(client, issue_number)?;
    match select_winning_claim(&claims, now) {
        Some(winner) if winner.comment_id == own_claim.comment_id => {
            Ok(ClaimAcquireOutcome::Acquired(winner.clone()))
        }
        Some(winner) => {
            own_claim.status = ClaimStatus::Lost;
            let _ = client.patch_comment(created.id, &render_claim_comment(&own_claim));
            Ok(ClaimAcquireOutcome::Lost {
                own_claim,
                winning_claim: winner.clone(),
            })
        }
        None => Ok(ClaimAcquireOutcome::Acquired(own_claim)),
    }
}

fn fetch_claims<C: IssueClient>(
    client: &C,
    issue_number: IssueNumber,
) -> Result<Vec<ClaimComment>, ApiError> {
    match client.fetch(issue_number, None)? {
        FetchResult::Updated(snapshot) => Ok(extract_claim_comments(&snapshot.comments)),
        FetchResult::NotModified => Ok(Vec::new()),
    }
}
