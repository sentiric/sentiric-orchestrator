use axum::{
    extract::{State, Query, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    http::{StatusCode, header},
    Json, Router,
};
use std::sync::Arc;
use crate::core::domain::{ActionParams, ToggleParams};
use crate::AppState;

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        // Statik dosyaları binary içinden sunuyoruz
        .route("/ui/css/theme.css", get(css_theme_handler))
        .route("/ui/css/layout.css", get(css_layout_handler)) // Eğer layout.css varsa
        .route("/ui/js/app.js", get(js_app_handler))
        .route("/ui/js/websocket.js", get(js_ws_handler))
        // API
        .route("/api/status", get(status_handler))
        .route("/api/update", post(update_handler))
        .route("/api/toggle-autopilot", post(toggle_handler))
        .with_state(state)
}

// HTML Handler
async fn index_handler() -> impl IntoResponse {
    Html(include_str!("../ui/index.html"))
}

// CSS Handlers
async fn css_theme_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("../ui/css/theme.css"),
    )
}

// Eğer layout.css oluşturmadıysan bu handler'ı ve route'u silebilirsin
async fn css_layout_handler() -> Response {
    // Dosya yoksa 404 dön veya boş string
    // include_str! hata verirse burayı silebilirsin.
    // Şimdilik boş dönüyorum hata vermemesi için
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css")],
        "", 
    ).into_response()
}

// JS Handlers
async fn js_app_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("../ui/js/app.js"),
    )
}

async fn js_ws_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("../ui/js/websocket.js"),
    )
}

// ... (Geri kalan ws_handler, handle_socket, status_handler vb. AYNI KALACAK)
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
        Ok(msg) => (StatusCode::OK, msg).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn toggle_handler(State(state): State<Arc<AppState>>, Json(p): Json<ToggleParams>) -> Json<bool> {
    state.auto_pilot_config.lock().await.insert(p.service, p.enabled);
    Json(p.enabled)
}