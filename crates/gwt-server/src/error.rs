use axum::{http::StatusCode, response::IntoResponse, Json};
use gwt_core::StructuredError;

/// Wrapper to convert `StructuredError` into an axum HTTP response.
pub struct HttpError(pub StructuredError);

impl IntoResponse for HttpError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::to_value(&self.0).unwrap_or_default();
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}

impl From<StructuredError> for HttpError {
    fn from(e: StructuredError) -> Self {
        Self(e)
    }
}

/// Run a blocking closure on the tokio blocking pool.
pub async fn blocking<T, F>(cmd: &'static str, f: F) -> Result<Json<T>, HttpError>
where
    T: serde::Serialize + Send + 'static,
    F: FnOnce() -> Result<T, StructuredError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(StructuredError::internal(&e.to_string(), cmd)))
        .map(Json)
        .map_err(HttpError::from)
}
