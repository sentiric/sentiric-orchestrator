use axum::{
    extract::{State, Query, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use bollard::Docker;
use bollard::container::{
    ListContainersOptions, StopContainerOptions, RemoveContainerOptions, 
    Config, CreateContainerOptions, StartContainerOptions, NetworkingConfig
};
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;
use std::{env, net::SocketAddr, sync::Arc, time::Duration, collections::HashMap, process::Command};
use tokio::sync::{Mutex, broadcast};
use tracing::{info, debug, error}; // 'warn' silindi, 'error' kullanıldı
use serde::{Deserialize, Serialize};
use tonic::{Request, Response, Status};
use sysinfo::System; // [FIX]: CpuExt kaldırıldı, sadece System yeterli

// Proto Modülü
pub mod orchestrator_proto {
    tonic::include_proto!("sentiric.orchestrator.v1");
}
use orchestrator_proto::orchestrator_service_server::{OrchestratorService, OrchestratorServiceServer};
use orchestrator_proto::orchestrator_service_client::OrchestratorServiceClient;
use orchestrator_proto::{NodeStatus, Ack};

// --- Veri Modelleri ---

#[derive(Serialize, Clone, Debug)]
struct ServiceInstance {
    name: String,
    image: String,
    status: String,
    short_id: String,
    last_sync: String,
    auto_pilot: bool,
    node: String,
}

#[derive(Serialize, Clone, Debug, Default)]
struct LocalNodeStats {
    name: String,
    cpu_usage: f32,
    ram_used: u64,
    ram_total: u64,
    gpu_usage: f32,
    gpu_mem_used: u64,
    gpu_mem_total: u64,
    last_seen: String,
    status: String,
}

#[derive(Deserialize)]
struct ActionParams { service: String }

#[derive(Deserialize)]
struct ToggleParams { service: String, enabled: bool }

struct AppState {
    docker: Docker,
    auto_pilot_config: Mutex<HashMap<String, bool>>,
    nodes_cache: Mutex<HashMap<String, LocalNodeStats>>,
    services_cache: Mutex<Vec<ServiceInstance>>,
    tx: Arc<broadcast::Sender<String>>,
}

// --- GPU Helper ---
fn get_gpu_metrics() -> (f32, u64, u64) {
    let output = Command::new("nvidia-smi")
        .args(&["--query-gpu=utilization.gpu,memory.used,memory.total", "--format=csv,noheader,nounits"])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            let parts: Vec<&str> = s.trim().split(',').collect();
            if parts.len() >= 3 {
                let usage = parts[0].trim().parse::<f32>().unwrap_or(0.0);
                let mem_used = parts[1].trim().parse::<u64>().unwrap_or(0);
                let mem_total = parts[2].trim().parse::<u64>().unwrap_or(0);
                return (usage, mem_used, mem_total);
            }
        }
    }
    (0.0, 0, 0)
}

// --- gRPC Server Implementation ---
#[tonic::async_trait]
impl OrchestratorService for Arc<AppState> {
    async fn report_node_status(&self, request: Request<NodeStatus>) -> Result<Response<Ack>, Status> {
        let req = request.into_inner();
        let node_id = req.node_name.to_uppercase();

        let stats = LocalNodeStats {
            name: node_id.clone(),
            cpu_usage: req.cpu_usage,
            ram_used: req.ram_used,
            ram_total: req.ram_total,
            gpu_usage: req.gpu_usage,
            gpu_mem_used: req.gpu_mem_used,
            gpu_mem_total: req.gpu_mem_total,
            last_seen: chrono::Utc::now().to_rfc3339(),
            status: "ONLINE".into(),
        };

        let mut nodes = self.nodes_cache.lock().await;
        nodes.insert(node_id, stats.clone());
        
        let update = serde_json::json!({ "type": "node_update", "data": stats });
        let _ = self.tx.send(update.to_string());

        Ok(Response::new(Ack { success: true }))
    }
}

// --- Main ---

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let node_name = env::var("NODE_NAME").unwrap_or_else(|_| "LOCAL".into()).to_uppercase();
    let upstream_url = env::var("UPSTREAM_ORCHESTRATOR_URL").ok();
    
    info!("🕹️ Sentiric Orchestrator v0.6.1 (Stable Monitor) | Node: {}", node_name);

    let docker = Docker::connect_with_local_defaults().expect("Docker connection failed");
    let (tx, _) = broadcast::channel::<String>(1000);

    let auto_pilot_env = env::var("AUTO_PILOT_SERVICES").unwrap_or_default();
    let mut initial_config = HashMap::new();
    for svc in auto_pilot_env.split(',') {
        if !svc.trim().is_empty() { initial_config.insert(svc.trim().to_string(), true); }
    }

    let state = Arc::new(AppState {
        docker: docker.clone(),
        auto_pilot_config: Mutex::new(initial_config),
        nodes_cache: Mutex::new(HashMap::new()),
        services_cache: Mutex::new(Vec::new()),
        tx: Arc::new(tx),
    });

    // 1. MONITOR TASK (Kendi Metriklerini Topla)
    let monitor_state = state.clone();
    let monitor_node = node_name.clone();
    let monitor_upstream = upstream_url.clone();
    
    tokio::spawn(async move {
        let mut sys = System::new_all();
        loop {
            // [FIX]: sysinfo 0.31 uyumlu API çağrıları
            sys.refresh_cpu_all(); 
            sys.refresh_memory();
            
            // [FIX]: global_cpu_usage() doğrudan f32 döner
            let cpu_usage = sys.global_cpu_usage(); 
            let ram_used = sys.used_memory() / 1024 / 1024;
            let ram_total = sys.total_memory() / 1024 / 1024;

            let (gpu_usage, gpu_mem_used, gpu_mem_total) = get_gpu_metrics();

            let stats = LocalNodeStats {
                name: monitor_node.clone(),
                cpu_usage,
                ram_used,
                ram_total,
                gpu_usage,
                gpu_mem_used,
                gpu_mem_total,
                last_seen: chrono::Utc::now().to_rfc3339(),
                status: "ONLINE".into(),
            };

            // A. Kendini Kaydet
            {
                let mut nodes = monitor_state.nodes_cache.lock().await;
                nodes.insert(monitor_node.clone(), stats.clone());
            }
            let update = serde_json::json!({ "type": "node_update", "data": stats.clone() });
            let _ = monitor_state.tx.send(update.to_string());

            // B. Upstream'e Gönder
            if let Some(url) = &monitor_upstream {
                match OrchestratorServiceClient::connect(url.clone()).await {
                    Ok(mut client) => {
                        let req = NodeStatus {
                            node_name: stats.name,
                            cpu_usage: stats.cpu_usage,
                            ram_used: stats.ram_used,
                            ram_total: stats.ram_total,
                            gpu_usage: stats.gpu_usage,
                            gpu_mem_used: stats.gpu_mem_used,
                            gpu_mem_total: stats.gpu_mem_total,
                            timestamp: stats.last_seen,
                            status: "ONLINE".into(),
                        };
                        if let Err(e) = client.report_node_status(req).await {
                            debug!("Upstream reporting failed: {}", e);
                        }
                    },
                    Err(e) => {
                        // Bağlantı hatasını debug seviyesinde logla, spam yapmasın
                        debug!("Failed to connect to upstream orchestrator: {}", e);
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // 2. Service Scanner
    let scanner_state = state.clone();
    let scanner_node = node_name.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            if let Ok(containers) = scanner_state.docker.list_containers(Some(ListContainersOptions::<String> { all: true, ..Default::default() })).await {
                let mut services = Vec::new();
                let ap_guard = scanner_state.auto_pilot_config.lock().await;

                for c in containers {
                    let name = c.names.unwrap_or_default().first().cloned().unwrap_or_default().replace("/", "");
                    if name.is_empty() || name.contains("orchestrator") { continue; }
                    let image_id = c.image_id.unwrap_or_default().replace("sha256:", "");
                    let short_id = if image_id.len() > 12 { image_id[0..12].to_string() } else { image_id };

                    services.push(ServiceInstance {
                        name: name.clone(),
                        image: c.image.unwrap_or_default(),
                        status: c.status.unwrap_or_default(),
                        short_id,
                        last_sync: chrono::Utc::now().format("%H:%M:%S").to_string(),
                        auto_pilot: *ap_guard.get(&name).unwrap_or(&false),
                        node: scanner_node.clone(),
                    });
                }
                services.sort_by(|a, b| a.name.cmp(&b.name));
                {
                    let mut cache = scanner_state.services_cache.lock().await;
                    *cache = services.clone();
                }
                let update = serde_json::json!({ "type": "services_update", "data": services });
                let _ = scanner_state.tx.send(update.to_string());
            }
        }
    });

    // 3. Node Watchdog
    let watchdog_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            let mut nodes = watchdog_state.nodes_cache.lock().await;
            let now = chrono::Utc::now();
            let mut changed = false;

            for (_, node) in nodes.iter_mut() {
                if let Ok(last) = chrono::DateTime::parse_from_rfc3339(&node.last_seen) {
                    let last_utc = last.with_timezone(&chrono::Utc);
                    if (now - last_utc).num_seconds() > 30 {
                        if node.status == "ONLINE" { node.status = "OFFLINE".to_string(); changed = true; }
                    } else if node.status == "OFFLINE" {
                        node.status = "ONLINE".to_string(); changed = true;
                    }
                }
            }
            if changed {
                let list: Vec<LocalNodeStats> = nodes.values().cloned().collect();
                let update = serde_json::json!({ "type": "nodes_list_update", "data": list });
                let _ = watchdog_state.tx.send(update.to_string());
            }
        }
    });

    // 4. Servers
    let app = Router::new()
        .route("/", get(|| async { Html(include_str!("index.html")) }))
        .route("/ws", get(ws_handler))
        .route("/api/status", get(status_api_handler))
        .route("/api/nodes", get(nodes_api_handler))
        .route("/api/update", post(manual_update_handler))
        .route("/api/toggle-autopilot", post(toggle_autopilot_handler))
        .with_state(state.clone());

    let http_port = 11080;
    let grpc_port = 11081;

    let grpc_state = state.clone();
    tokio::spawn(async move {
        info!("🔗 Orchestrator gRPC Active: 0.0.0.0:{}", grpc_port);
        let addr = format!("0.0.0.0:{}", grpc_port).parse().unwrap();
        // [FIX]: Hata logu eklendi, unwrap kaldırıldı
        if let Err(e) = tonic::transport::Server::builder()
            .add_service(OrchestratorServiceServer::new(grpc_state))
            .serve(addr).await 
        {
            error!("gRPC Server Error: {}", e);
        }
    });

    info!("🚀 Orchestrator UI: http://localhost:{}", http_port);
    let addr = SocketAddr::from(([0, 0, 0, 0], http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn perform_update(docker: &Docker, svc_name: &str) -> Result<String, String> {
    info!("🔄 Performing update for: {}", svc_name);
    let inspect = docker.inspect_container(svc_name, None).await.map_err(|e| e.to_string())?;
    let image_name = inspect.config.as_ref().and_then(|c| c.image.clone()).unwrap_or_default();
    
    let mut pull_stream = docker.create_image(
        Some(CreateImageOptions { from_image: image_name.clone(), ..Default::default() }),
        None, None
    );
    while let Some(res) = pull_stream.next().await {
        if let Err(e) = res { return Err(format!("Pull failed: {}", e)); }
    }

    let config = Config {
        image: Some(image_name),
        env: inspect.config.as_ref().and_then(|c| c.env.clone()),
        labels: inspect.config.as_ref().and_then(|c| c.labels.clone()),
        host_config: inspect.host_config.clone(),
        networking_config: inspect.network_settings.as_ref().and_then(|n| {
            Some(NetworkingConfig { endpoints_config: n.networks.clone().unwrap_or_default() })
        }),
        ..Default::default()
    };

    let _ = docker.stop_container(svc_name, Some(StopContainerOptions { t: 5 })).await;
    let _ = docker.remove_container(svc_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;

    docker.create_container(Some(CreateContainerOptions { name: svc_name.to_string(), platform: None }), config)
        .await.map_err(|e| format!("Create failed: {}", e))?;
    
    docker.start_container(svc_name, None::<StartContainerOptions<String>>)
        .await.map_err(|e| format!("Start failed: {}", e))?;

    Ok(format!("{} updated successfully.", svc_name))
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

async fn status_api_handler(State(state): State<Arc<AppState>>) -> Json<Vec<ServiceInstance>> {
    let guard = state.services_cache.lock().await;
    Json(guard.clone())
}

async fn nodes_api_handler(State(state): State<Arc<AppState>>) -> Json<Vec<LocalNodeStats>> {
    let guard = state.nodes_cache.lock().await;
    let list: Vec<LocalNodeStats> = guard.values().cloned().collect();
    Json(list)
}

async fn manual_update_handler(State(state): State<Arc<AppState>>, Query(params): Query<ActionParams>) -> impl IntoResponse {
    match perform_update(&state.docker, &params.service).await {
        Ok(msg) => (axum::http::StatusCode::OK, msg),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn toggle_autopilot_handler(State(state): State<Arc<AppState>>, Json(payload): Json<ToggleParams>) -> Json<bool> {
    let mut guard = state.auto_pilot_config.lock().await;
    guard.insert(payload.service, payload.enabled);
    Json(payload.enabled)
}