mod config;
mod core;
mod adapters;
mod api;
mod telemetry; // YENİ

use std::{sync::Arc, collections::HashMap, time::Duration};
use tokio::sync::{Mutex, broadcast};
use tracing::{info, error, warn, debug}; 
use tracing_subscriber::{fmt, prelude::*, EnvFilter, Registry};
use bollard::container::ListContainersOptions;
use reqwest::Client;

use crate::config::AppConfig;
use crate::adapters::docker::DockerAdapter;
use crate::adapters::system::SystemMonitor;
use crate::core::domain::{ServiceInstance, NodeStats, ClusterReport};
use crate::telemetry::SutsFormatter;

struct CpuStatsCache {
    cpu_usage: u64,
    system_usage: u64,
}

pub struct AppState {
    pub docker: DockerAdapter,
    pub auto_pilot_config: Mutex<HashMap<String, bool>>,
    pub services_cache: Mutex<HashMap<String, ServiceInstance>>,
    pub node_stats_cache: Mutex<NodeStats>,
    pub cluster_cache: Mutex<HashMap<String, ClusterReport>>, 
    pub tx: Arc<broadcast::Sender<String>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = AppConfig::load();

    // --- SUTS v4.0 LOGGING SETUP ---
    let rust_log_env = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let env_filter = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(&rust_log_env))?;
    let subscriber = Registry::default().with(env_filter);
    
    // Config'de log formatı yoksa varsayılan olarak json (prod için) veya text
    let log_format = std::env::var("LOG_FORMAT").unwrap_or_else(|_| "json".to_string());

    if log_format == "json" {
        let suts_formatter = SutsFormatter::new(
            "orchestrator-service".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
            cfg.env.clone(),
            cfg.node_name.clone(),
        );
        subscriber.with(fmt::layer().event_format(suts_formatter)).init();
    } else {
        subscriber.with(fmt::layer().compact()).init();
    }

    info!(
        event = "SYSTEM_STARTUP",
        service.version = env!("CARGO_PKG_VERSION"),
        node.name = %cfg.node_name,
        mode = if cfg.upstream_url.is_some() { "EDGE" } else { "MASTER" },
        "💠 SENTIRIC ORCHESTRATOR v5.4 (HIVE MIND) Booting..."
    );

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
        cluster_cache: Mutex::new(HashMap::new()),
        tx: tx.clone(),
    });

    // 1. SYSTEM MONITOR
    let mon_state = state.clone();
    let mon_node = cfg.node_name.clone();
    let mon_tx = tx.clone();
    
    tokio::spawn(async move {
        loop {
            let stats = sys_mon.snapshot();
            
            // [SUTS]: Periyodik metrik logu (Sadece önemli değişimlerde loglanabilir, şimdilik debug)
            debug!(
                event = "SYSTEM_METRICS_COLLECTED",
                cpu.usage = stats.cpu_usage,
                ram.used_mb = stats.ram_used,
                gpu.usage = stats.gpu_usage,
                "Sistem metrikleri toplandı"
            );

            let mut node_cache = mon_state.node_stats_cache.lock().await;
            *node_cache = stats.clone();
            drop(node_cache);

            // Local veriyi cluster cache'e ekle
            let svcs = mon_state.services_cache.lock().await.values().cloned().collect();
            let report = ClusterReport {
                node: mon_node.clone(),
                stats: stats,
                services: svcs,
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
            
            mon_state.cluster_cache.lock().await.insert(mon_node.clone(), report);
            
            // Cluster update gönder
            let cluster_map = mon_state.cluster_cache.lock().await.clone();
            let _ = mon_tx.send(serde_json::json!({ "type": "cluster_update", "data": cluster_map }).to_string());
            
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    // 2. DOCKER SCAN
    let scan_state = state.clone();
    let scan_node = cfg.node_name.clone();
    let poll_interval = cfg.poll_interval;

    tokio::spawn(async move {
        let client = scan_state.docker.get_client();
        let mut tick_count = 0;
        let update_check_ticks = 12; 
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

                        let is_auto_pilot = *ap_guard.get(&name).unwrap_or(&false);
                        let container_id = c.id.clone().unwrap_or_default();
                        
                        let mut cpu_percent = 0.0;
                        let mut mem_usage_mb = 0;

                        if c.status.clone().unwrap_or_default().to_lowercase().contains("up") {
                            match scan_state.docker.get_container_stats(&container_id).await {
                                Ok(stats) => {
                                    let mem = &stats.memory_stats;
                                    mem_usage_mb = mem.usage.unwrap_or(0) / 1024 / 1024;
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

                        if is_auto_pilot && do_update_check {
                            let docker_adapter = &scan_state.docker;
                            let svc_name = name.clone();
                            let d_adapter = docker_adapter.clone();
                            tokio::spawn(async move {
                                let _ = d_adapter.check_and_update_service(&svc_name).await;
                            });
                        }

                        // GEÇİÇİ GPU TESPİTİ
                        let mut has_gpu = false;
                        if name.contains("llm") || name.contains("ocr") || name.contains("cuda") || name.contains("diffusion") {
                            has_gpu = true;
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
                            has_gpu: has_gpu, 
                        };
                        
                        cache.insert(name, svc.clone());
                        list.push(svc);
                    }
                }
                Err(e) => { 
                    error!(event="DOCKER_LIST_ERROR", error=%e, "Docker konteyner listesi alınamadı"); 
                }
            }
            tokio::time::sleep(Duration::from_secs(poll_interval)).await;
        }
    });

    // 3. UPSTREAM LOOP
    if let Some(upstream_url) = cfg.upstream_url {
        let up_state = state.clone();
        let http_client = Client::new();
        let node_name = cfg.node_name.clone();

        tokio::spawn(async move {
            info!(event="UPSTREAM_LINK_INIT", url=%upstream_url, "Upstream raporlama başlatılıyor.");
            loop {
                let svcs: Vec<ServiceInstance> = up_state.services_cache.lock().await.values().cloned().collect();
                let stats: NodeStats = up_state.node_stats_cache.lock().await.clone();
                let payload = ClusterReport {
                    node: node_name.clone(),
                    stats: stats,
                    services: svcs,
                    timestamp: chrono::Utc::now().to_rfc3339()
                };

                match http_client.post(&upstream_url).json(&payload).send().await {
                    Ok(resp) => { 
                        if !resp.status().is_success() { 
                            warn!(event="UPSTREAM_SYNC_FAIL", status=%resp.status(), "Rapor gönderilemedi"); 
                        } else {
                            debug!(event="UPSTREAM_SYNC_SUCCESS", "Rapor başarıyla gönderildi");
                        }
                    },
                    Err(e) => { 
                        warn!(event="UPSTREAM_UNREACHABLE", error=%e, "Merkezi sunucuya ulaşılamıyor"); 
                    }
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });
    }

    let app = api::routes::create_router(state.clone());
    let addr = format!("{}:{}", cfg.host, cfg.http_port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}