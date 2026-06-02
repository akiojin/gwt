//! Provider sign-in orchestration (SPEC-2963 Phase 5, FR-005/FR-012).
//!
//! Bridges settings + the OAuth session core: builds the provider OAuth config
//! (client id from settings; Slack client secret from the `GWT_SLACK_CLIENT_SECRET`
//! env var so it never lands in `config.toml`, FR-006), starts a sign-in, and
//! reports / clears auth state. The embedded-server `/oauth/callback` route
//! completes the flow against [`sessions`].

use std::sync::OnceLock;

use gwt_config::{BoardProviderKind, Settings, SlackConfig, TeamsConfig};

use super::oauth::OAuthConfig;
use super::oauth_session::{OAuthSessions, PendingAuth};
use super::token_store;

const SLACK_SCOPES: &[&str] = &["chat:write", "channels:history", "channels:read"];
const TEAMS_SCOPES: &[&str] = &[
    "offline_access",
    "ChannelMessage.Send",
    "ChannelMessage.Read.All",
    "Channel.ReadBasic.All",
];

/// Process-global pending OAuth session, shared by the sign-in initiator and
/// the embedded-server `/oauth/callback` route.
pub fn sessions() -> &'static OAuthSessions {
    static SESSIONS: OnceLock<OAuthSessions> = OnceLock::new();
    SESSIONS.get_or_init(OAuthSessions::default)
}

/// Resolve the Slack client secret: the `GWT_SLACK_CLIENT_SECRET` env var wins
/// (useful for CI / one-off overrides), otherwise the value the user saved from
/// the settings UI into the secure credential store (FR-006). Never read from
/// `config.toml`.
fn slack_client_secret() -> Option<String> {
    std::env::var("GWT_SLACK_CLIENT_SECRET")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| token_store::load_secret("slack").ok().flatten())
}

fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .clone()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Build the Slack OAuth config from settings + env secret + redirect URI.
pub fn slack_oauth_config(
    config: &SlackConfig,
    redirect_uri: &str,
) -> std::result::Result<OAuthConfig, String> {
    let client_id = non_empty(&config.client_id)
        .ok_or_else(|| "Slack client_id is not configured".to_string())?;
    Ok(OAuthConfig::slack(
        client_id,
        slack_client_secret(),
        redirect_uri,
        SLACK_SCOPES.iter().map(|s| s.to_string()).collect(),
    ))
}

/// Build the Teams (Microsoft identity) OAuth config from settings + redirect.
pub fn teams_oauth_config(
    config: &TeamsConfig,
    redirect_uri: &str,
) -> std::result::Result<OAuthConfig, String> {
    let client_id = non_empty(&config.client_id)
        .ok_or_else(|| "Teams client_id is not configured".to_string())?;
    let tenant = non_empty(&config.tenant_id).unwrap_or_else(|| "common".to_string());
    Ok(OAuthConfig::teams(
        client_id,
        &tenant,
        redirect_uri,
        TEAMS_SCOPES.iter().map(|s| s.to_string()).collect(),
    ))
}

/// Compose the OAuth redirect URI from the embedded server base.
pub fn redirect_uri(redirect_base: &str) -> String {
    format!("{}/oauth/callback", redirect_base.trim_end_matches('/'))
}

/// Begin a sign-in for `kind`. Returns the authorize URL to open in a browser
/// and records the pending state in [`sessions`].
pub fn begin_signin(
    kind: BoardProviderKind,
    settings: &Settings,
    redirect_base: &str,
) -> std::result::Result<String, String> {
    let redirect = redirect_uri(redirect_base);
    let state = uuid::Uuid::new_v4().to_string();
    // Slack uses a client secret; the Microsoft (Teams) public client uses PKCE.
    let (provider_key, config, pkce) = match kind {
        BoardProviderKind::Slack => (
            "slack".to_string(),
            slack_oauth_config(&settings.board.slack, &redirect)?,
            None,
        ),
        BoardProviderKind::Teams => (
            "teams".to_string(),
            teams_oauth_config(&settings.board.teams, &redirect)?,
            Some(super::oauth::generate_pkce()),
        ),
        BoardProviderKind::Local => {
            return Err("Local provider does not require sign-in".to_string())
        }
    };
    let pkce_verifier = pkce.as_ref().map(|(verifier, _)| verifier.clone());
    let pkce_challenge = pkce.map(|(_, challenge)| challenge);
    let pending = PendingAuth {
        provider_key,
        state,
        config,
        pkce_verifier,
    };
    sessions().begin(pending, pkce_challenge.as_deref())
}

/// Provider key (`"slack"` / `"teams"`) for a remote kind, if any.
pub fn provider_key(kind: BoardProviderKind) -> Option<&'static str> {
    match kind {
        BoardProviderKind::Slack => Some("slack"),
        BoardProviderKind::Teams => Some("teams"),
        BoardProviderKind::Local => None,
    }
}

/// Whether `provider_key` has a stored token.
pub fn is_signed_in(provider_key: &str) -> bool {
    token_store::load(provider_key).ok().flatten().is_some()
}

/// Remove the stored token for `provider_key`.
pub fn sign_out(provider_key: &str) -> std::io::Result<()> {
    token_store::clear(provider_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_config_requires_client_id() {
        assert!(
            slack_oauth_config(&SlackConfig::default(), "http://127.0.0.1:5/oauth/callback")
                .is_err()
        );
        let cfg = SlackConfig {
            client_id: Some("C-slack".to_string()),
            ..Default::default()
        };
        let oauth = slack_oauth_config(&cfg, "http://127.0.0.1:5/oauth/callback").unwrap();
        assert_eq!(oauth.client_id, "C-slack");
        assert_eq!(oauth.redirect_uri, "http://127.0.0.1:5/oauth/callback");
        assert!(oauth.authorize_endpoint.contains("slack.com"));
        assert!(oauth.scopes.contains(&"chat:write".to_string()));
    }

    #[test]
    fn teams_config_defaults_tenant_to_common() {
        let cfg = TeamsConfig {
            client_id: Some("T-app".to_string()),
            ..Default::default()
        };
        let oauth = teams_oauth_config(&cfg, "http://127.0.0.1:5/oauth/callback").unwrap();
        assert_eq!(oauth.client_id, "T-app");
        assert!(oauth.authorize_endpoint.contains("/common/"));
        assert!(oauth.scopes.iter().any(|s| s == "ChannelMessage.Send"));
    }

    #[test]
    fn redirect_uri_appends_callback_path() {
        assert_eq!(
            redirect_uri("http://127.0.0.1:8080/"),
            "http://127.0.0.1:8080/oauth/callback"
        );
        assert_eq!(
            redirect_uri("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080/oauth/callback"
        );
    }

    #[test]
    fn begin_signin_rejects_local() {
        let settings = Settings::default();
        assert!(begin_signin(BoardProviderKind::Local, &settings, "http://127.0.0.1:5").is_err());
    }

    #[test]
    fn provider_key_maps_remote_kinds() {
        assert_eq!(provider_key(BoardProviderKind::Slack), Some("slack"));
        assert_eq!(provider_key(BoardProviderKind::Teams), Some("teams"));
        assert_eq!(provider_key(BoardProviderKind::Local), None);
    }
}
