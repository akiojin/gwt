//! Static file serving for embedded frontend assets
//!
//! Uses rust-embed to bundle WASM frontend into the binary.

use axum::{
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};
use rust_embed::Embed;

/// Embedded frontend assets
///
/// In development, files are loaded from disk.
/// In release builds, files are embedded in the binary.
#[derive(Embed)]
#[folder = "../gwt-frontend/dist"]
#[prefix = ""]
struct FrontendAssets;

/// Serve the index.html file
pub async fn serve_index() -> impl IntoResponse {
    match FrontendAssets::get("index.html") {
        Some(content) => Html(content.data.into_owned()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "Frontend not built. Run 'trunk build' in gwt-frontend first.",
        )
            .into_response(),
    }
}

/// Serve a static file by path
pub async fn serve_static(path: axum::extract::Path<String>) -> impl IntoResponse {
    let path = path.0;

    match FrontendAssets::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(axum::body::Body::from(content.data.into_owned()))
                .unwrap()
        }
        None => (StatusCode::NOT_FOUND, "File not found").into_response(),
    }
}
