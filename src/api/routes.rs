// src/api/routes.rs
use axum::{
    extract::{State, Query, Path, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    http::{StatusCode, header},
    Json, Router,
};
use std::sync::Arc;
use crate::core::domain::{ActionParams, ToggleParams, ClusterReport, ServiceInstance, TopologyMap, TopologyNode, TopologyEdge};
use crate::AppState;
use futures_util::StreamExt;
use tracing::info;

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/ui/css/theme.css", get(css_theme_handler))
        .route("/ui/css/layout.css", get(css_layout_handler))
        .route("/ui/js/app.js", get(js_app_handler))
        .route("/ui/js/websocket.js", get(js_ws_handler))
        .route("/ui/js/components/topology.js", get(js_topology_handler)) // YENİ
        .route("/ws", get(ws_handler))
        .route("/ws/logs/:id", get(ws_logs_handler))
        
        // API
        .route("/api/status", get(status_handler))
        .route("/api/topology", get(topology_handler)) // YENİ
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

// --- YENİ: SENTIRIC ANAYASAL TOPOLOJİSİ (HARDCODED EXPECTED STATE) ---
async fn topology_handler() -> Json<TopologyMap> {
    let nodes = vec![
        // Edge & Telecom
        TopologyNode { id: "sbc-service".into(), label: "SBC\n(Edge)".into(), group: "edge".into() },
        TopologyNode { id: "proxy-service".into(), label: "Proxy\n(Router)".into(), group: "telecom".into() },
        TopologyNode { id: "b2bua-service".into(), label: "B2BUA\n(Session)".into(), group: "telecom".into() },
        TopologyNode { id: "registrar-service".into(), label: "Registrar\n(Location)".into(), group: "telecom".into() },
        TopologyNode { id: "media-service".into(), label: "Media\n(RTP Engine)".into(), group: "telecom".into() },
        
        // Core Logic
        TopologyNode { id: "dialplan-service".into(), label: "Dialplan\n(Routing)".into(), group: "core".into() },
        TopologyNode { id: "user-service".into(), label: "User\n(Identity)".into(), group: "core".into() },
        TopologyNode { id: "workflow-service".into(), label: "Workflow\n(Cortex)".into(), group: "core".into() },
        TopologyNode { id: "agent-service".into(), label: "Agent\n(Orchestrator)".into(), group: "core".into() },
        
        // AI Gateways
        TopologyNode { id: "stt-gateway-service".into(), label: "STT\nGateway".into(), group: "ai".into() },
        TopologyNode { id: "tts-gateway-service".into(), label: "TTS\nGateway".into(), group: "ai".into() },
        TopologyNode { id: "llm-gateway-service".into(), label: "LLM\nGateway".into(), group: "ai".into() },
        
        // Infra
        TopologyNode { id: "rabbitmq".into(), label: "RabbitMQ\n(Event Bus)".into(), group: "infra".into() },
        TopologyNode { id: "redis".into(), label: "Redis\n(State)".into(), group: "infra".into() },
        TopologyNode { id: "postgres".into(), label: "Postgres\n(Data)".into(), group: "infra".into() },
    ];

    let edges = vec![
        // SIP Flow
        TopologyEdge { from: "sbc-service".into(), to: "proxy-service".into(), label: "SIP".into(), dashes: false },
        TopologyEdge { from: "proxy-service".into(), to: "b2bua-service".into(), label: "SIP".into(), dashes: false },
        TopologyEdge { from: "proxy-service".into(), to: "registrar-service".into(), label: "gRPC".into(), dashes: false },
        TopologyEdge { from: "proxy-service".into(), to: "dialplan-service".into(), label: "gRPC".into(), dashes: false },
        
        // B2BUA Media & Events
        TopologyEdge { from: "b2bua-service".into(), to: "media-service".into(), label: "gRPC".into(), dashes: false },
        TopologyEdge { from: "b2bua-service".into(), to: "rabbitmq".into(), label: "AMQP".into(), dashes: false },
        TopologyEdge { from: "b2bua-service".into(), to: "redis".into(), label: "TCP".into(), dashes: false },
        TopologyEdge { from: "b2bua-service".into(), to: "dialplan-service".into(), label: "gRPC".into(), dashes: false },
        
        // Logic Data
        TopologyEdge { from: "registrar-service".into(), to: "redis".into(), label: "TCP".into(), dashes: false },
        TopologyEdge { from: "registrar-service".into(), to: "user-service".into(), label: "gRPC".into(), dashes: false },
        TopologyEdge { from: "dialplan-service".into(), to: "user-service".into(), label: "gRPC".into(), dashes: false },
        TopologyEdge { from: "dialplan-service".into(), to: "postgres".into(), label: "TCP".into(), dashes: false },
        TopologyEdge { from: "user-service".into(), to: "postgres".into(), label: "TCP".into(), dashes: false },
        
        // Workflow Cortex
        TopologyEdge { from: "rabbitmq".into(), to: "workflow-service".into(), label: "AMQP".into(), dashes: false },
        TopologyEdge { from: "workflow-service".into(), to: "postgres".into(), label: "TCP".into(), dashes: false },
        TopologyEdge { from: "workflow-service".into(), to: "redis".into(), label: "TCP".into(), dashes: false },
        TopologyEdge { from: "workflow-service".into(), to: "media-service".into(), label: "gRPC".into(), dashes: true },
        TopologyEdge { from: "workflow-service".into(), to: "agent-service".into(), label: "gRPC".into(), dashes: false },
        TopologyEdge { from: "workflow-service".into(), to: "b2bua-service".into(), label: "gRPC".into(), dashes: true },

        // Agent & AI
        TopologyEdge { from: "agent-service".into(), to: "stt-gateway-service".into(), label: "gRPC".into(), dashes: false },
        TopologyEdge { from: "agent-service".into(), to: "tts-gateway-service".into(), label: "gRPC".into(), dashes: false },
        TopologyEdge { from: "agent-service".into(), to: "llm-gateway-service".into(), label: "gRPC".into(), dashes: false },
        TopologyEdge { from: "agent-service".into(), to: "redis".into(), label: "TCP".into(), dashes: false },
    ];

    Json(TopologyMap { nodes, edges })
}

async fn ingest_report_handler(State(state): State<Arc<AppState>>, Json(report): Json<ClusterReport>) -> StatusCode {
    let node_name = report.node.clone();
    state.cluster_cache.lock().await.insert(node_name, report);
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
    info!(event="MANUAL_UPDATE_TRIGGERED", service=%p.service, "API Update Request");
    match state.docker.force_update_service(&p.service).await { Ok(m) => (StatusCode::OK, m).into_response(), Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response() }
}
async fn toggle_handler(State(state): State<Arc<AppState>>, Json(p): Json<ToggleParams>) -> Json<bool> {
    info!(event="AUTOPILOT_TOGGLED", service=%p.service, enabled=%p.enabled, "Auto-pilot toggle");
    state.auto_pilot_config.lock().await.insert(p.service, p.enabled); Json(p.enabled)
}
async fn start_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    info!(event="MANUAL_START", container=%id, "API Start Request");
    match state.docker.start_service(&id).await { Ok(_) => (StatusCode::OK, "Started").into_response(), Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response() }
}
async fn stop_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    info!(event="MANUAL_STOP", container=%id, "API Stop Request");
    match state.docker.stop_service(&id).await { Ok(_) => (StatusCode::OK, "Stopped").into_response(), Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response() }
}
async fn restart_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    info!(event="MANUAL_RESTART", container=%id, "API Restart Request");
    match state.docker.restart_service(&id).await { Ok(_) => (StatusCode::OK, "Restarted").into_response(), Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response() }
}
async fn index_handler() -> impl IntoResponse { Html(include_str!("../ui/index.html")) }
async fn css_theme_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "text/css")], include_str!("../ui/css/theme.css")) }
async fn css_layout_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "text/css")], include_str!("../ui/css/layout.css")) }
async fn js_app_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "application/javascript")], include_str!("../ui/js/app.js")) }
async fn js_ws_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "application/javascript")], include_str!("../ui/js/websocket.js")) }
async fn js_topology_handler() -> impl IntoResponse { ([(header::CONTENT_TYPE, "application/javascript")], include_str!("../ui/js/components/topology.js")) }