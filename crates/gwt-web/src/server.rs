//! Axum web server

use axum::{
    routing::{delete, get, post},
    Router,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

use crate::api::{self, AppState};

/// Server configuration
pub struct ServerConfig {
    pub port: u16,
    pub address: String,
    pub cors_enabled: bool,
    pub repo_path: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3000,
            address: "127.0.0.1".to_string(),
            cors_enabled: true,
            repo_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

impl ServerConfig {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            ..Default::default()
        }
    }

    pub fn with_address(mut self, address: impl Into<String>) -> Self {
        self.address = address.into();
        self
    }

    pub fn with_repo_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.repo_path = path.into();
        self
    }

    pub fn with_cors(mut self, enabled: bool) -> Self {
        self.cors_enabled = enabled;
        self
    }
}

/// Build the router with all API routes
fn build_router(state: Arc<AppState>, cors_enabled: bool) -> Router {
    let api_routes = Router::new()
        // Health
        .route("/health", get(api::health))
        // Worktrees
        .route("/worktrees", get(api::list_worktrees))
        .route("/worktrees", post(api::create_worktree))
        .route("/worktrees/:branch", delete(api::delete_worktree))
        // Branches
        .route("/branches", get(api::list_branches))
        .route("/branches", post(api::create_branch))
        .route("/branches/:name", delete(api::delete_branch))
        // Settings
        .route("/settings", get(api::get_settings))
        .with_state(state);

    let router = Router::new().nest("/api", api_routes);

    if cors_enabled {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);
        router.layer(cors)
    } else {
        router
    }
}

/// Start the web server with default configuration
pub async fn serve(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let config = ServerConfig::new(port);
    serve_with_config(config).await
}

/// Start the web server with custom configuration
pub async fn serve_with_config(config: ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(AppState::new(config.repo_path));
    let app = build_router(state, config.cors_enabled);

    let addr: SocketAddr = format!("{}:{}", config.address, config.port).parse()?;
    tracing::info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.port, 3000);
        assert_eq!(config.address, "127.0.0.1");
        assert!(config.cors_enabled);
    }

    #[test]
    fn test_server_config_builder() {
        let config = ServerConfig::new(8080)
            .with_address("0.0.0.0")
            .with_cors(false)
            .with_repo_path("/tmp/repo");

        assert_eq!(config.port, 8080);
        assert_eq!(config.address, "0.0.0.0");
        assert!(!config.cors_enabled);
        assert_eq!(config.repo_path, PathBuf::from("/tmp/repo"));
    }
}
