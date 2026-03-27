use axum::{response::IntoResponse, Json};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathRequest {
    pub path: String,
}

pub async fn probe_path(Json(req): Json<PathRequest>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        let path = std::path::PathBuf::from(&req.path);
        let exists = path.exists();
        let is_dir = path.is_dir();
        let is_git = gwt_core::git::is_git_repo(&path);
        serde_json::json!({
            "exists": exists,
            "isDirectory": is_dir,
            "isGitRepo": is_git,
            "path": req.path,
        })
    })
    .await;

    match result {
        Ok(info) => Json(info).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn is_git_repo(Json(req): Json<PathRequest>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        gwt_core::git::is_git_repo(&std::path::PathBuf::from(&req.path))
    })
    .await;

    Json(serde_json::json!({ "isGitRepo": result.unwrap_or(false) }))
}

pub async fn get_current_branch(Json(req): Json<PathRequest>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        let ctx = gwt_core::git::get_header_context(&std::path::PathBuf::from(&req.path));
        ctx.branch_name
    })
    .await;

    match result {
        Ok(Some(branch)) => Json(serde_json::json!({ "branch": branch })).into_response(),
        Ok(None) => Json(serde_json::json!({ "branch": null })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e.to_string() })),
        )
            .into_response(),
    }
}
