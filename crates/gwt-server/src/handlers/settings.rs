use axum::{response::IntoResponse, Json};
use gwt_core::config::Settings;

pub async fn get_settings() -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(|| -> Result<serde_json::Value, String> {
        let settings = Settings::load_global_raw().map_err(|e| e.to_string())?;
        serde_json::to_value(settings).map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(settings)) => Json(settings).into_response(),
        Ok(Err(e)) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e })),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn save_settings(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        // Settings are saved via gwt-core's global save mechanism.
        // For now, serialize to the config file path directly.
        let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
        let config_path = home.join(".gwt").join("config.toml");
        std::fs::create_dir_all(config_path.parent().unwrap()).map_err(|e| e.to_string())?;

        let settings: Settings = serde_json::from_value(body).map_err(|e| e.to_string())?;
        let toml_str = toml::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        std::fs::write(&config_path, toml_str).map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(e)) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e })),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e.to_string() })),
        )
            .into_response(),
    }
}
