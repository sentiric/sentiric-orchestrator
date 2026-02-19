use axum::{
    extract::{State, Query, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use tower_http::services::ServeDir;
use std::sync::Arc;
use crate::core::domain::{ActionParams, ToggleParams};
use crate::AppState;

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .route("/api/status", get(status_handler))
        .route("/api/update", post(update_handler))
        .route("/api/toggle-autopilot", post(toggle_handler))
        .nest_service("/ui", ServeDir::new("src/ui"))
        .with_state(state)
}

async fn index_handler() -> impl IntoResponse {
    match std::fs::read_to_string("src/ui/index.html") {
        Ok(html) => Html(html),
        Err(_) => Html("<h1>Error: UI not found</h1>".to_string()),
    }
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.tx.subscribe();
    while let Ok(msg) = rx.recv().await {
        if socket.send(Message::Text(msg)).await.is_err() { break; }
    }
}

async fn status_handler(State(state): State<Arc<AppState>>) -> Json<Vec<crate::core::domain::ServiceInstance>> {
    let services = state.services_cache.lock().await;
    Json(services.values().cloned().collect())
}

async fn update_handler(State(state): State<Arc<AppState>>, Query(p): Query<ActionParams>) -> impl IntoResponse {
    match state.docker.update_service(&p.service).await {
        Ok(msg) => (axum::http::StatusCode::OK, msg).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn toggle_handler(State(state): State<Arc<AppState>>, Json(p): Json<ToggleParams>) -> Json<bool> {
    state.auto_pilot_config.lock().await.insert(p.service, p.enabled);
    Json(p.enabled)
}