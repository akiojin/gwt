//! Bounded, credential-silent access to Codex's free reset-credit API.
use std::{path::Path, process::Stdio, time::Duration};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    runtime::Runtime,
};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, PartialEq)]
pub enum ResetOutcome {
    Reset,
    AlreadyRedeemed,
    NothingToReset,
    NoCredit,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCredit {
    pub id: String,
    pub reset_type: String,
    pub status: String,
}
#[derive(Debug)]
pub struct ResetSnapshot {
    pub account_id: String,
    pub available_count: u64,
    pub credits: Vec<ResetCredit>,
    pub ordinary_usage_allowed: Option<bool>,
}

fn parse_snapshot(value: Value) -> Result<ResetSnapshot, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Summary {
        available_count: u64,
        credits: Option<Vec<ResetCredit>>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Snapshot {
        account_id: String,
        rate_limit_reset_credits: Summary,
        ordinary_usage_allowed: Option<bool>,
    }
    let data: Snapshot = serde_json::from_value(value)
        .map_err(|_| "Codex reset snapshot is missing required data")?;
    if data.account_id.trim().is_empty() {
        return Err("Codex account identity is unavailable".into());
    }
    let credits = data
        .rate_limit_reset_credits
        .credits
        .unwrap_or_default()
        .into_iter()
        .filter(|c| {
            !c.id.trim().is_empty() && c.reset_type == "codexRateLimits" && c.status == "available"
        })
        .collect();
    Ok(ResetSnapshot {
        account_id: data.account_id,
        available_count: data.rate_limit_reset_credits.available_count,
        credits,
        ordinary_usage_allowed: data.ordinary_usage_allowed,
    })
}

fn parse_outcome(value: Value) -> Result<ResetOutcome, String> {
    match value.get("outcome").and_then(Value::as_str) {
        Some("reset") => Ok(ResetOutcome::Reset),
        Some("alreadyRedeemed") => Ok(ResetOutcome::AlreadyRedeemed),
        Some("nothingToReset") => Ok(ResetOutcome::NothingToReset),
        Some("noCredit") => Ok(ResetOutcome::NoCredit),
        _ => Err("Codex returned an unknown reset outcome; usage remains held".into()),
    }
}

async fn send(writer: &mut (impl AsyncWrite + Unpin), message: Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&message).map_err(|_| "Codex request encoding failed")?;
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .await
        .map_err(|_| "Codex request write failed")?;
    writer
        .flush()
        .await
        .map_err(|_| "Codex request flush failed".into())
}

async fn request(
    reader: &mut (impl AsyncBufRead + Unpin),
    writer: &mut (impl AsyncWrite + Unpin),
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    use tokio::io::AsyncReadExt;
    send(writer, json!({"id":id,"method":method,"params":params})).await?;
    loop {
        let mut line = Vec::new();
        let count = reader
            .take(MAX_RESPONSE_BYTES + 1)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| "Codex response read failed")?;
        if count == 0 {
            return Err("Codex app-server closed the connection".into());
        }
        if count as u64 > MAX_RESPONSE_BYTES {
            return Err("Codex response exceeded size limit".into());
        }
        let value: Value =
            serde_json::from_slice(&line).map_err(|_| "Codex returned malformed JSON")?;
        if value.get("id").is_none() && value.get("method").and_then(Value::as_str).is_some() {
            continue;
        }
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            return Err("Codex response identity mismatch".into());
        }
        if value.get("error").is_some() {
            return Err("Codex rejected the reset API request".into());
        }
        return value
            .get("result")
            .filter(|r| r.is_object())
            .cloned()
            .ok_or_else(|| "Codex response has no result".into());
    }
}

/// Use the proven target authentication root, never the calling agent's credentials.
fn app_server_request(
    executable: &Path,
    cwd: &Path,
    home: &Path,
    auth_root: &Path,
) -> gwt_core::process::ProcessPlanRequest {
    let mut request = gwt_core::process::ProcessPlanRequest::new(executable).inherit_env(false);
    // These contain process/runtime setup only; provider/API variables are excluded.
    for key in [
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
    ] {
        if let Some(value) = std::env::var_os(key) {
            request = request.env(key, value);
        }
    }
    request
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CODEX_HOME", auth_root)
        .args(["app-server", "--stdio"])
        .current_dir(cwd)
}

/// Use on a blocking worker, outside an async runtime. Drop terminates the helper.
pub struct CodexResetClient {
    child: Child,
    reader: BufReader<ChildStdout>,
    writer: ChildStdin,
    runtime: Runtime,
    next_id: u64,
    available_ids: Vec<String>,
}

impl CodexResetClient {
    pub fn start(
        executable: &Path,
        cwd: &Path,
        home: &Path,
        auth_root: &Path,
    ) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "Could not initialize Codex API runtime")?;
        let (child, reader, writer) = runtime.block_on(async {
            let mut command = gwt_core::process::resolved_tokio_command(app_server_request(
                executable, cwd, home, auth_root,
            ))
            .map_err(|_| "Could not resolve the installed Codex executable")?;
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| "Could not start Codex app-server")?;
            let writer = child.stdin.take().ok_or("Codex stdin unavailable")?;
            let reader = BufReader::new(child.stdout.take().ok_or("Codex stdout unavailable")?);
            Ok::<_, String>((child, reader, writer))
        })?;
        let mut client = Self {
            child,
            reader,
            writer,
            runtime,
            next_id: 1,
            available_ids: Vec::new(),
        };
        client.call("initialize", json!({"clientInfo":{"name":"gwt","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}))?;
        let result = client.runtime.block_on(async {
            tokio::time::timeout(
                RESPONSE_TIMEOUT,
                send(&mut client.writer, json!({"method":"initialized"})),
            )
            .await
        });
        match result {
            Ok(result) => result?,
            Err(_) => return Err("Codex initialization timed out".into()),
        }
        Ok(client)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let result = self.runtime.block_on(async {
            tokio::time::timeout(
                RESPONSE_TIMEOUT,
                request(&mut self.reader, &mut self.writer, id, method, params),
            )
            .await
        });
        match result {
            Ok(Ok(value)) => Ok(value),
            result => {
                let _ = self.child.start_kill();
                self.available_ids.clear();
                match result {
                    Ok(Err(error)) => Err(error),
                    _ => {
                        Err("Codex API timed out; outcome is unknown and usage remains held".into())
                    }
                }
            }
        }
    }

    pub fn read(&mut self) -> Result<ResetSnapshot, String> {
        self.available_ids.clear();
        let value = self.call("account/rateLimits/read", Value::Null)?;
        let snapshot = parse_snapshot(value)?;
        if snapshot.available_count > 0 {
            self.available_ids = snapshot.credits.iter().map(|c| c.id.clone()).collect();
        }
        Ok(snapshot)
    }

    pub fn consume(
        &mut self,
        idempotency_key: &str,
        credit_id: &str,
    ) -> Result<ResetOutcome, String> {
        if idempotency_key.trim().is_empty() || !self.available_ids.iter().any(|id| id == credit_id)
        {
            return Err(
                "An observed available free Codex credit and idempotency key are required".into(),
            );
        }
        let value = self.call(
            "account/rateLimitResetCredit/consume",
            json!({"idempotencyKey":idempotency_key,"creditId":credit_id}),
        )?;
        self.available_ids.clear();
        parse_outcome(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn request_preserves_key_and_credit_and_ignores_notifications() {
        let (client, server) = tokio::io::duplex(4096);
        let (client_read, mut client_write) = tokio::io::split(client);
        let mut client_read = BufReader::new(client_read);
        let (server_read, mut server_write) = tokio::io::split(server);
        let peer = async {
            let mut reader = BufReader::new(server_read);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(
                request,
                json!({"id":7,"method":"account/rateLimitResetCredit/consume", "params":{"creditId":"free","idempotencyKey":"attempt"}})
            );
            send(
                &mut server_write,
                json!({"method":"account/rateLimits/updated","params":{}}),
            )
            .await
            .unwrap();
            send(
                &mut server_write,
                json!({"id":7,"result":{"outcome":"reset"}}),
            )
            .await
            .unwrap();
        };
        let call = request(
            &mut client_read,
            &mut client_write,
            7,
            "account/rateLimitResetCredit/consume",
            json!({"creditId":"free","idempotencyKey":"attempt"}),
        );
        let (result, ()) = tokio::join!(call, peer);
        assert_eq!(parse_outcome(result.unwrap()).unwrap(), ResetOutcome::Reset);
    }

    #[tokio::test]
    async fn malformed_or_mismatched_response_never_becomes_success() {
        for response in [
            "not json\n",
            "{\"id\":2,\"result\":{}}\n",
            "{\"id\":1,\"error\":{\"message\":\"secret\"}}\n",
            "",
        ] {
            let mut reader = BufReader::new(response.as_bytes());
            let mut writer = tokio::io::sink();
            let error = request(
                &mut reader,
                &mut writer,
                1,
                "account/rateLimits/read",
                Value::Null,
            )
            .await
            .unwrap_err();
            assert!(!error.contains("secret"));
        }
    }

    #[test]
    fn reset_helper_uses_proven_target_root_and_does_not_inherit_provider_credentials() {
        let home = std::env::temp_dir().join("gwt-reset-test-home");
        let request = app_server_request(Path::new("/installed/codex"), &home, &home, &home);
        assert!(!request.inherit_env);
        let environment = request
            .env
            .iter()
            .map(|(key, value)| (key.as_os_str(), value.as_os_str()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            environment.get(std::ffi::OsStr::new("CODEX_HOME")).copied(),
            Some(home.as_os_str())
        );
        for key in [
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "OPENAI_BASE_URL",
            "CODEX_HOME_OVERRIDE",
        ] {
            assert!(!environment.contains_key(std::ffi::OsStr::new(key)));
        }
    }

    #[test]
    fn snapshot_selects_only_available_free_codex_credits() {
        let result = parse_snapshot(json!({"accountId":"account", "ordinaryUsageAllowed":false,
            "rateLimitResetCredits":{"availableCount":3,"credits":[
                {"id":"free","resetType":"codexRateLimits","status":"available"},
                {"id":"spent","resetType":"codexRateLimits","status":"redeemed"},
                {"id":"other","resetType":"unknown","status":"available"}]}}))
        .unwrap();
        assert_eq!(result.account_id, "account");
        assert_eq!(result.available_count, 3);
        assert_eq!(result.credits.len(), 1);
        assert_eq!(result.credits[0].id, "free");
        assert_eq!(result.ordinary_usage_allowed, Some(false));
    }

    #[test]
    fn snapshot_missing_identity_or_credit_summary_fails_closed() {
        assert!(parse_snapshot(json!({"rateLimitResetCredits":{"availableCount":1}})).is_err());
        assert!(parse_snapshot(json!({"accountId":"account"})).is_err());
        let snapshot = parse_snapshot(json!({"accountId":"account", "rateLimitResetCredits":{"availableCount":0,"credits":null}})).unwrap();
        assert!(snapshot.credits.is_empty());
        assert_eq!(snapshot.ordinary_usage_allowed, None);
    }

    #[test]
    fn consume_outcomes_are_explicit_and_unknown_is_an_error() {
        for (wire, expected) in [
            ("reset", ResetOutcome::Reset),
            ("alreadyRedeemed", ResetOutcome::AlreadyRedeemed),
            ("nothingToReset", ResetOutcome::NothingToReset),
            ("noCredit", ResetOutcome::NoCredit),
        ] {
            assert_eq!(parse_outcome(json!({"outcome":wire})).unwrap(), expected);
        }
        assert!(parse_outcome(json!({"outcome":"success"})).is_err());
        assert!(parse_outcome(json!({})).is_err());
    }
}
