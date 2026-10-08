//! Issue #4538 AC-4 / AC-5: wire contract of the local project-open control
//! request that `gwt open <path>` sends to the running tray-resident process.
//!
//! The request is authorized by a per-process control token that only the
//! OS user can read (the tray lock file is written `0600`). Validation is pure
//! so the embedded server and its tests share exactly one definition of the
//! accepted request shape and of every rejection status.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Route served by the embedded browser server.
pub const PROJECT_OPEN_CONTROL_PATH: &str = "/internal/projects/open";

/// Upper bound on the JSON body. A path never needs more.
pub const PROJECT_OPEN_CONTROL_MAX_BODY_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectOpenControlRequest {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectOpenControlResponse {
    pub project_key: String,
    /// Path-only per-project URL (`/p/<project_key>`), resolved against the
    /// server URL the caller already knows.
    pub url_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectOpenControlErrorBody {
    pub error: String,
}

/// Per-project URL path for a canonical ProjectKey.
pub fn project_url_path(project_key: &str) -> String {
    format!("/p/{project_key}")
}

/// Why a control request was refused before or after reaching the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectOpenControlRejection {
    Unauthorized,
    BadRequest(String),
    PayloadTooLarge,
    Unprocessable(String),
    Unavailable(String),
    Timeout,
}

impl ProjectOpenControlRejection {
    pub fn status(&self) -> u16 {
        match self {
            Self::Unauthorized => 401,
            Self::BadRequest(_) => 400,
            Self::PayloadTooLarge => 413,
            Self::Unprocessable(_) => 422,
            Self::Unavailable(_) => 503,
            Self::Timeout => 504,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Unauthorized => "missing or invalid control token".to_string(),
            Self::BadRequest(message)
            | Self::Unprocessable(message)
            | Self::Unavailable(message) => message.clone(),
            Self::PayloadTooLarge => {
                format!("request body exceeds {PROJECT_OPEN_CONTROL_MAX_BODY_BYTES} bytes")
            }
            Self::Timeout => "timed out waiting for the project to open".to_string(),
        }
    }
}

/// Accept only `Authorization: Bearer <token>` matching the process token.
/// A process that published no token accepts nothing.
pub fn authorize_control_request(
    authorization: Option<&str>,
    expected_token: Option<&str>,
) -> Result<(), ProjectOpenControlRejection> {
    let Some(expected) = expected_token.filter(|token| !token.is_empty()) else {
        return Err(ProjectOpenControlRejection::Unauthorized);
    };
    let presented = authorization
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(ProjectOpenControlRejection::Unauthorized)
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// Validate the JSON body and return the absolute path to open.
pub fn parse_control_request(
    content_type: Option<&str>,
    body: &[u8],
) -> Result<PathBuf, ProjectOpenControlRejection> {
    if body.len() > PROJECT_OPEN_CONTROL_MAX_BODY_BYTES {
        return Err(ProjectOpenControlRejection::PayloadTooLarge);
    }
    let is_json = content_type
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"));
    if !is_json {
        return Err(ProjectOpenControlRejection::BadRequest(
            "content type must be application/json".to_string(),
        ));
    }
    let text = std::str::from_utf8(body).map_err(|_| {
        ProjectOpenControlRejection::BadRequest("request body is not UTF-8".to_string())
    })?;
    let request: ProjectOpenControlRequest = serde_json::from_str(text).map_err(|error| {
        ProjectOpenControlRejection::BadRequest(format!("invalid request: {error}"))
    })?;
    if request.path.trim().is_empty() {
        return Err(ProjectOpenControlRejection::Unprocessable(
            "path must not be empty".to_string(),
        ));
    }
    let path = PathBuf::from(&request.path);
    if !Path::new(&request.path).is_absolute() {
        return Err(ProjectOpenControlRejection::Unprocessable(
            "path must be absolute".to_string(),
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: Option<&str> = Some("application/json");

    fn absolute_path() -> String {
        std::env::temp_dir().join("gwt-open").display().to_string()
    }

    #[test]
    fn authorization_requires_the_exact_bearer_token() {
        assert_eq!(
            authorize_control_request(Some("Bearer secret"), Some("secret")),
            Ok(())
        );
        for presented in [
            None,
            Some("secret"),
            Some("Bearer other"),
            Some("bearer secret"),
        ] {
            assert_eq!(
                authorize_control_request(presented, Some("secret")),
                Err(ProjectOpenControlRejection::Unauthorized),
                "{presented:?}"
            );
        }
        assert_eq!(
            authorize_control_request(Some("Bearer "), None),
            Err(ProjectOpenControlRejection::Unauthorized),
            "a process without a token accepts nothing"
        );
    }

    #[test]
    fn request_body_accepts_one_absolute_path() {
        let path = absolute_path();
        let body = serde_json::to_vec(&ProjectOpenControlRequest { path: path.clone() }).unwrap();
        assert_eq!(
            parse_control_request(Some("application/json; charset=utf-8"), &body),
            Ok(PathBuf::from(path))
        );
    }

    #[test]
    fn request_body_rejections_map_to_their_statuses() {
        let path = absolute_path();
        let valid = serde_json::to_vec(&ProjectOpenControlRequest { path }).unwrap();
        let cases: Vec<(Option<&str>, Vec<u8>, u16)> = vec![
            (Some("text/plain"), valid.clone(), 400),
            (None, valid, 400),
            (JSON, vec![b'{', 0xff, b'}'], 400),
            (JSON, b"not json".to_vec(), 400),
            (JSON, br#"{"path":1}"#.to_vec(), 400),
            (JSON, br#"{"path":"/x","extra":true}"#.to_vec(), 400),
            (JSON, br#"{}"#.to_vec(), 400),
            (
                JSON,
                vec![b' '; PROJECT_OPEN_CONTROL_MAX_BODY_BYTES + 1],
                413,
            ),
            (JSON, br#"{"path":"  "}"#.to_vec(), 422),
            (JSON, br#"{"path":"relative/project"}"#.to_vec(), 422),
        ];
        for (content_type, body, status) in cases {
            let rejection = parse_control_request(content_type, &body).expect_err("rejected");
            assert_eq!(
                rejection.status(),
                status,
                "{content_type:?} {:?}",
                String::from_utf8_lossy(&body)
            );
        }
    }

    #[test]
    fn runtime_outcomes_map_to_their_statuses() {
        assert_eq!(
            ProjectOpenControlRejection::Unavailable("busy".into()).status(),
            503
        );
        assert_eq!(ProjectOpenControlRejection::Timeout.status(), 504);
        assert_eq!(project_url_path("0123456789abcdef"), "/p/0123456789abcdef");
    }
}
