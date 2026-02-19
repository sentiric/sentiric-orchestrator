mod config;
mod core;
mod adapters;
mod api;

use std::{sync::Arc, collections::HashMap, time::Duration};
use tokio::sync::{Mutex, broadcast};
use tracing::{info, error};
use bollard::container::{ListContainersOptions, StatsOptions};
use futures_util::StreamExt;

use crate::config::AppConfig;
use crate::adapters::docker::DockerAdapter;
use crate::adapters::system::SystemMonitor;
use crate::core::domain::ServiceInstance;

// Shared State
pub struct AppState {
    pub docker: DockerAdapter,
    pub auto_pilot_config: Mutex<HashMap<String, bool>>,
    pub services_cache: Mutex<HashMap<String, ServiceInstance>>,
    pub tx: Arc<broadcast::Sender<String>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cfg = AppConfig::load();
    
    info!("💠 SENTIRIC NEXUS v5.0 Booting...");
    info!("🔧 Node: {} | Host: {}", cfg.node_name, cfg.host);

    // Channels
    let (tx, _) = broadcast::channel::<String>(100);
    let tx = Arc::new(tx);

    // Adapters
    let docker = DockerAdapter::new(&cfg.docker_socket)?;
    let mut sys_mon = SystemMonitor::new(cfg.node_name.clone());

    // State Init
    let mut initial_ap = HashMap::new();
    for svc in cfg.auto_pilot_services { initial_ap.insert(svc, true); }

    let state = Arc::new(AppState {
        docker: docker.clone(),
        auto_pilot_config: Mutex::new(initial_ap),
        services_cache: Mutex::new(HashMap::new()),
        tx: tx.clone(),
    });

    // 1. SYSTEM MONITOR LOOP
    let mon_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            let stats = sys_mon.snapshot();
            let _ = mon_tx.send(serde_json::json!({ "type": "node_update", "data": stats }).to_string());
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // 2. DOCKER SCAN LOOP
    let scan_state = state.clone();
    let scan_node = cfg.node_name.clone();
    tokio::spawn(async move {
        let client = scan_state.docker.get_client();
        loop {
            if let Ok(containers) = client.list_containers(Some(ListContainersOptions::<String> { all: true, ..Default::default() })).await {
                let ap_guard = scan_state.auto_pilot_config.lock().await;
                let mut cache = scan_state.services_cache.lock().await;
                let mut list = Vec::new();

                for c in containers {
                    let name = c.names.unwrap_or_default().first().cloned().unwrap_or_default().replace("/", "");
                    if name.is_empty() || name.contains("orchestrator") { continue; }

                    // Quick Stats (CPU/Mem)
                    let mut cpu = 0.0;
                    let mut mem = 0;
                    // Detaylı stats logic'i burada basitleştirildi, prod için optimize edilmeli
                    
                    let svc = ServiceInstance {
                        name: name.clone(),
                        image: c.image.unwrap_or_default(),
                        status: c.status.unwrap_or_default(),
                        short_id: c.id.unwrap_or_default().chars().take(12).collect(),
                        auto_pilot: *ap_guard.get(&name).unwrap_or(&false),
                        node: scan_node.clone(),
                        cpu_usage: cpu,
                        mem_usage: mem,
                        has_gpu: name.contains("llm") || name.contains("stt"),
                    };
                    
                    cache.insert(name, svc.clone());
                    list.push(svc);
                }
                
                let _ = scan_state.tx.send(serde_json::json!({ "type": "services_update", "data": list }).to_string());
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // 3. SERVER START
    let app = api::routes::create_router(state.clone());
    let addr = format!("{}:{}", cfg.host, cfg.http_port);
    info!("🚀 Nexus Dashboard: http://{}", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}