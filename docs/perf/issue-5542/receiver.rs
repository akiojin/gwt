//! Private native receiver using the production transport's HTTP stack.
use axum::{body::Bytes, http::{HeaderMap, StatusCode}, routing::post, Router};

fn main() {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all()
        .build().unwrap().block_on(async {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            println!("http://{}/internal/hook-live", listener.local_addr().unwrap());
            let app = Router::new().route("/internal/hook-live", post(
                |headers: HeaderMap, _body: Bytes| async move {
                    if headers.get("authorization").and_then(|value| value.to_str().ok())
                        == Some("Bearer measurement-only") {
                        StatusCode::NO_CONTENT
                    } else {
                        StatusCode::UNAUTHORIZED
                    }
                }));
            axum::serve(listener, app).await.unwrap();
        });
}
