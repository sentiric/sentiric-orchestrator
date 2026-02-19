mod config;
mod core;
mod adapters;
mod api;

use std::{sync::Arc, collections::HashMap, time::Duration};
use tokio::sync::{Mutex, broadcast};
use tracing::{info, error, warn};
use bollard::container::ListContainersOptions;

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
    
    info!("💠 SENTIRIC ORCHESTRATOR v5.1 (Iron Core) Booting...");
    info!("🔧 Node: {} | Host: {}:{}", cfg.node_name, cfg.host, cfg.http_port);

    // Channels
    let (tx, _) = broadcast::channel::<String>(100);
    let tx = Arc::new(tx);

    // Adapters
    // Docker bağlantısı başta başarısız olsa bile program çökmesin, retry yapabilsin diye burada unwrap yerine match kullanıyoruz ama
    // Adapter içinde unwrap var. Basitlik için panic yapıp Docker'ın restart etmesini beklemek (container orchestrator pattern) daha doğrudur.
    let docker = DockerAdapter::new(&cfg.docker_socket, cfg.node_name.clone())?;
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
            // Hata olursa (kanal dolu vb) logla ama çökme
            if let Err(_) = mon_tx.send(serde_json::json!({ "type": "node_update", "data": stats }).to_string()) {
                // Kanal hatası (receiver yoksa) normaldir
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // 2. DOCKER SCAN LOOP & AUTO-PILOT
    let scan_state = state.clone();
    let scan_node = cfg.node_name.clone();
    let poll_interval = cfg.poll_interval; // Config'den al

    tokio::spawn(async move {
        let client = scan_state.docker.get_client();
        loop {
            // Docker'a ulaşamazsak panik yapma, log bas ve bekle
            match client.list_containers(Some(ListContainersOptions::<String> { all: true, ..Default::default() })).await {
                Ok(containers) => {
                    let ap_guard = scan_state.auto_pilot_config.lock().await;
                    let mut cache = scan_state.services_cache.lock().await;
                    let mut list = Vec::new();

                    for c in containers {
                        let name = c.names.unwrap_or_default().first().cloned().unwrap_or_default().replace("/", "");
                        if name.is_empty() { continue; }
                        // Kendini listede gösterme (Opsiyonel, kafa karıştırmaması için)
                        if name.contains("orchestrator") { continue; }

                        let status = c.status.unwrap_or_default();
                        let state_str = c.state.unwrap_or_default(); // running, exited...

                        let svc = ServiceInstance {
                            name: name.clone(),
                            image: c.image.unwrap_or_default(),
                            status: format!("{} ({})", status, state_str),
                            short_id: c.id.unwrap_or_default().chars().take(12).collect(),
                            auto_pilot: *ap_guard.get(&name).unwrap_or(&false),
                            node: scan_node.clone(),
                            cpu_usage: 0.0, // İlerde stats API ile doldurulabilir
                            mem_usage: 0,
                            has_gpu: name.contains("llm") || name.contains("stt") || name.contains("ocr"),
                        };
                        
                        cache.insert(name, svc.clone());
                        list.push(svc);
                    }
                    
                    let _ = scan_state.tx.send(serde_json::json!({ "type": "services_update", "data": list }).to_string());
                }
                Err(e) => {
                    error!("⚠️ Docker Daemon Unreachable: {}", e);
                    let _ = scan_state.tx.send(serde_json::json!({ 
                        "type": "alert", 
                        "data": { "level": "error", "message": "Docker Daemon Unreachable!" } 
                    }).to_string());
                }
            }
            tokio::time::sleep(Duration::from_secs(poll_interval)).await;
        }
    });

    // 3. SERVER START (Graceful Shutdown ile)
    let app = api::routes::create_router(state.clone());
    let addr = format!("{}:{}", cfg.host, cfg.http_port);
    info!("🚀 Dashboard: http://{}", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    // Graceful shutdown sinyali
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("🛑 Shutting down gracefully...");
}