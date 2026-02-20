use axum::{
    extract::{State, Query, Path, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    http::{StatusCode, header},
    Json, Router,
};
use std::sync::Arc;
use crate::core::domain::{ActionParams, ToggleParams};
use crate::AppState;
use futures_util::StreamExt; // SinkExt kaldırıldı
use tracing::warn;


// DÜZELTİLDİ: handle_log_socket
async fn handle_log_socket(mut socket: WebSocket, state: Arc<AppState>, id: String) {
    let mut log_stream = state.docker.get_log_stream(&id);
    while let Some(log_result) = log_stream.next().await {
        match log_result {
            Ok(log_output) => {
                // DÜZELTME: LogOutput enum'ını match ile işle
                let bytes: Vec<u8> = match log_output {
                    bollard::container::LogOutput::StdOut { message } => message.into(),
                    bollard::container::LogOutput::StdErr { message } => message.into(),
                    bollard::container::LogOutput::Console { message } => message.into(),
                    bollard::container::LogOutput::StdIn { message } => message.into(), // Genelde kullanılmaz ama kapsayıcı olsun
                };

                if socket.send(Message::Binary(bytes)).await.is_err() {
                    break;
                }
            },
            Err(e) => {
                warn!("Log stream for {} error: {}", id, e);
                break;
            }
        }
    }
}


pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        // Static assets
        .route("/ui/css/theme.css", get(css_theme_handler))
        .route("/ui/css/layout.css", get(css_layout_handler))
        .route("/ui/js/app.js", get(js_app_handler))
        .route("/ui/js/websocket.js", get(js_ws_handler))
        // Main WebSocket
        .route("/ws", get(ws_handler))
        // Log WebSocket (YENİ)
        .route("/ws/logs/:id", get(ws_logs_handler))
        // API
        .route("/api/status", get(status_handler))
        .route("/api/update", post(update_handler))
        .route("/api/toggle-autopilot", post(toggle_handler))
        // Lifecycle API (YENİ)
        .route("/api/service/:id/start", post(start_handler))
        .route("/api/service/:id/stop", post(stop_handler))
        .route("/api/service/:id/restart", post(restart_handler))
        .with_state(state)
}

// --- WebSocket Handlers ---
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.tx.subscribe();
    while let Ok(msg) = rx.recv().await {
        if socket.send(Message::Text(msg)).await.is_err() { break; }
    }
}

// YENİ: Log WebSocket Handler
async fn ws_logs_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_log_socket(socket, state, id))
}


// --- API Handlers ---
async fn status_handler(State(state): State<Arc<AppState>>) -> Json<Vec<crate::core::domain::ServiceInstance>> {
    let services = state.services_cache.lock().await;
    Json(services.values().cloned().collect())
}

async fn update_handler(State(state): State<Arc<AppState>>, Query(p): Query<ActionParams>) -> Response {
    match state.docker.force_update_service(&p.service).await {
        Ok(msg) => (StatusCode::OK, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn toggle_handler(State(state): State<Arc<AppState>>, Json(p): Json<ToggleParams>) -> Json<bool> {
    state.auto_pilot_config.lock().await.insert(p.service, p.enabled);
    Json(p.enabled)
}

// YENİ: Lifecycle API Handlers
async fn start_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.docker.start_service(&id).await {
        Ok(_) => (StatusCode::OK, "Service started").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn stop_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.docker.stop_service(&id).await {
        Ok(_) => (StatusCode::OK, "Service stopped").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn restart_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.docker.restart_service(&id).await {
        Ok(_) => (StatusCode::OK, "Service restarted").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}


// --- Static Asset Handlers ---
async fn index_handler() -> impl IntoResponse { Html(include_str!("../ui/index.html")) }
async fn css_theme_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "text/css")], include_str!("../ui/css/theme.css")) }
async fn css_layout_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "text/css")], "") }
async fn js_app_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "application/javascript")], include_str!("../ui/js/app.js")) }
async fn js_ws_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "application/javascript")], include_str!("../ui/js/websocket.js")) }