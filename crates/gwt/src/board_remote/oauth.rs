//! OAuth authorization-code flow for remote Board providers (SPEC-2963 FR-005).
//!
//! The HTTP token exchange is abstracted behind [`FormPoster`] so the URL
//! building, state handling, and response parsing are unit-testable without
//! real network calls. The production poster (reqwest blocking) is wired in a
//! later phase alongside the embedded-server `/oauth/callback` route.

use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::board_remote::token_store::TokenSet;

/// Compute the PKCE S256 code challenge for a verifier:
/// `base64url-nopad(sha256(verifier))` (RFC 7636).
pub fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Generate a PKCE `(verifier, S256 challenge)` pair. The verifier is 64
/// unreserved characters (two UUIDs), within RFC 7636's 43–128 length range.
pub fn generate_pkce() -> (String, String) {
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = pkce_challenge(&verifier);
    (verifier, challenge)
}

/// Which remote provider an OAuth flow targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthProvider {
    Slack,
    Teams,
}

/// Static endpoints + client config for one OAuth flow.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub provider: OAuthProvider,
    pub client_id: String,
    /// Slack requires a client secret for token exchange. The Microsoft public
    /// (desktop) client uses PKCE and leaves this `None`.
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub authorize_endpoint: String,
    pub token_endpoint: String,
}

impl OAuthConfig {
    /// Slack OAuth v2 endpoints.
    pub fn slack(
        client_id: impl Into<String>,
        client_secret: Option<String>,
        redirect_uri: impl Into<String>,
        scopes: Vec<String>,
    ) -> Self {
        Self {
            provider: OAuthProvider::Slack,
            client_id: client_id.into(),
            client_secret,
            redirect_uri: redirect_uri.into(),
            scopes,
            authorize_endpoint: "https://slack.com/oauth/v2/authorize".to_string(),
            token_endpoint: "https://slack.com/api/oauth.v2.access".to_string(),
        }
    }

    /// Microsoft identity platform endpoints for a tenant (`common`,
    /// `organizations`, or a tenant id).
    pub fn teams(
        client_id: impl Into<String>,
        tenant: &str,
        redirect_uri: impl Into<String>,
        scopes: Vec<String>,
    ) -> Self {
        let tenant = if tenant.trim().is_empty() {
            "common"
        } else {
            tenant.trim()
        };
        Self {
            provider: OAuthProvider::Teams,
            client_id: client_id.into(),
            client_secret: None,
            redirect_uri: redirect_uri.into(),
            scopes,
            authorize_endpoint: format!(
                "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"
            ),
            token_endpoint: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
        }
    }
}

/// Build the authorization URL the user opens in the browser. `state` guards
/// against CSRF; `pkce_challenge` (S256) is included when present.
pub fn build_authorize_url(
    config: &OAuthConfig,
    state: &str,
    pkce_challenge: Option<&str>,
) -> Result<String, String> {
    let scope = config.scopes.join(" ");
    let mut params: Vec<(&str, &str)> = vec![
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("response_type", "code"),
        ("state", state),
    ];
    // Slack uses `scope`; both accept `scope`. Microsoft additionally honors
    // `response_mode=query` which is the default for code flow.
    params.push(("scope", scope.as_str()));
    if let Some(challenge) = pkce_challenge {
        params.push(("code_challenge", challenge));
        params.push(("code_challenge_method", "S256"));
    }
    reqwest::Url::parse_with_params(&config.authorize_endpoint, &params)
        .map(|url| url.to_string())
        .map_err(|err| format!("invalid authorize endpoint: {err}"))
}

/// Validate the `state` returned on the callback against the one we issued.
pub fn validate_state(expected: &str, received: &str) -> bool {
    !expected.is_empty() && expected == received
}

/// HTTP form-POST abstraction so the token exchange is unit-testable. The
/// production implementation uses a blocking reqwest client.
pub trait FormPoster {
    /// POST `params` as `application/x-www-form-urlencoded` to `url` and return
    /// the response body on success.
    fn post_form(&self, url: &str, params: &[(&str, &str)]) -> Result<String, String>;
}

#[derive(Deserialize)]
struct SlackTokenResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Deserialize)]
struct OidcTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

fn expires_at(now: DateTime<Utc>, expires_in: Option<i64>) -> Option<DateTime<Utc>> {
    expires_in.map(|secs| now + Duration::seconds(secs))
}

fn parse_token_response(
    provider: OAuthProvider,
    body: &str,
    now: DateTime<Utc>,
) -> Result<TokenSet, String> {
    match provider {
        OAuthProvider::Slack => {
            let parsed: SlackTokenResponse =
                serde_json::from_str(body).map_err(|err| format!("slack token parse: {err}"))?;
            if !parsed.ok {
                return Err(format!(
                    "slack oauth error: {}",
                    parsed.error.unwrap_or_else(|| "unknown".to_string())
                ));
            }
            let access_token = parsed
                .access_token
                .ok_or_else(|| "slack oauth: missing access_token".to_string())?;
            Ok(TokenSet {
                access_token,
                refresh_token: parsed.refresh_token,
                expires_at: expires_at(now, parsed.expires_in),
            })
        }
        OAuthProvider::Teams => {
            let parsed: OidcTokenResponse =
                serde_json::from_str(body).map_err(|err| format!("ms token parse: {err}"))?;
            Ok(TokenSet {
                access_token: parsed.access_token,
                refresh_token: parsed.refresh_token,
                expires_at: expires_at(now, parsed.expires_in),
            })
        }
    }
}

/// Exchange an authorization `code` for a [`TokenSet`].
pub fn exchange_code(
    config: &OAuthConfig,
    code: &str,
    pkce_verifier: Option<&str>,
    poster: &dyn FormPoster,
    now: DateTime<Utc>,
) -> Result<TokenSet, String> {
    let mut params: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("client_id", config.client_id.as_str()),
    ];
    if let Some(secret) = config.client_secret.as_deref() {
        params.push(("client_secret", secret));
    }
    if let Some(verifier) = pkce_verifier {
        params.push(("code_verifier", verifier));
    }
    let body = poster.post_form(&config.token_endpoint, &params)?;
    parse_token_response(config.provider, &body, now)
}

/// Mint a fresh [`TokenSet`] from a stored `refresh_token`.
pub fn refresh(
    config: &OAuthConfig,
    refresh_token: &str,
    poster: &dyn FormPoster,
    now: DateTime<Utc>,
) -> Result<TokenSet, String> {
    let mut params: Vec<(&str, &str)> = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", config.client_id.as_str()),
    ];
    if let Some(secret) = config.client_secret.as_deref() {
        params.push(("client_secret", secret));
    }
    let body = poster.post_form(&config.token_endpoint, &params)?;
    let mut tokens = parse_token_response(config.provider, &body, now)?;
    // Some providers omit the refresh_token on refresh; keep the existing one.
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = Some(refresh_token.to_string());
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    struct MockPoster {
        body: String,
        expect_url: Option<String>,
    }

    impl FormPoster for MockPoster {
        fn post_form(&self, url: &str, _params: &[(&str, &str)]) -> Result<String, String> {
            if let Some(expected) = &self.expect_url {
                assert_eq!(url, expected);
            }
            Ok(self.body.clone())
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_000_000, 0).unwrap()
    }

    #[test]
    fn authorize_url_includes_required_params() {
        let cfg = OAuthConfig::slack(
            "C1",
            Some("secret".into()),
            "http://127.0.0.1:5000/oauth/callback",
            vec!["chat:write".into(), "channels:history".into()],
        );
        let url = build_authorize_url(&cfg, "state-xyz", None).unwrap();
        assert!(url.starts_with("https://slack.com/oauth/v2/authorize?"));
        assert!(url.contains("client_id=C1"));
        assert!(url.contains("state=state-xyz"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("chat%3Awrite")); // url-encoded scope
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5000%2Foauth%2Fcallback"));
    }

    #[test]
    fn authorize_url_includes_pkce_when_present() {
        let cfg = OAuthConfig::teams(
            "T1",
            "common",
            "http://127.0.0.1:5000/oauth/callback",
            vec!["ChannelMessage.Send".into()],
        );
        let url = build_authorize_url(&cfg, "s", Some("challenge123")).unwrap();
        assert!(url.contains("code_challenge=challenge123"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("login.microsoftonline.com/common/oauth2/v2.0/authorize"));
    }

    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        // sha256("abc") base64url-nopad — a stable known value.
        assert_eq!(
            pkce_challenge("abc"),
            "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"
        );
    }

    #[test]
    fn generate_pkce_is_unreserved_and_unique() {
        let (verifier, challenge) = generate_pkce();
        assert!(verifier.len() >= 43 && verifier.len() <= 128);
        assert!(verifier.chars().all(|c| c.is_ascii_alphanumeric()));
        // S256 challenge is url-safe base64 without padding.
        assert!(!challenge.contains('+') && !challenge.contains('/') && !challenge.contains('='));
        assert_eq!(challenge, pkce_challenge(&verifier));
        let (other, _) = generate_pkce();
        assert_ne!(verifier, other);
    }

    #[test]
    fn state_validation_rejects_mismatch_and_empty() {
        assert!(validate_state("abc", "abc"));
        assert!(!validate_state("abc", "xyz"));
        assert!(!validate_state("", ""));
    }

    #[test]
    fn slack_code_exchange_parses_token_and_expiry() {
        let cfg = OAuthConfig::slack("C1", Some("s".into()), "http://localhost/cb", vec![]);
        let poster = MockPoster {
            body: r#"{"ok":true,"access_token":"xoxb-abc","refresh_token":"xoxr-1","expires_in":3600}"#
                .to_string(),
            expect_url: Some("https://slack.com/api/oauth.v2.access".to_string()),
        };
        let token = exchange_code(&cfg, "code-1", None, &poster, now()).unwrap();
        assert_eq!(token.access_token, "xoxb-abc");
        assert_eq!(token.refresh_token.as_deref(), Some("xoxr-1"));
        assert_eq!(token.expires_at, Some(now() + Duration::seconds(3600)));
    }

    #[test]
    fn slack_oauth_error_is_surfaced() {
        let cfg = OAuthConfig::slack("C1", Some("s".into()), "http://localhost/cb", vec![]);
        let poster = MockPoster {
            body: r#"{"ok":false,"error":"invalid_code"}"#.to_string(),
            expect_url: None,
        };
        let err = exchange_code(&cfg, "bad", None, &poster, now()).unwrap_err();
        assert!(err.contains("invalid_code"));
    }

    #[test]
    fn teams_code_exchange_parses_oidc_token() {
        let cfg = OAuthConfig::teams("T1", "common", "http://localhost/cb", vec![]);
        let poster = MockPoster {
            body: r#"{"access_token":"ms-access","refresh_token":"ms-refresh","expires_in":3599}"#
                .to_string(),
            expect_url: None,
        };
        let token = exchange_code(&cfg, "code", Some("verifier"), &poster, now()).unwrap();
        assert_eq!(token.access_token, "ms-access");
        assert_eq!(token.refresh_token.as_deref(), Some("ms-refresh"));
    }

    #[test]
    fn refresh_keeps_existing_refresh_token_when_omitted() {
        let cfg = OAuthConfig::slack("C1", Some("s".into()), "http://localhost/cb", vec![]);
        let poster = MockPoster {
            body: r#"{"ok":true,"access_token":"new-access","expires_in":3600}"#.to_string(),
            expect_url: None,
        };
        let token = refresh(&cfg, "kept-refresh", &poster, now()).unwrap();
        assert_eq!(token.access_token, "new-access");
        assert_eq!(token.refresh_token.as_deref(), Some("kept-refresh"));
    }
}
