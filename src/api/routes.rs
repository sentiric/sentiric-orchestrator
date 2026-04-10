// src/api/routes.rs
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use std::sync::Arc;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::core::domain::{
    ActionParams, ClusterReport, ServiceInstance, ToggleParams, TopologyEdge, TopologyMap,
    TopologyNode,
};
use crate::AppState;
use serde_json::json;

const UI_ASSETS_PATH: &str = "src/ui";

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .nest_service("/ui", ServeDir::new(UI_ASSETS_PATH))
        .route("/ws", get(ws_handler))
        .route("/ws/logs/:id", get(ws_logs_handler))
        // [KRİTİK DÜZELTME]: 404 Hatasını çözen rota eklendi!
        .route("/api/config", get(get_system_config))
        .route("/api/status", get(status_handler))
        .route("/api/topology", get(topology_handler))
        .route("/api/update", post(update_handler))
        .route("/api/toggle-autopilot", post(toggle_handler))
        .route("/api/service/:id/start", post(start_handler))
        .route("/api/service/:id/stop", post(stop_handler))
        .route("/api/service/:id/restart", post(restart_handler))
        .route("/api/service/:id/inspect", get(inspect_handler))
        .route("/api/system/prune", post(prune_handler))
        .route("/api/export/llm", get(export_llm_handler))
        .route("/api/ingest/report", post(ingest_report_handler))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn get_system_config(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let version = env!("CARGO_PKG_VERSION");
    let node_name = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or("unknown".into());
    Json(json!({
        "version": version,
        "node_name": node_name,
        "is_upstream_enabled": !std::env::var("UPSTREAM_ORCHESTRATOR_URL").unwrap_or_default().is_empty(),
    }))
}

async fn index_handler() -> impl IntoResponse {
    match std::fs::read_to_string(format!("{}/index.html", UI_ASSETS_PATH)) {
        Ok(html) => Html(html),
        Err(_) => Html("<h1>System Error: UI assets not found.</h1>".to_string()),
    }
}

async fn topology_handler() -> Json<TopologyMap> {
    let nodes = vec![
        TopologyNode {
            id: "sip-sbc-service".into(),
            label: "SBC\n(Edge)".into(),
            group: "edge".into(),
        },
        TopologyNode {
            id: "sip-proxy-service".into(),
            label: "Proxy\n(Router)".into(),
            group: "telecom".into(),
        },
        TopologyNode {
            id: "sip-b2bua-service".into(),
            label: "B2BUA\n(Session)".into(),
            group: "telecom".into(),
        },
        TopologyNode {
            id: "sip-registrar-service".into(),
            label: "Registrar\n(Location)".into(),
            group: "telecom".into(),
        },
        TopologyNode {
            id: "media-service".into(),
            label: "Media\n(RTP Engine)".into(),
            group: "telecom".into(),
        },
        TopologyNode {
            id: "dialplan-service".into(),
            label: "Dialplan\n(Routing)".into(),
            group: "core".into(),
        },
        TopologyNode {
            id: "user-service".into(),
            label: "User\n(Identity)".into(),
            group: "core".into(),
        },
        TopologyNode {
            id: "workflow-service".into(),
            label: "Workflow\n(Cortex)".into(),
            group: "core".into(),
        },
        TopologyNode {
            id: "agent-service".into(),
            label: "Agent\n(Orchestrator)".into(),
            group: "core".into(),
        },
        TopologyNode {
            id: "telephony-action-service".into(),
            label: "TAS\n(Pipeline)".into(),
            group: "core".into(),
        },
        TopologyNode {
            id: "dialog-service".into(),
            label: "Dialog\n(Memory)".into(),
            group: "core".into(),
        },
        TopologyNode {
            id: "stt-gateway-service".into(),
            label: "STT\nGateway".into(),
            group: "ai".into(),
        },
        TopologyNode {
            id: "tts-gateway-service".into(),
            label: "TTS\nGateway".into(),
            group: "ai".into(),
        },
        TopologyNode {
            id: "llm-gateway-service".into(),
            label: "LLM\nGateway".into(),
            group: "ai".into(),
        },
        TopologyNode {
            id: "rabbitmq".into(),
            label: "RabbitMQ\n(Event)".into(),
            group: "infra".into(),
        },
        TopologyNode {
            id: "redis".into(),
            label: "Redis\n(State)".into(),
            group: "infra".into(),
        },
        TopologyNode {
            id: "postgres".into(),
            label: "Postgres\n(Data)".into(),
            group: "infra".into(),
        },
    ];

    let edges = vec![
        TopologyEdge {
            from: "sip-sbc-service".into(),
            to: "sip-proxy-service".into(),
            label: "SIP".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "sip-proxy-service".into(),
            to: "sip-b2bua-service".into(),
            label: "SIP".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "sip-proxy-service".into(),
            to: "sip-registrar-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "sip-proxy-service".into(),
            to: "dialplan-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "sip-b2bua-service".into(),
            to: "media-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "sip-b2bua-service".into(),
            to: "dialplan-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "sip-b2bua-service".into(),
            to: "rabbitmq".into(),
            label: "AMQP".into(),
            dashes: true,
        },
        TopologyEdge {
            from: "sip-b2bua-service".into(),
            to: "redis".into(),
            label: "TCP".into(),
            dashes: true,
        },
        TopologyEdge {
            from: "sip-registrar-service".into(),
            to: "redis".into(),
            label: "TCP".into(),
            dashes: true,
        },
        TopologyEdge {
            from: "sip-registrar-service".into(),
            to: "user-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "dialplan-service".into(),
            to: "user-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "dialplan-service".into(),
            to: "postgres".into(),
            label: "TCP".into(),
            dashes: true,
        },
        TopologyEdge {
            from: "user-service".into(),
            to: "postgres".into(),
            label: "TCP".into(),
            dashes: true,
        },
        TopologyEdge {
            from: "rabbitmq".into(),
            to: "workflow-service".into(),
            label: "AMQP".into(),
            dashes: true,
        },
        TopologyEdge {
            from: "workflow-service".into(),
            to: "postgres".into(),
            label: "TCP".into(),
            dashes: true,
        },
        TopologyEdge {
            from: "workflow-service".into(),
            to: "redis".into(),
            label: "TCP".into(),
            dashes: true,
        },
        TopologyEdge {
            from: "workflow-service".into(),
            to: "media-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "workflow-service".into(),
            to: "agent-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "workflow-service".into(),
            to: "sip-b2bua-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "agent-service".into(),
            to: "telephony-action-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "agent-service".into(),
            to: "redis".into(),
            label: "TCP".into(),
            dashes: true,
        },
        TopologyEdge {
            from: "telephony-action-service".into(),
            to: "stt-gateway-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "telephony-action-service".into(),
            to: "tts-gateway-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "telephony-action-service".into(),
            to: "dialog-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
        TopologyEdge {
            from: "dialog-service".into(),
            to: "llm-gateway-service".into(),
            label: "gRPC".into(),
            dashes: false,
        },
    ];

    Json(TopologyMap { nodes, edges })
}

async fn ingest_report_handler(
    State(state): State<Arc<AppState>>,
    Json(report): Json<ClusterReport>,
) -> StatusCode {
    let node_name = report.node.clone();
    state.cluster_cache.lock().await.insert(node_name, report);
    let cluster_map = state.cluster_cache.lock().await.clone();
    let _ = state
        .tx
        .send(serde_json::json!({ "type": "cluster_update", "data": cluster_map }).to_string());
    StatusCode::OK
}

async fn export_llm_handler(State(state): State<Arc<AppState>>) -> String {
    let cluster = state.cluster_cache.lock().await;
    let mut report = String::from("# 🤖 SENTIRIC CLUSTER DIAGNOSTIC REPORT\n\n");

    report.push_str("## 1. INFRASTRUCTURE HEALTH\n");
    for (node, data) in cluster.iter() {
        report.push_str(&format!(
            "- **{}** | CPU: {:.1}% | RAM: {}/{} MB | Status: {}\n",
            node,
            data.stats.cpu_usage,
            data.stats.ram_used,
            data.stats.ram_total,
            data.stats.status
        ));
    }

    report.push_str("\n## 2. CONFIG DRIFT DETECTION\n");
    let mut service_versions: std::collections::HashMap<String, Vec<(String, String)>> =
        std::collections::HashMap::new();

    for (node, data) in cluster.iter() {
        for svc in &data.services {
            let img_hash = svc
                .image
                .split('@')
                .last()
                .unwrap_or(&svc.image)
                .to_string();
            service_versions
                .entry(svc.name.clone())
                .or_insert_with(Vec::new)
                .push((node.clone(), img_hash));
        }
    }

    let mut drift_found = false;
    for (svc_name, deployments) in service_versions {
        if deployments.len() > 1 {
            let first_hash = &deployments[0].1;
            let has_mismatch = deployments.iter().any(|d| &d.1 != first_hash);

            if has_mismatch {
                drift_found = true;
                report.push_str(&format!("⚠️ **DRIFT DETECTED: {}**\n", svc_name));
                for d in deployments {
                    report.push_str(&format!("   - {}: Image Hash {}\n", d.0, d.1));
                }
            }
        }
    }
    if !drift_found {
        report.push_str("✅ No configuration drift detected. Cluster is synchronized.\n");
    }

    report.push_str("\n## 3. SERVICE DETAILS\n");
    for (node, data) in cluster.iter() {
        report.push_str(&format!("### {}\n", node));
        for svc in &data.services {
            let status_icon = if svc.status.to_lowercase().contains("up") {
                "🟢"
            } else {
                "🔴"
            };
            report.push_str(&format!(
                "- {} **{}** | CPU: {:.1}% | RAM: {}MB | AP: {}\n",
                status_icon, svc.name, svc.cpu_usage, svc.mem_usage, svc.auto_pilot
            ));
        }
        report.push_str("\n");
    }
    report
}

async fn inspect_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    // [KRİTİK DÜZELTME]: null id gelirse çökme!
    if id.is_empty() || id == "null" {
        return (StatusCode::BAD_REQUEST, "Invalid ID").into_response();
    }
    match state.docker.inspect_service(&id).await {
        Ok(d) => Json(d).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn prune_handler(State(state): State<Arc<AppState>>) -> Response {
    match state.docker.prune_system().await {
        Ok(m) => (StatusCode::OK, m).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.tx.subscribe();
    while let Ok(msg) = rx.recv().await {
        if socket.send(Message::Text(msg)).await.is_err() {
            break;
        }
    }
}

async fn ws_logs_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_log_socket(socket, state, id))
}

async fn handle_log_socket(mut socket: WebSocket, state: Arc<AppState>, id: String) {
    if id.is_empty() || id == "null" {
        return;
    }
    let mut log_stream = state.docker.get_log_stream(&id);
    while let Some(res) = log_stream.next().await {
        if let Ok(out) = res {
            let b: Vec<u8> = match out {
                bollard::container::LogOutput::StdOut { message } => message.into(),
                bollard::container::LogOutput::StdErr { message } => message.into(),
                _ => vec![],
            };
            if socket
                .send(Message::Text(String::from_utf8_lossy(&b).to_string()))
                .await
                .is_err()
            {
                break;
            }
        }
    }
}

async fn status_handler(State(state): State<Arc<AppState>>) -> Json<Vec<ServiceInstance>> {
    let s = state.services_cache.lock().await;
    Json(s.values().cloned().collect())
}

async fn update_handler(
    State(state): State<Arc<AppState>>,
    Query(p): Query<ActionParams>,
) -> Response {
    info!(event="MANUAL_UPDATE_TRIGGERED", service=%p.service, "API Update Request");
    match state.docker.force_update_service(&p.service).await {
        Ok(m) => (StatusCode::OK, m).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn toggle_handler(
    State(state): State<Arc<AppState>>,
    Json(p): Json<ToggleParams>,
) -> Json<bool> {
    info!(event="AUTOPILOT_TOGGLED", service=%p.service, enabled=%p.enabled, "Auto-pilot toggle");
    state
        .auto_pilot_config
        .lock()
        .await
        .insert(p.service, p.enabled);
    Json(p.enabled)
}

async fn start_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    if id.is_empty() || id == "null" {
        return (StatusCode::BAD_REQUEST, "Invalid ID").into_response();
    }
    match state.docker.start_service(&id).await {
        Ok(_) => (StatusCode::OK, "Started").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn stop_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    if id.is_empty() || id == "null" {
        return (StatusCode::BAD_REQUEST, "Invalid ID").into_response();
    }
    match state.docker.stop_service(&id).await {
        Ok(_) => (StatusCode::OK, "Stopped").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn restart_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    if id.is_empty() || id == "null" {
        return (StatusCode::BAD_REQUEST, "Invalid ID").into_response();
    }
    match state.docker.restart_service(&id).await {
        Ok(_) => (StatusCode::OK, "Restarted").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
