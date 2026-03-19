//! Profiles (env + AI settings) management commands

use gwt_core::ai::{format_error_for_display, AIClient, ModelInfo};
use gwt_core::config::ProfilesConfig;
use gwt_core::StructuredError;
use std::panic::{catch_unwind, AssertUnwindSafe};
use tauri::AppHandle;
use tracing::{error, instrument};

fn with_panic_guard<T>(
    context: &str,
    command: &str,
    f: impl FnOnce() -> Result<T, StructuredError>,
) -> Result<T, StructuredError> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            error!(
                category = "tauri",
                operation = context,
                "Unexpected panic while handling profiles command"
            );
            Err(StructuredError::internal(
                &format!("Unexpected error while {}", context),
                command,
            ))
        }
    }
}

/// Get current profiles config (global: ~/.gwt/config.toml [profiles]).
#[instrument(skip_all, fields(command = "get_profiles"))]
#[tauri::command]
pub fn get_profiles() -> Result<ProfilesConfig, StructuredError> {
    with_panic_guard("loading profiles", "get_profiles", || {
        ProfilesConfig::load().map_err(|e| StructuredError::from_gwt_error(&e, "get_profiles"))
    })
}

/// Save profiles config (writes into ~/.gwt/config.toml [profiles]).
#[instrument(skip_all, fields(command = "save_profiles"))]
#[tauri::command]
pub fn save_profiles(config: ProfilesConfig, app_handle: AppHandle) -> Result<(), StructuredError> {
    with_panic_guard("saving profiles", "save_profiles", || {
        config
            .save()
            .map_err(|e| StructuredError::from_gwt_error(&e, "save_profiles"))?;
        let _ = crate::menu::rebuild_menu(&app_handle);
        Ok(())
    })
}

/// List AI models from a specific OpenAI-compatible endpoint (`GET /models`).
#[instrument(skip_all, fields(command = "list_ai_models"))]
#[tauri::command]
pub fn list_ai_models(
    endpoint: String,
    api_key: String,
) -> Result<Vec<ModelInfo>, StructuredError> {
    with_panic_guard("listing ai models", "list_ai_models", || {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            return Err(StructuredError::internal(
                "Endpoint is required",
                "list_ai_models",
            ));
        }

        let client = AIClient::new_for_list_models(endpoint, api_key.trim()).map_err(|e| {
            StructuredError::internal(&format_error_for_display(&e), "list_ai_models")
        })?;
        let mut models = client.list_models().map_err(|e| {
            StructuredError::internal(&format_error_for_display(&e), "list_ai_models")
        })?;
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models.dedup_by(|a, b| a.id == b.id);
        Ok(models)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
    use tauri::{
        ipc::{CallbackFn, InvokeBody},
        webview::InvokeRequest,
        WebviewWindowBuilder,
    };

    fn spawn_models_server(
        body: &'static str,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have local addr");
        let (tx, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("server should accept client");
            let mut request = Vec::new();
            let mut buf = [0_u8; 4096];

            loop {
                let read = stream.read(&mut buf).expect("request should be readable");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            tx.send(String::from_utf8_lossy(&request).to_string())
                .expect("request should be captured");

            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should be writable");
        });

        (format!("http://{addr}/v1"), rx, handle)
    }

    #[test]
    fn list_ai_models_rejects_empty_endpoint() {
        let err = list_ai_models("   ".to_string(), String::new()).unwrap_err();
        assert!(err.message.contains("Endpoint is required"));
    }

    #[test]
    fn list_ai_models_rejects_invalid_endpoint() {
        let err = list_ai_models("not-a-url".to_string(), String::new()).unwrap_err();
        assert!(
            err.message.contains("Invalid endpoint"),
            "unexpected error message: {}",
            err.message
        );
    }

    #[test]
    fn list_ai_models_accepts_camel_case_api_key_over_ipc() {
        let (endpoint, rx, handle) =
            spawn_models_server(r#"{"data":[{"id":"gpt-4o-mini"},{"id":"gpt-5"}]}"#);
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![list_ai_models])
            .build(mock_context(noop_assets()))
            .expect("mock tauri app should build");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview should build");
        let response = get_ipc_response(
            &webview,
            InvokeRequest {
                cmd: "list_ai_models".into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: InvokeBody::Json(serde_json::json!({
                    "endpoint": endpoint,
                    "apiKey": "sk-live-check"
                })),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .expect("IPC request should succeed");
        let models = response
            .deserialize::<Vec<ModelInfo>>()
            .expect("response should deserialize");
        let raw_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("server should capture request");
        handle.join().expect("server thread should finish");
        let normalized_request = raw_request.to_ascii_lowercase();

        assert!(raw_request.starts_with("GET /v1/models HTTP/1.1"));
        assert!(normalized_request.contains("\r\nauthorization: bearer sk-live-check\r\n"));
        assert_eq!(
            models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
            vec!["gpt-4o-mini".to_string(), "gpt-5".to_string()]
        );
    }
}
