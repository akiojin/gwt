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

    fn post_json(
        &self,
        url: &str,
        bearer: &str,
        body: &str,
    ) -> std::result::Result<HttpResponse, String> {
        let response = self
            .client
            .post(url)
            .bearer_auth(bearer)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
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

    fn patch_json(
        &self,
        url: &str,
        bearer: &str,
        body: &str,
    ) -> std::result::Result<HttpResponse, String> {
        let response = self
            .client
            .patch(url)
            .bearer_auth(bearer)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// One-shot loopback HTTP server: serves `responses` in order, one per
    /// accepted connection, then the thread exits. Parallel-safe (binds an
    /// ephemeral port; no shared state, no env).
    struct TestServer {
        base: String,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn start(responses: Vec<String>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let handle = thread::spawn(move || {
                for body in responses {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    // Small requests fit a single read; we only need to drain
                    // enough that the client's write completes before we reply.
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let _ = stream.write_all(body.as_bytes());
                    let _ = stream.flush();
                }
            });
            Self {
                base: format!("http://127.0.0.1:{port}"),
                handle: Some(handle),
            }
        }

        fn join(mut self) {
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn http_response(headers: &[&str], body: &str) -> String {
        let mut out = String::from("HTTP/1.1 200 OK\r\n");
        for header in headers {
            out.push_str(header);
            out.push_str("\r\n");
        }
        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
        out.push_str("Connection: close\r\n\r\n");
        out.push_str(body);
        out
    }

    /// A port that is guaranteed closed: bind then immediately drop the
    /// listener so a connect attempt is refused (covers the send-error paths).
    fn closed_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn reqwest_client_exercises_all_verbs() {
        let server = TestServer::start(vec![
            http_response(&["Retry-After: 30"], "GET_OK"),
            http_response(&[], "FORM_OK"),
            http_response(&[], "JSON_OK"),
            http_response(&[], "POSTER_OK"),
        ]);
        let base = server.base.clone();
        // Default::default() path.
        let client = ReqwestHttpClient::default();

        let get = HttpClient::get(&client, &base, "tok", &[("a", "b")]).unwrap();
        assert_eq!(get.status, 200);
        assert_eq!(get.body, "GET_OK");
        assert_eq!(get.retry_after, Some(30));

        let form = HttpClient::post_form(&client, &base, "tok", &[("k", "v")]).unwrap();
        assert_eq!(form.body, "FORM_OK");
        assert_eq!(form.retry_after, None);

        let json = client.post_json(&base, "tok", "{\"x\":1}").unwrap();
        assert_eq!(json.body, "JSON_OK");

        let poster = FormPoster::post_form(&client, &base, &[("k", "v")]).unwrap();
        assert_eq!(poster, "POSTER_OK");

        server.join();
    }

    #[test]
    fn send_failures_surface_as_errors() {
        let client = ReqwestHttpClient::new();
        let dead = format!("http://127.0.0.1:{}/x", closed_port());
        assert!(HttpClient::get(&client, &dead, "t", &[]).is_err());
        assert!(HttpClient::post_form(&client, &dead, "t", &[("k", "v")]).is_err());
        assert!(client.post_json(&dead, "t", "{}").is_err());
        assert!(FormPoster::post_form(&client, &dead, &[("k", "v")]).is_err());
    }
}
