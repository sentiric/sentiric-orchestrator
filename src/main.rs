// sentiric-orchestrator/src/main.rs

use axum::{
    extract::{State, Query, ws::{Message, WebSocketUpgrade}},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use bollard::Docker;
use bollard::container::{
    ListContainersOptions, StopContainerOptions, RemoveContainerOptions, 
    Config, CreateContainerOptions, StartContainerOptions, StatsOptions
};
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;
use std::{env, sync::Arc, time::Duration, collections::HashMap, process::Command};
use tokio::sync::{Mutex, broadcast};
use tracing::info; // Sadece kullanılanlar
use serde::{Deserialize, Serialize};
use tonic::{Request, Response, Status};
use sysinfo::System; // [FIX]: CpuExt kaldırıldı

// Proto Modülü
pub mod orchestrator_proto {
    tonic::include_proto!("sentiric.orchestrator.v1");
}
use orchestrator_proto::orchestrator_service_server::{OrchestratorService, OrchestratorServiceServer};
use orchestrator_proto::orchestrator_service_client::OrchestratorServiceClient;
use orchestrator_proto::{NodeStatus, Ack};

lazy_static::lazy_static! {
    static ref METRIC_REGEX: regex::Regex = regex::Regex::new(r"CPU: (\d+\.?\d*)% \| RAM: (\d+)/(\d+)MB").unwrap();
}

// --- Veri Modelleri ---

#[derive(Serialize, Clone, Debug)]
struct ServiceInstance {
    name: String,
    image: String,
    status: String,
    short_id: String,
    auto_pilot: bool,
    node: String,
    cpu_usage: f64,
    mem_usage: u64,
    has_gpu: bool,
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
    services_cache: Mutex<HashMap<String, ServiceInstance>>,
    tx: Arc<broadcast::Sender<String>>,
}

// --- GPU Metrics Collector ---
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

        {
            let mut nodes = self.nodes_cache.lock().await;
            nodes.insert(node_id.clone(), stats.clone());
        }
        
        let _ = self.tx.send(serde_json::json!({ "type": "node_update", "data": stats }).to_string());
        Ok(Response::new(Ack { success: true }))
    }
}

// --- Lifecycle & Update Engine ---
async fn perform_update(docker: &Docker, svc_name: &str) -> anyhow::Result<String> {
    info!("🔄 [ENGINE] Atomic update starting for: {}", svc_name);
    
    let inspect = docker.inspect_container(svc_name, None).await?;
    let image_name = inspect.config.as_ref().and_then(|c| c.image.clone()).ok_or_else(|| anyhow::anyhow!("Image not found"))?;

    let mut pull_stream = docker.create_image(Some(CreateImageOptions { from_image: image_name.clone(), ..Default::default() }), None, None);
    while let Some(res) = pull_stream.next().await {
        if let Err(e) = res { return Err(anyhow::anyhow!("Pull fail: {}", e)); }
    }

    let config = Config {
        image: Some(image_name),
        env: inspect.config.as_ref().and_then(|c| c.env.clone()),
        labels: inspect.config.as_ref().and_then(|c| c.labels.clone()),
        host_config: inspect.host_config.clone(),
        networking_config: inspect.network_settings.as_ref().and_then(|n| {
            Some(bollard::container::NetworkingConfig { endpoints_config: n.networks.clone().unwrap_or_default() })
        }),
        ..Default::default()
    };

    let _ = docker.stop_container(svc_name, Some(StopContainerOptions { t: 10 })).await;
    let _ = docker.remove_container(svc_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
    docker.create_container(Some(CreateContainerOptions { name: svc_name.to_string(), platform: None }), config).await?;
    docker.start_container(svc_name, None::<StartContainerOptions<String>>).await?;

    Ok(format!("{} successfully updated.", svc_name))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let node_name = env::var("NODE_NAME").unwrap_or_else(|_| "CENTRAL-NODE".into()).to_uppercase();
    let upstream_url = env::var("UPSTREAM_ORCHESTRATOR_URL").ok();
    let auto_pilot_list = env::var("AUTO_PILOT_SERVICES").unwrap_or_default();
    let poll_interval = env::var("POLL_INTERVAL").unwrap_or_else(|_| "60".into()).parse::<u64>().unwrap_or(60);

    info!("🕹️ Sentiric Orchestrator Nexus v0.7.6 | Node: {} | AP: {}", node_name, auto_pilot_list);

    let docker = Docker::connect_with_local_defaults().expect("Docker socket fail");
    let (tx, _) = broadcast::channel::<String>(2000);
    let tx = Arc::new(tx);

    let mut initial_ap = HashMap::new();
    for s in auto_pilot_list.split(',') {
        if !s.trim().is_empty() { initial_ap.insert(s.trim().to_string(), true); }
    }

    let state = Arc::new(AppState {
        docker: docker.clone(),
        auto_pilot_config: Mutex::new(initial_ap),
        nodes_cache: Mutex::new(HashMap::new()),
        services_cache: Mutex::new(HashMap::new()),
        tx,
    });

    // 1. MONITORING TASK
    let mon_state = state.clone();
    let mon_node = node_name.clone();
    let mon_up = upstream_url.clone();
    tokio::spawn(async move {
        let mut sys = System::new_all();
        loop {
            // [FIX]: sysinfo 0.31 compatible calls
            sys.refresh_cpu_usage(); 
            sys.refresh_memory();
            
            let stats = LocalNodeStats {
                name: mon_node.clone(),
                cpu_usage: sys.global_cpu_usage(),
                ram_used: sys.used_memory() / 1024 / 1024,
                ram_total: sys.total_memory() / 1024 / 1024,
                gpu_usage: get_gpu_metrics().0,
                gpu_mem_used: get_gpu_metrics().1,
                gpu_mem_total: get_gpu_metrics().2,
                last_seen: chrono::Utc::now().to_rfc3339(),
                status: "ONLINE".into(),
            };

            { mon_state.nodes_cache.lock().await.insert(mon_node.clone(), stats.clone()); }
            let _ = mon_state.tx.send(serde_json::json!({ "type": "node_update", "data": stats.clone() }).to_string());

            if let Some(url) = &mon_up {
                if let Ok(mut client) = OrchestratorServiceClient::connect(url.clone()).await {
                    let req = NodeStatus {
                        node_name: stats.name, cpu_usage: stats.cpu_usage,
                        ram_used: stats.ram_used, ram_total: stats.ram_total,
                        gpu_usage: stats.gpu_usage, gpu_mem_used: stats.gpu_mem_used, gpu_mem_total: stats.gpu_mem_total,
                        timestamp: stats.last_seen, status: "ONLINE".into(),
                    };
                    let _ = client.report_node_status(req).await;
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // 2. SCANNER TASK
    let scan_state = state.clone();
    let scan_node = node_name.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            if let Ok(containers) = scan_state.docker.list_containers(Some(ListContainersOptions::<String> { all: true, ..Default::default() })).await {
                let ap_guard = scan_state.auto_pilot_config.lock().await;
                let mut svc_cache = scan_state.services_cache.lock().await;

                for c in containers {
                    let name = c.names.unwrap_or_default().first().cloned().unwrap_or_default().replace("/", "");
                    if name.is_empty() || name.contains("orchestrator") { continue; }
                    
                    let mut cpu = 0.0; let mut mem = 0u64;
                    // [FIX]: next().await returns Option<Result<T, E>>
                    if let Some(Ok(s)) = scan_state.docker.stats(&name, Some(StatsOptions { stream: false, one_shot: true })).next().await {
                        let cpu_delta = (s.cpu_stats.cpu_usage.total_usage - s.precpu_stats.cpu_usage.total_usage) as f64;
                        let sys_delta = (s.cpu_stats.system_cpu_usage.unwrap_or(0) - s.precpu_stats.system_cpu_usage.unwrap_or(0)) as f64;
                        if sys_delta > 0.0 { cpu = (cpu_delta / sys_delta) * s.cpu_stats.online_cpus.unwrap_or(1) as f64 * 100.0; }
                        mem = s.memory_stats.usage.unwrap_or(0) / 1024 / 1024;
                    }

                    svc_cache.insert(name.clone(), ServiceInstance {
                        name: name.clone(),
                        image: c.image.unwrap_or_default(),
                        status: c.status.unwrap_or_default(),
                        short_id: c.image_id.unwrap_or_default().replace("sha256:", "").chars().take(12).collect(),
                        auto_pilot: *ap_guard.get(&name).unwrap_or(&false),
                        node: scan_node.clone(),
                        cpu_usage: cpu,
                        mem_usage: mem,
                        has_gpu: name.contains("llm") || name.contains("stt") || name.contains("tts"),
                    });
                }
                let list: Vec<ServiceInstance> = svc_cache.values().cloned().collect();
                let _ = scan_state.tx.send(serde_json::json!({ "type": "services_update", "data": list }).to_string());
            }
        }
    });

    // 3. AUTO-PILOT TASK
    let ap_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(poll_interval));
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            interval.tick().await;
            let targets: Vec<String>;
            {
                let guard = ap_state.auto_pilot_config.lock().await;
                targets = guard.iter().filter(|(_, &v)| v).map(|(k, _)| k.clone()).collect();
            }

            for name in targets {
                if let Ok(inspect) = ap_state.docker.inspect_container(&name, None).await {
                    let current_id = inspect.image.unwrap_or_default();
                    let image_name = inspect.config.and_then(|c| c.image).unwrap_or_default();
                    
                    let mut stream = ap_state.docker.create_image(Some(CreateImageOptions { from_image: image_name.clone(), ..Default::default() }), None, None);
                    while let Some(_) = stream.next().await {}

                    if let Ok(new_img) = ap_state.docker.inspect_image(&image_name).await {
                        if current_id != new_img.id.unwrap_or_default() {
                            info!("🚀 [AUTO-PILOT] New version found for {}. Updating...", name);
                            let _ = perform_update(&ap_state.docker, &name).await;
                        }
                    }
                }
            }
        }
    });

    // 4. SERVERS
    let app = Router::new()
        .route("/", get(|| async { Html(include_str!("index.html")) }))
        .route("/ws", get(|ws: WebSocketUpgrade, State(st): State<Arc<AppState>>| async move {
            ws.on_upgrade(|socket| async move {
                let mut rx = st.tx.subscribe();
                let mut socket = socket;
                while let Ok(m) = rx.recv().await { if socket.send(Message::Text(m.into())).await.is_err() { break; } }
            })
        }))
        .route("/api/status", get(|State(st): State<Arc<AppState>>| async move {
            Json(st.services_cache.lock().await.values().cloned().collect::<Vec<_>>())
        }))
        .route("/api/nodes", get(|State(st): State<Arc<AppState>>| async move {
            Json(st.nodes_cache.lock().await.values().cloned().collect::<Vec<_>>())
        }))
        .route("/api/update", post(|State(st): State<Arc<AppState>>, Query(p): Query<ActionParams>| async move {
            match perform_update(&st.docker, &p.service).await {
                Ok(m) => (axum::http::StatusCode::OK, m).into_response(),
                Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        }))
        .route("/api/toggle-autopilot", post(|State(st): State<Arc<AppState>>, Json(p): Json<ToggleParams>| async move {
            st.auto_pilot_config.lock().await.insert(p.service, p.enabled);
            Json(p.enabled)
        }))
        .with_state(state.clone());

    let grpc_state = state.clone();
    tokio::spawn(async move {
        let addr = "0.0.0.0:11081".parse().unwrap();
        let _ = tonic::transport::Server::builder().add_service(OrchestratorServiceServer::new(grpc_state)).serve(addr).await;
    });

    info!("🚀 Orchestrator Nexus operational on http://0.0.0.0:11080");
    axum::serve(tokio::net::TcpListener::bind("0.0.0.0:11080").await?, app).await?;
    Ok(())
}