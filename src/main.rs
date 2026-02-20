mod config;
mod core;
mod adapters;
mod api;

use std::{sync::Arc, collections::HashMap, time::Duration};
use tokio::sync::{Mutex, broadcast};
use tracing::{info, error, warn}; 
use bollard::container::ListContainersOptions;
use reqwest::Client; // YENİ: Upstream için

use crate::config::AppConfig;
use crate::adapters::docker::DockerAdapter;
use crate::adapters::system::SystemMonitor;
use crate::core::domain::{ServiceInstance, NodeStats};

// CPU Delta Hesaplama Cache
struct CpuStatsCache {
    cpu_usage: u64,
    system_usage: u64,
}

pub struct AppState {
    pub docker: DockerAdapter,
    pub auto_pilot_config: Mutex<HashMap<String, bool>>,
    pub services_cache: Mutex<HashMap<String, ServiceInstance>>,
    pub node_stats_cache: Mutex<NodeStats>, // YENİ: Stats'ı tutmak için (Upstream'e göndereceğiz)
    pub tx: Arc<broadcast::Sender<String>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cfg = AppConfig::load();
    
    info!("💠 SENTIRIC ORCHESTRATOR v5.3 (Upstream & AI) Booting...");
    info!("🔧 Node: {} | Upstream: {:?}", cfg.node_name, cfg.upstream_url);

    let (tx, _) = broadcast::channel::<String>(100);
    let tx = Arc::new(tx);

    let docker = DockerAdapter::new(&cfg.docker_socket, cfg.node_name.clone())?;
    let mut sys_mon = SystemMonitor::new(cfg.node_name.clone());

    let mut initial_ap = HashMap::new();
    for svc in &cfg.auto_pilot_services { initial_ap.insert(svc.clone(), true); }

    let state = Arc::new(AppState {
        docker: docker.clone(),
        auto_pilot_config: Mutex::new(initial_ap),
        services_cache: Mutex::new(HashMap::new()),
        node_stats_cache: Mutex::new(NodeStats::default()),
        tx: tx.clone(),
    });

    // 1. SYSTEM MONITOR (Host Stats)
    let mon_state = state.clone();
    let mon_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            let stats = sys_mon.snapshot();
            
            // Cache güncelle (Upstream için)
            let mut node_cache = mon_state.node_stats_cache.lock().await;
            *node_cache = stats.clone();
            drop(node_cache);

            // UI'a gönder
            let _ = mon_tx.send(serde_json::json!({ "type": "node_update", "data": stats }).to_string());
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    // 2. DOCKER SCAN & METRICS & AUTO-PILOT LOOP
    let scan_state = state.clone();
    let scan_node = cfg.node_name.clone();
    let poll_interval = cfg.poll_interval;

    tokio::spawn(async move {
        let client = scan_state.docker.get_client();
        info!("🕵️ Service Scanner & Telemetry Engine Started (Interval: {}s)", poll_interval);
        
        let mut tick_count = 0;
        let update_check_ticks = 12; // 60sn
        let mut cpu_cache: HashMap<String, CpuStatsCache> = HashMap::new();

        loop {
            tick_count += 1;
            let do_update_check = tick_count >= update_check_ticks;
            if do_update_check { tick_count = 0; }

            match client.list_containers(Some(ListContainersOptions::<String> { all: true, ..Default::default() })).await {
                Ok(containers) => {
                    let ap_guard = scan_state.auto_pilot_config.lock().await;
                    let mut cache = scan_state.services_cache.lock().await;
                    let mut list = Vec::new();

                    for c in containers {
                        let name = c.names.unwrap_or_default().first().cloned().unwrap_or_default().replace("/", "");
                        if name.is_empty() { continue; }

                        // DÜZELTME: Başına _ eklendi
                        let _is_orchestrator = name.contains("orchestrator");
                        let is_auto_pilot = *ap_guard.get(&name).unwrap_or(&false);
                        let container_id = c.id.clone().unwrap_or_default();
                        
                        // Telemetry
                        let mut cpu_percent = 0.0;
                        let mut mem_usage_mb = 0;

                        if c.status.clone().unwrap_or_default().to_lowercase().contains("up") {
                            match scan_state.docker.get_container_stats(&container_id).await {
                                Ok(stats) => {
                                    // RAM
                                    let mem = &stats.memory_stats;
                                    mem_usage_mb = mem.usage.unwrap_or(0) / 1024 / 1024;

                                    // CPU
                                    let cpu = &stats.cpu_stats;
                                    let pre_cpu = &stats.precpu_stats;
                                    let cpu_total = cpu.cpu_usage.total_usage;
                                    let system_total = cpu.system_cpu_usage.unwrap_or(0);
                                    
                                    let (prev_cpu_total, prev_system_total) = if let Some(cached) = cpu_cache.get(&container_id) {
                                        (cached.cpu_usage, cached.system_usage)
                                    } else {
                                        (pre_cpu.cpu_usage.total_usage, pre_cpu.system_cpu_usage.unwrap_or(0))
                                    };

                                    if system_total > prev_system_total && cpu_total > prev_cpu_total {
                                        let cpu_delta = (cpu_total - prev_cpu_total) as f64;
                                        let system_delta = (system_total - prev_system_total) as f64;
                                        let online_cpus = cpu.online_cpus.unwrap_or(1) as f64;
                                        if system_delta > 0.0 { cpu_percent = (cpu_delta / system_delta) * online_cpus * 100.0; }
                                    }
                                    cpu_cache.insert(container_id.clone(), CpuStatsCache { cpu_usage: cpu_total, system_usage: system_total });
                                },
                                Err(_) => { }
                            }
                        } else {
                            cpu_cache.remove(&container_id);
                        }

                        // Auto-Pilot Update
                        if is_auto_pilot && do_update_check {
                            let docker_adapter = &scan_state.docker;
                            let svc_name = name.clone();
                            let d_adapter = docker_adapter.clone();
                            tokio::spawn(async move {
                                match d_adapter.check_and_update_service(&svc_name).await {
                                    Ok(updated) => if updated { info!("♻️ Auto-Pilot Action Completed: {}", svc_name) },
                                    Err(e) => error!("⚠️ Auto-Pilot Failed ({}): {}", svc_name, e),
                                }
                            });
                        }

                        let svc = ServiceInstance {
                            name: name.clone(),
                            image: c.image.unwrap_or_default(),
                            status: c.status.unwrap_or_default(),
                            short_id: container_id.chars().take(12).collect(),
                            auto_pilot: is_auto_pilot,
                            node: scan_node.clone(),
                            cpu_usage: cpu_percent,
                            mem_usage: mem_usage_mb,
                            has_gpu: name.contains("llm") || name.contains("ocr") || name.contains("media"),
                        };
                        
                        cache.insert(name, svc.clone());
                        list.push(svc);
                    }
                    
                    let _ = scan_state.tx.send(serde_json::json!({ "type": "services_update", "data": list }).to_string());
                }
                Err(e) => { error!("⚠️ Docker Daemon Hatası: {}", e); }
            }
            tokio::time::sleep(Duration::from_secs(poll_interval)).await;
        }
    });

    // 4. UPSTREAM SYNC LOOP (YENİ: Ana Merkeze Veri Akışı)
    if let Some(upstream_url) = cfg.upstream_url {
        let up_state = state.clone();
        let http_client = Client::new();
        let node_name = cfg.node_name.clone();

        info!("📡 Upstream Uplink Activated: Target -> {}", upstream_url);

        tokio::spawn(async move {
            loop {
                // Verileri Topla
                let svcs: Vec<ServiceInstance> = up_state.services_cache.lock().await.values().cloned().collect();
                let stats: NodeStats = up_state.node_stats_cache.lock().await.clone();

                let payload = serde_json::json!({
                    "node": node_name,
                    "stats": stats,
                    "services": svcs,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                });

                // Gönder
                match http_client.post(&upstream_url).json(&payload).send().await {
                    Ok(resp) => {
                        if !resp.status().is_success() {
                            warn!("⚠️ Upstream rejected data: {}", resp.status());
                        }
                    },
                    Err(e) => {
                        warn!("⚠️ Upstream Connection Failed: {}", e);
                    }
                }

                tokio::time::sleep(Duration::from_secs(10)).await; // 10 saniyede bir raporla
            }
        });
    }

    // 5. API SERVER
    let app = api::routes::create_router(state.clone());
    let addr = format!("{}:{}", cfg.host, cfg.http_port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}