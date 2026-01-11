//! Axum web server

use axum::{routing::get, Router};
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};

use crate::api;

/// Start the web server
pub async fn serve(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/health", get(api::health))
        .route("/api/worktrees", get(api::list_worktrees))
        .route("/api/branches", get(api::list_branches))
        .layer(cors);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
