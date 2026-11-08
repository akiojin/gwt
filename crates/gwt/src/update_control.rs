//! Explicit CLI updates hand their staged version to the owning GUI.

use std::{net::IpAddr, time::Duration};

use serde::{Deserialize, Serialize};

use crate::cli::tray::lock::TrayLockFile;

pub const UPDATE_APPLY_CONTROL_PATH: &str = "/internal/update/apply";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateApplyControlRequest {
    pub version: String,
}

/// A successful response acknowledges dispatch, not completion of the update.
pub fn request_apply(owner: &TrayLockFile, version: &str) -> Result<(), String> {
    let token = owner
        .control_token
        .as_deref()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            "the running GUI has no update control token; restart it first".to_string()
        })?;
    let base = reqwest::Url::parse(&owner.url)
        .map_err(|_| "the running GUI has not published a usable control URL".to_string())?;
    let loopback = base
        .host_str()
        .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
        .is_some_and(|address| address.is_loopback());
    if base.scheme() != "http"
        || !loopback
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err("the running GUI control URL must use a loopback HTTP address".to_string());
    }
    let endpoint = base
        .join(UPDATE_APPLY_CONTROL_PATH)
        .map_err(|_| "the running GUI control URL is invalid".to_string())?;
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "could not prepare the local update request".to_string())?;
    let response = client
        .post(endpoint)
        .bearer_auth(token)
        .json(&UpdateApplyControlRequest {
            version: version.to_string(),
        })
        .send()
        .map_err(|_| "the running GUI did not accept the local update request".to_string())?;
    if response.status() != reqwest::StatusCode::ACCEPTED {
        // A remote body or request error can reflect credentials. Report only
        // the status and let the CLI name the existing recovery actions.
        return Err(format!(
            "the running GUI refused the update request (HTTP {})",
            response.status().as_u16()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
    };

    #[test]
    fn update_control_refuses_remote_urls_redirects_and_incompatible_residents() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut owner = TrayLockFile {
            pid: std::process::id(),
            url: format!("http://{}/", listener.local_addr().unwrap()),
            started_at: chrono::Utc::now(),
            version: "9.122.0".to_string(),
            control_token: Some("private-control-token".to_string()),
        };
        let server = std::thread::spawn(move || {
            for status in [307, 401] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<u64>().unwrap();
                    }
                }
                std::io::copy(&mut reader.take(length), &mut std::io::sink()).unwrap();
                let body = "private-control-token";
                write!(stream, "HTTP/1.1 {status} Refused\r\nLocation: http://192.0.2.1/\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        for status in [307, 401] {
            let error = request_apply(&owner, "99.0.0").unwrap_err();
            assert!(error.contains(&format!("HTTP {status}")), "{error}");
            assert!(!error.contains("private-control-token"));
        }
        server.join().unwrap();
        for url in [
            "http://192.0.2.1/",
            "http://localhost/",
            "https://127.0.0.1/",
            "http://user@127.0.0.1/",
        ] {
            owner.url = url.to_string();
            assert!(request_apply(&owner, "99.0.0")
                .unwrap_err()
                .contains("loopback"));
        }
        owner.url = "http://[::1]:0/".to_string();
        assert!(!request_apply(&owner, "99.0.0")
            .unwrap_err()
            .contains("loopback"));
        owner.control_token = None;
        assert!(request_apply(&owner, "99.0.0")
            .unwrap_err()
            .contains("restart it"));
    }
}
