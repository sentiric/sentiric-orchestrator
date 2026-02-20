use axum::{
    extract::{State, Query, Path, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    http::{StatusCode, header},
    Json, Router,
};
use std::sync::Arc;
use crate::core::domain::{ActionParams, ToggleParams, ClusterReport, ServiceInstance};
use crate::AppState;
use futures_util::StreamExt;

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/ui/css/theme.css", get(css_theme_handler))
        .route("/ui/css/layout.css", get(css_layout_handler))
        .route("/ui/js/app.js", get(js_app_handler))
        .route("/ui/js/websocket.js", get(js_ws_handler))
        .route("/ws", get(ws_handler))
        .route("/ws/logs/:id", get(ws_logs_handler))
        
        // API
        .route("/api/status", get(status_handler))
        .route("/api/update", post(update_handler))
        .route("/api/toggle-autopilot", post(toggle_handler))
        .route("/api/service/:id/start", post(start_handler))
        .route("/api/service/:id/stop", post(stop_handler))
        .route("/api/service/:id/restart", post(restart_handler))
        .route("/api/service/:id/inspect", get(inspect_handler))
        .route("/api/system/prune", post(prune_handler))
        .route("/api/export/llm", get(export_llm_handler))
        
        // HIVE MIND
        .route("/api/ingest/report", post(ingest_report_handler))
        
        .with_state(state)
}

// HANDLERS

async fn ingest_report_handler(State(state): State<Arc<AppState>>, Json(report): Json<ClusterReport>) -> StatusCode {
    let node_name = report.node.clone();
    state.cluster_cache.lock().await.insert(node_name, report);
    
    // UI Update Trigger
    let cluster_map = state.cluster_cache.lock().await.clone();
    let _ = state.tx.send(serde_json::json!({ "type": "cluster_update", "data": cluster_map }).to_string());
    
    StatusCode::OK
}

async fn export_llm_handler(State(state): State<Arc<AppState>>) -> String {
    let cluster = state.cluster_cache.lock().await;
    let mut report = String::from("# 🤖 SENTIRIC CLUSTER DIAGNOSTIC REPORT\n\n");
    for (node, data) in cluster.iter() {
        report.push_str(&format!("## NODE: {} (Status: {})\n", node, data.stats.status));
        report.push_str(&format!("CPU: {:.1}% | RAM: {}MB\n", data.stats.cpu_usage, data.stats.ram_used));
        for svc in &data.services {
             report.push_str(&format!("- [{}] {} (Img: {})\n", svc.status, svc.name, svc.image));
        }
        report.push_str("\n");
    }
    report
}

async fn inspect_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.docker.inspect_service(&id).await { Ok(d) => Json(d).into_response(), Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response() }
}
async fn prune_handler(State(state): State<Arc<AppState>>) -> Response {
    match state.docker.prune_system().await { Ok(m) => (StatusCode::OK, m).into_response(), Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response() }
}
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse { ws.on_upgrade(|socket| handle_socket(socket, state)) }
async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.tx.subscribe();
    while let Ok(msg) = rx.recv().await { if socket.send(Message::Text(msg)).await.is_err() { break; } }
}
async fn ws_logs_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>, Path(id): Path<String>) -> impl IntoResponse { ws.on_upgrade(move |socket| handle_log_socket(socket, state, id)) }
async fn handle_log_socket(mut socket: WebSocket, state: Arc<AppState>, id: String) {
    let mut log_stream = state.docker.get_log_stream(&id);
    while let Some(res) = log_stream.next().await {
        if let Ok(out) = res {
             let b: Vec<u8> = match out { bollard::container::LogOutput::StdOut{message} => message.into(), bollard::container::LogOutput::StdErr{message} => message.into(), _ => vec![] };
             if socket.send(Message::Text(String::from_utf8_lossy(&b).to_string())).await.is_err() { break; }
        }
    }
}
async fn status_handler(State(state): State<Arc<AppState>>) -> Json<Vec<ServiceInstance>> {
    let s = state.services_cache.lock().await; Json(s.values().cloned().collect())
}
async fn update_handler(State(state): State<Arc<AppState>>, Query(p): Query<ActionParams>) -> Response {
    match state.docker.force_update_service(&p.service).await { Ok(m) => (StatusCode::OK, m).into_response(), Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response() }
}
async fn toggle_handler(State(state): State<Arc<AppState>>, Json(p): Json<ToggleParams>) -> Json<bool> {
    state.auto_pilot_config.lock().await.insert(p.service, p.enabled); Json(p.enabled)
}
async fn start_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.docker.start_service(&id).await { Ok(_) => (StatusCode::OK, "Started").into_response(), Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response() }
}
async fn stop_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.docker.stop_service(&id).await { Ok(_) => (StatusCode::OK, "Stopped").into_response(), Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response() }
}
async fn restart_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.docker.restart_service(&id).await { Ok(_) => (StatusCode::OK, "Restarted").into_response(), Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response() }
}
async fn index_handler() -> impl IntoResponse { Html(include_str!("../ui/index.html")) }
async fn css_theme_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "text/css")], include_str!("../ui/css/theme.css")) }
async fn css_layout_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "text/css")], include_str!("../ui/css/layout.css")) }
async fn js_app_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "application/javascript")], include_str!("../ui/js/app.js")) }
async fn js_ws_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "application/javascript")], include_str!("../ui/js/websocket.js")) }