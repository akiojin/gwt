//! OAuth sign-in session management (SPEC-2963 Phase 5, FR-005/FR-012).
//!
//! Holds the single in-flight sign-in (authorize URL issued, awaiting the
//! browser redirect) and completes the callback: validate `state`, exchange the
//! authorization `code`, and persist the resulting token. The state handling
//! and exchange are unit-testable with a mock [`FormPoster`] and a temp
//! credentials directory; the axum `/oauth/callback` route and the browser
//! launch are thin glue layered on top.

use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use super::oauth::{self, FormPoster, OAuthConfig};
use super::token_store;

/// An issued-but-not-yet-completed sign-in.
pub struct PendingAuth {
    /// token_store key for the provider (`"slack"` / `"teams"`).
    pub provider_key: String,
    /// CSRF state echoed back on the callback.
    pub state: String,
    /// OAuth endpoints + client config used for the exchange.
    pub config: OAuthConfig,
    /// PKCE code verifier, when the provider uses PKCE (Teams).
    pub pkce_verifier: Option<String>,
}

/// Tracks the single in-flight OAuth sign-in for the process.
#[derive(Default)]
pub struct OAuthSessions {
    pending: Mutex<Option<PendingAuth>>,
}

impl OAuthSessions {
    /// Begin a sign-in: store the pending auth and return the authorize URL to
    /// open in the browser. PKCE S256 is only attached when a verifier is set
    /// (and its challenge is precomputed by the caller for now).
    pub fn begin(
        &self,
        pending: PendingAuth,
        pkce_challenge: Option<&str>,
    ) -> std::result::Result<String, String> {
        let url = oauth::build_authorize_url(&pending.config, &pending.state, pkce_challenge)?;
        let mut guard = self
            .pending
            .lock()
            .map_err(|_| "oauth session lock".to_string())?;
        *guard = Some(pending);
        Ok(url)
    }

    /// Consume the pending sign-in if `state` matches.
    pub fn take(&self, state: &str) -> Option<PendingAuth> {
        let mut guard = self.pending.lock().ok()?;
        let matches = guard
            .as_ref()
            .is_some_and(|pending| oauth::validate_state(&pending.state, state));
        if matches {
            guard.take()
        } else {
            None
        }
    }

    /// Whether a sign-in is currently in flight.
    pub fn is_pending(&self) -> bool {
        self.pending.lock().map(|g| g.is_some()).unwrap_or(false)
    }
}

/// Complete an OAuth callback end to end: validate state, exchange the code,
/// and persist the token under `credentials_dir`. Returns the provider key.
pub fn complete_callback(
    sessions: &OAuthSessions,
    code: &str,
    state: &str,
    poster: &dyn FormPoster,
    credentials_dir: &Path,
    now: DateTime<Utc>,
) -> std::result::Result<String, String> {
    let pending = sessions
        .take(state)
        .ok_or_else(|| "oauth callback: state mismatch or no pending sign-in".to_string())?;
    let token = oauth::exchange_code(
        &pending.config,
        code,
        pending.pkce_verifier.as_deref(),
        poster,
        now,
    )?;
    token_store::save_in(credentials_dir, &pending.provider_key, &token)
        .map_err(|err| format!("oauth callback: save token: {err}"))?;
    Ok(pending.provider_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board_remote::oauth::OAuthConfig;
    use chrono::TimeZone;
    use tempfile::tempdir;

    struct MockPoster {
        body: String,
    }
    impl FormPoster for MockPoster {
        fn post_form(
            &self,
            _url: &str,
            _params: &[(&str, &str)],
        ) -> std::result::Result<String, String> {
            Ok(self.body.clone())
        }
    }

    fn slack_pending(state: &str) -> PendingAuth {
        PendingAuth {
            provider_key: "slack".to_string(),
            state: state.to_string(),
            config: OAuthConfig::slack(
                "C1",
                Some("secret".into()),
                "http://127.0.0.1:5000/oauth/callback",
                vec!["chat:write".into()],
            ),
            pkce_verifier: None,
        }
    }

    #[test]
    fn begin_returns_authorize_url_and_marks_pending() {
        let sessions = OAuthSessions::default();
        let url = sessions.begin(slack_pending("state-1"), None).unwrap();
        assert!(url.contains("state=state-1"));
        assert!(url.contains("client_id=C1"));
        assert!(sessions.is_pending());
    }

    #[test]
    fn take_requires_matching_state() {
        let sessions = OAuthSessions::default();
        sessions.begin(slack_pending("state-1"), None).unwrap();
        assert!(sessions.take("wrong").is_none());
        // still pending after a mismatch.
        assert!(sessions.is_pending());
        assert!(sessions.take("state-1").is_some());
        // consumed.
        assert!(!sessions.is_pending());
    }

    #[test]
    fn complete_callback_exchanges_and_persists_token() {
        let sessions = OAuthSessions::default();
        sessions.begin(slack_pending("state-1"), None).unwrap();
        let poster = MockPoster {
            body: r#"{"ok":true,"access_token":"xoxb-final","expires_in":3600}"#.to_string(),
        };
        let dir = tempdir().unwrap();
        let now = Utc.timestamp_opt(1_000, 0).unwrap();
        let key =
            complete_callback(&sessions, "code-1", "state-1", &poster, dir.path(), now).unwrap();
        assert_eq!(key, "slack");
        let saved = token_store::load_in(dir.path(), "slack").unwrap().unwrap();
        assert_eq!(saved.access_token, "xoxb-final");
        assert!(!sessions.is_pending());
    }

    #[test]
    fn complete_callback_rejects_state_mismatch() {
        let sessions = OAuthSessions::default();
        sessions.begin(slack_pending("state-1"), None).unwrap();
        let poster = MockPoster {
            body: "{}".to_string(),
        };
        let dir = tempdir().unwrap();
        let now = Utc.timestamp_opt(1_000, 0).unwrap();
        let err = complete_callback(&sessions, "code", "bad-state", &poster, dir.path(), now)
            .unwrap_err();
        assert!(err.contains("state mismatch"));
    }
}
