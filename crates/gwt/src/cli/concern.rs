//! Machine-local tracking of user concerns (SPEC #4320).

use gwt_core::concern::{
    ConcernFilter, ConcernPatch, ConcernResolution, ConcernStore, MeasurementUpdate, NewConcern,
};
use gwt_github::{ApiError, SpecOpsError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{CliEnv, CliParseError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConcernCommand {
    Create(NewConcern),
    Update {
        id: String,
        patch: ConcernPatch,
    },
    List(ConcernFilter),
    Measure {
        id: String,
        update: MeasurementUpdate,
    },
    Resolve {
        id: String,
        state: ConcernResolution,
    },
}

pub(super) fn parse(
    operation: &str,
    params: &Map<String, Value>,
) -> Result<ConcernCommand, CliParseError> {
    match operation {
        "concern.create" => decode(operation, params.clone()).map(ConcernCommand::Create),
        "concern.list" => decode(operation, params.clone()).map(ConcernCommand::List),
        "concern.update" => {
            let (id, fields) = take_id(operation, params)?;
            Ok(ConcernCommand::Update {
                id,
                patch: decode(operation, fields)?,
            })
        }
        "concern.measure" => {
            let (id, fields) = take_id(operation, params)?;
            Ok(ConcernCommand::Measure {
                id,
                update: decode(operation, fields)?,
            })
        }
        "concern.resolve" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Resolution {
                state: ConcernResolution,
            }
            let (id, fields) = take_id(operation, params)?;
            let resolution: Resolution = decode(operation, fields)?;
            Ok(ConcernCommand::Resolve {
                id,
                state: resolution.state,
            })
        }
        _ => Err(CliParseError::UnknownSubcommand(operation.to_string())),
    }
}

fn decode<T: DeserializeOwned>(
    operation: &str,
    params: Map<String, Value>,
) -> Result<T, CliParseError> {
    serde_json::from_value(Value::Object(params))
        .map_err(|error| params_error(operation, &error.to_string()))
}

/// Issue #7259: name the rejected param without echoing caller-controlled
/// keys or values. Serde's `missing field` names and `expected` lists come
/// from the schema; an unknown key and any invalid value do not.
fn params_error(operation: &str, serde_message: &str) -> CliParseError {
    let detail = if let Some(rest) = serde_message.strip_prefix("missing field ") {
        let field = rest.split('`').nth(1).unwrap_or_default();
        format!("missing required param `{field}`")
    } else if serde_message.starts_with("unknown field ") {
        match serde_message.split_once(", expected ") {
            Some((_, expected)) => format!("unknown param; expected {expected}"),
            None => "unknown param".to_string(),
        }
    } else {
        "a param has an invalid type or value".to_string()
    };
    let hint = match operation {
        "concern.measure" => {
            "; required: id, cycle_id, measurement, owner_progress (one entry per owner Issue)"
        }
        "concern.resolve" => "; required: id, state",
        _ => "",
    };
    CliParseError::InvalidParams(format!("{operation}: {detail}{hint}"))
}

fn take_id(
    operation: &str,
    params: &Map<String, Value>,
) -> Result<(String, Map<String, Value>), CliParseError> {
    let mut fields = params.clone();
    match fields.remove("id") {
        Some(Value::String(id)) if !id.trim().is_empty() => Ok((id, fields)),
        _ => Err(CliParseError::InvalidParams(format!(
            "{operation}: param `id` must be a non-empty string"
        ))),
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: ConcernCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    gwt_core::paths::record_operation_project_store(env.repo_path());
    let store = ConcernStore::for_repo(env.repo_path());
    let response = match command {
        ConcernCommand::Create(input) => encode(store.create(input))?,
        ConcernCommand::Update { id, patch } => encode(
            store
                .update(&id, patch)
                .map(|concern| serde_json::json!({"concern":concern})),
        )?,
        ConcernCommand::List(filter) => encode(store.list(filter))?,
        ConcernCommand::Measure { id, update } => encode(
            store
                .measure(&id, update)
                .map(|concern| serde_json::json!({"concern":concern})),
        )?,
        ConcernCommand::Resolve { id, state } => encode(store.resolve(&id, state))?,
    };
    out.push_str(&response);
    out.push('\n');
    Ok(0)
}

fn encode<T: Serialize>(result: gwt_core::Result<T>) -> Result<String, SpecOpsError> {
    let value = result.map_err(|error| ApiError::Network(error.to_string()))?;
    serde_json::to_string(&value).map_err(|error| ApiError::Network(error.to_string()).into())
}
