mod error;
mod handlers;
mod server;
mod state;
mod ws;

use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() {
    // Initialize tracing.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gwt_server=info,gwt_core=info".into()),
        )
        .init();

    // Create shared state with event broadcaster.
    let broadcaster = ws::EventBroadcaster::new(4096);
    let app_state = Arc::new(state::AppState::new(broadcaster));

    // Bind to random port on localhost.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind TCP listener");

    let port = listener
        .local_addr()
        .expect("failed to get local address")
        .port();

    // Store port in state.
    app_state
        .http_port
        .store(port, std::sync::atomic::Ordering::Relaxed);

    // Output port for Electron to read from stdout.
    println!("GWT_SERVER_PORT={port}");

    info!(port, "gwt-server listening");

    // Build router and serve.
    let router = server::build_router(app_state);

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    info!("gwt-server shut down");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutdown signal received");
}
