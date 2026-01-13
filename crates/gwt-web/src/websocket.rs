//! WebSocket support for terminal

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use crate::api::AppState;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Ready { session_id: String },
    Output { data: String },
    Error { message: String },
}

struct PtySession {
    id: String,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

impl PtySession {
    fn new(working_dir: &Path, shell: Option<&str>) -> Result<Self, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| err.to_string())?;

        let shell_path = shell.map(str::to_string).unwrap_or_else(default_shell);
        let mut cmd = CommandBuilder::new(shell_path);
        cmd.cwd(working_dir);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|err| err.to_string())?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|err| err.to_string())?;
        let writer = pair.master.take_writer().map_err(|err| err.to_string())?;

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            master: pair.master,
            child,
            reader,
            writer,
        })
    }
}

struct PtyIo {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
}

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let session = match PtySession::new(&state.repo_path, None) {
        Ok(session) => session,
        Err(message) => {
            let (mut sender, _) = socket.split();
            let payload = ServerMessage::Error { message };
            if let Ok(text) = serde_json::to_string(&payload) {
                let _ = sender.send(Message::Text(text.into())).await;
            }
            return;
        }
    };

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (pty_tx, mut pty_rx) = mpsc::channel::<String>(64);

    let mut reader = session.reader;
    let io = Arc::new(Mutex::new(PtyIo {
        master: session.master,
        writer: session.writer,
    }));
    let child = Arc::new(Mutex::new(session.child));

    let session_id = session.id.clone();
    let ready = ServerMessage::Ready { session_id };
    if let Ok(text) = serde_json::to_string(&ready) {
        let _ = ws_sender.send(Message::Text(text.into())).await;
    }

    let output_task = tokio::task::spawn_blocking(move || {
        let mut buffer = [0u8; 8192];
        loop {
            let bytes = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(bytes) => bytes,
                Err(_) => break,
            };
            let data = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            if pty_tx.blocking_send(data).is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            output = pty_rx.recv() => {
                let Some(data) = output else { break };
                let payload = ServerMessage::Output { data };
                if let Ok(text) = serde_json::to_string(&payload) {
                    if ws_sender.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
            message = ws_receiver.next() => {
                let Some(Ok(message)) = message else { break };
                match message {
                    Message::Text(text) => {
                        handle_client_message(text.to_string(), &io, &mut ws_sender).await;
                    }
                    Message::Binary(_) => {
                        let payload = ServerMessage::Error {
                            message: "Binary frames are not supported.".to_string(),
                        };
                        if let Ok(text) = serde_json::to_string(&payload) {
                            let _ = ws_sender.send(Message::Text(text.into())).await;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    let mut child = child.lock().await;
    let _ = child.kill();
    let _ = child.wait();

    let _ = output_task.await;
}

async fn handle_client_message(
    text: String,
    io: &Arc<Mutex<PtyIo>>,
    ws_sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) {
    match serde_json::from_str::<ClientMessage>(&text) {
        Ok(ClientMessage::Input { data }) => {
            let write_result = {
                let mut io = io.lock().await;
                let result = io.writer.write_all(data.as_bytes());
                if result.is_ok() {
                    let _ = io.writer.flush();
                }
                result
            };
            if write_result.is_err() {
                send_error(ws_sender, "Failed to write to PTY.").await;
            }
        }
        Ok(ClientMessage::Resize { cols, rows }) => {
            let io = io.lock().await;
            let _ = io.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        Err(_) => {
            send_error(ws_sender, "Invalid WebSocket payload.").await;
        }
    }
}

async fn send_error(
    ws_sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: &str,
) {
    let payload = ServerMessage::Error {
        message: message.to_string(),
    };
    if let Ok(text) = serde_json::to_string(&payload) {
        let _ = ws_sender.send(Message::Text(text.into())).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_message_input_parse() {
        let message: ClientMessage =
            serde_json::from_str(r#"{"type":"input","data":"ls"}"#).unwrap();
        match message {
            ClientMessage::Input { data } => assert_eq!(data, "ls"),
            _ => panic!("expected input message"),
        }
    }

    #[test]
    fn test_client_message_resize_parse() {
        let message: ClientMessage =
            serde_json::from_str(r#"{"type":"resize","cols":120,"rows":32}"#).unwrap();
        match message {
            ClientMessage::Resize { cols, rows } => {
                assert_eq!(cols, 120);
                assert_eq!(rows, 32);
            }
            _ => panic!("expected resize message"),
        }
    }

    #[test]
    fn test_server_message_ready_serializes() {
        let message = ServerMessage::Ready {
            session_id: "session-123".to_string(),
        };
        let json = serde_json::to_string(&message).unwrap();
        assert!(json.contains(r#""type":"ready""#));
        assert!(json.contains(r#""session_id":"session-123""#));
    }
}
