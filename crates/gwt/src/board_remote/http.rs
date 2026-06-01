//! Production HTTP client for remote Board providers (SPEC-2963 FR-014).
//!
//! Uses `reqwest::blocking` — the established HTTP client across this codebase
//! (gwt-core update, gwt-github, embedded_server). It runs requests on
//! reqwest's dedicated background runtime thread, so the synchronous
//! [`HttpClient`] / [`FormPoster`] traits are safe to call from both the sync
//! hook/CLI paths and the async GUI handlers.

use std::time::Duration;

use reqwest::blocking::Client;

use super::oauth::FormPoster;
use super::slack::{HttpClient, HttpResponse};

/// Blocking reqwest-backed implementation of the remote HTTP traits.
pub struct ReqwestHttpClient {
    client: Client,
}

impl ReqwestHttpClient {
    /// Build a client with a sensible request timeout.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client }
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

fn retry_after_seconds(response: &reqwest::blocking::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.trim().parse().ok())
}

impl HttpClient for ReqwestHttpClient {
    fn get(
        &self,
        url: &str,
        bearer: &str,
        query: &[(&str, &str)],
    ) -> std::result::Result<HttpResponse, String> {
        let response = self
            .client
            .get(url)
            .bearer_auth(bearer)
            .query(query)
            .send()
            .map_err(|err| err.to_string())?;
        let status = response.status().as_u16();
        let retry_after = retry_after_seconds(&response);
        let body = response.text().map_err(|err| err.to_string())?;
        Ok(HttpResponse {
            status,
            body,
            retry_after,
        })
    }

    fn post_form(
        &self,
        url: &str,
        bearer: &str,
        params: &[(&str, &str)],
    ) -> std::result::Result<HttpResponse, String> {
        let response = self
            .client
            .post(url)
            .bearer_auth(bearer)
            .form(params)
            .send()
            .map_err(|err| err.to_string())?;
        let status = response.status().as_u16();
        let retry_after = retry_after_seconds(&response);
        let body = response.text().map_err(|err| err.to_string())?;
        Ok(HttpResponse {
            status,
            body,
            retry_after,
        })
    }
}

impl FormPoster for ReqwestHttpClient {
    fn post_form(&self, url: &str, params: &[(&str, &str)]) -> std::result::Result<String, String> {
        let response = self
            .client
            .post(url)
            .form(params)
            .send()
            .map_err(|err| err.to_string())?;
        response.text().map_err(|err| err.to_string())
    }
}
