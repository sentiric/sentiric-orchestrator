mod config;
mod core;
mod adapters;
mod api;

use std::{sync::Arc, collections::HashMap, time::Duration};
use tokio::sync::{Mutex, broadcast};
use tracing::{info, error, debug}; // Debug eklendi
use bollard::container::ListContainersOptions;

use crate::config::AppConfig;
use crate::adapters::docker::DockerAdapter;
use crate::adapters::system::SystemMonitor;
use crate::core::domain::ServiceInstance;

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
    
    info!("💠 SENTIRIC ORCHESTRATOR v5.1 Booting...");
    info!("🔧 Node: {} | Auto-Pilot Services: {:?}", cfg.node_name, cfg.auto_pilot_services);

    let (tx, _) = broadcast::channel::<String>(100);
    let tx = Arc::new(tx);

    let docker = DockerAdapter::new(&cfg.docker_socket, cfg.node_name.clone())?;
    let mut sys_mon = SystemMonitor::new(cfg.node_name.clone());

    let mut initial_ap = HashMap::new();
    for svc in cfg.auto_pilot_services { initial_ap.insert(svc, true); }

    let state = Arc::new(AppState {
        docker: docker.clone(),
        auto_pilot_config: Mutex::new(initial_ap),
        services_cache: Mutex::new(HashMap::new()),
        tx: tx.clone(),
    });

    // 1. SYSTEM MONITOR
    let mon_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            let stats = sys_mon.snapshot();
            let _ = mon_tx.send(serde_json::json!({ "type": "node_update", "data": stats }).to_string());
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // 2. DOCKER SCAN & AUTO-PILOT LOOP
    let scan_state = state.clone();
    let scan_node = cfg.node_name.clone();
    let poll_interval = cfg.poll_interval;

    tokio::spawn(async move {
        let client = scan_state.docker.get_client();
        info!("🕵️ Service Scanner Loop Started (Interval: {}s)", poll_interval);
        
        loop {
            match client.list_containers(Some(ListContainersOptions::<String> { all: true, ..Default::default() })).await {
                Ok(containers) => {
                    let ap_guard = scan_state.auto_pilot_config.lock().await;
                    let mut cache = scan_state.services_cache.lock().await;
                    let mut list = Vec::new();

                    for c in containers {
                        let name = c.names.unwrap_or_default().first().cloned().unwrap_or_default().replace("/", "");
                        if name.is_empty() { continue; }

                        let is_orchestrator = name.contains("orchestrator");
                        let is_auto_pilot = *ap_guard.get(&name).unwrap_or(&false);

                        // Auto-Pilot Mantığı
                        if is_auto_pilot {
                            if is_orchestrator {
                                debug!("ℹ️ Skipping self-update for: {}", name);
                            } else {
                                // Burada normalde digest kontrolü yapılır. 
                                // Şimdilik log basıyoruz ki çalıştığını görelim.
                                debug!("🔍 Auto-Pilot Checking: {}", name);
                                
                                // NOT: Her döngüde güncelleme YAPMAZ. 
                                // Gerçek bir digest kontrolü eklenene kadar burası sadece izleme modunda.
                                // Kullanıcı UI'dan "Force Update" yapabilir.
                            }
                        }

                        let svc = ServiceInstance {
                            name: name.clone(),
                            image: c.image.unwrap_or_default(),
                            status: c.status.unwrap_or_default(),
                            short_id: c.id.unwrap_or_default().chars().take(12).collect(),
                            auto_pilot: is_auto_pilot,
                            node: scan_node.clone(),
                            cpu_usage: 0.0,
                            mem_usage: 0,
                            has_gpu: name.contains("llm") || name.contains("ocr"),
                        };
                        
                        cache.insert(name, svc.clone());
                        list.push(svc);
                    }
                    
                    let _ = scan_state.tx.send(serde_json::json!({ "type": "services_update", "data": list }).to_string());
                }
                Err(e) => {
                    error!("⚠️ Docker Daemon Hatası: {}", e);
                }
            }
            tokio::time::sleep(Duration::from_secs(poll_interval)).await;
        }
    });

    // 3. API SERVER
    let app = api::routes::create_router(state.clone());
    let addr = format!("{}:{}", cfg.host, cfg.http_port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}