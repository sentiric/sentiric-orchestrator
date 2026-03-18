// Dosya: src/main.rs
mod config;
mod core;
mod adapters;
mod api;
mod telemetry; 

use std::{sync::Arc, collections::HashMap, time::Duration};
use tokio::sync::{Mutex, broadcast};
use tracing::info; 
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

    let rust_log_env = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let env_filter = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(&rust_log_env))?;
    let subscriber = Registry::default().with(env_filter);
    
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
        "💠 SENTIRIC ORCHESTRATOR v5.5 (OPTIMIZED) Booting..."
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
            let mut node_cache = mon_state.node_stats_cache.lock().await;
            *node_cache = stats.clone();
            drop(node_cache);

            let svcs = mon_state.services_cache.lock().await.values().cloned().collect();
            let report = ClusterReport {
                node: mon_node.clone(),
                stats: stats,
                services: svcs,
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
            
            mon_state.cluster_cache.lock().await.insert(mon_node.clone(), report);
            let cluster_map = mon_state.cluster_cache.lock().await.clone();
            let _ = mon_tx.send(serde_json::json!({ "type": "cluster_update", "data": cluster_map }).to_string());
            
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    // 2. DOCKER SCAN (CPU OPTIMIZED)
    let scan_state = state.clone();
    let scan_node = cfg.node_name.clone();
    let poll_interval = cfg.poll_interval; 

    tokio::spawn(async move {
        let client = scan_state.docker.get_client();
        let mut loop_counter = 0;
        let mut cpu_cache: HashMap<String, CpuStatsCache> = HashMap::new();

        loop {
            loop_counter += 1;
            let fetch_heavy_metrics = loop_counter % 3 == 0; 
            let do_update_check = loop_counter % 12 == 0; 

            match client.list_containers(Some(ListContainersOptions::<String> { all: true, ..Default::default() })).await {
                Ok(containers) => {
                    let ap_guard = scan_state.auto_pilot_config.lock().await;
                    let mut cache = scan_state.services_cache.lock().await;

                    for c in containers {
                        let name = c.names.unwrap_or_default().first().cloned().unwrap_or_default().replace("/", "");
                        if name.is_empty() { continue; }

                        let is_auto_pilot = *ap_guard.get(&name).unwrap_or(&false);
                        let container_id = c.id.clone().unwrap_or_default();
                        let status_str = c.status.unwrap_or_default();
                        let is_up = status_str.to_lowercase().contains("up");

                        let mut cpu_percent = 0.0;
                        let mut mem_usage_mb = 0;
                        
                        if let Some(existing) = cache.get(&name) {
                            cpu_percent = existing.cpu_usage;
                            mem_usage_mb = existing.mem_usage;
                        }

                        if is_up && fetch_heavy_metrics {
                            if let Ok(stats) = scan_state.docker.get_container_stats(&container_id).await {
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
                            }
                        } else if !is_up {
                            cpu_cache.remove(&container_id);
                            cpu_percent = 0.0;
                            mem_usage_mb = 0;
                        }

                        if is_auto_pilot && do_update_check {
                            let docker_adapter = &scan_state.docker;
                            let svc_name = name.clone();
                            let d_adapter = docker_adapter.clone();
                            tokio::spawn(async move {
                                let _ = d_adapter.check_and_update_service(&svc_name).await;
                            });
                        }

                        let has_gpu = name.contains("llm") || name.contains("ocr") || name.contains("cuda") || name.contains("diffusion");

                        let svc = ServiceInstance {
                            name: name.clone(),
                            image: c.image.unwrap_or_default(),
                            status: status_str,
                            short_id: container_id.chars().take(12).collect(),
                            auto_pilot: is_auto_pilot,
                            node: scan_node.clone(),
                            cpu_usage: cpu_percent,
                            mem_usage: mem_usage_mb,
                            has_gpu, 
                        };
                        
                        cache.insert(name, svc);
                    }
                }
                Err(_) => { } 
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
                    stats,
                    services: svcs,
                    timestamp: chrono::Utc::now().to_rfc3339()
                };

                // [ARCH-COMPLIANCE] constraints.yaml'ın gerektirdiği şekilde bağlam yayılımı (trace_id) eklendi
                let trace_id = format!("tr-{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros());
                let _span = tracing::info_span!("upstream_push", trace_id = %trace_id).entered();

                let _ = http_client.post(&upstream_url)
                    .header("x-trace-id", &trace_id)
                    .json(&payload)
                    .send()
                    .await;
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