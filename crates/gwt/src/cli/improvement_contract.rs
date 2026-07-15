use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

pub const OWNER_PROJECTION_CONTRACT_REVISION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerProjectionSnapshot {
    pub contract_revision: u32,
    pub owners: Vec<OwnerProjectionAggregate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerProjectionAggregate {
    pub owner: OwnerProjectionOwner,
    pub fingerprint: String,
    pub aggregate_count: u64,
    pub last_seen: String,
    pub occurrences: Vec<OwnerProjectionOccurrence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerProjectionOwner {
    pub number: u64,
    pub kind: OwnerProjectionOwnerKind,
    pub active: bool,
    pub title: String,
    pub url: String,
    pub readback_verified_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum OwnerProjectionOwnerKind {
    Issue,
    Spec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerProjectionOccurrence {
    pub opaque_key: String,
    pub public_marker_digest: String,
    pub last_seen: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerProjectionReadError {
    message: String,
}

impl OwnerProjectionReadError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for OwnerProjectionReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for OwnerProjectionReadError {}

pub fn read_owner_projection() -> Result<OwnerProjectionSnapshot, OwnerProjectionReadError> {
    super::improvement_store::read_owner_projection_contract()
        .map_err(|error| OwnerProjectionReadError::new(error.to_string()))
}
