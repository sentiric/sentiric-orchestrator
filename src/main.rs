// src/main.rs
mod adapters;
mod api;
mod config;
mod core;
mod telemetry;

use bollard::container::ListContainersOptions;
use reqwest::Client;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Mutex};
use tracing::Instrument;
use tracing::{info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter, Registry};

use crate::adapters::docker::DockerAdapter;
use crate::adapters::system::SystemMonitor;
use crate::config::AppConfig;
use crate::core::domain::{ClusterReport, NodeStats, ServiceInstance};
use crate::core::governor::Governor;
use crate::telemetry::SutsFormatter;

struct ContainerStatsCache {
    cpu_usage: u64,
    system_usage: u64,
    net_rx: u64,
    net_tx: u64,
    disk_read: u64,
    disk_write: u64,
    last_update: Instant,
}

pub struct AppState {
    pub docker: DockerAdapter,
    pub auto_pilot_config: Mutex<HashMap<String, bool>>,
    pub services_cache: Mutex<HashMap<String, ServiceInstance>>,
    pub node_stats_cache: Mutex<NodeStats>,
    pub cluster_cache: Mutex<HashMap<String, ClusterReport>>,
    pub tx: Arc<broadcast::Sender<String>>,
    pub update_locks: Mutex<HashSet<String>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = AppConfig::load();

    let rust_log_env = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let env_filter =
        EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(&rust_log_env))?;
    let subscriber = Registry::default().with(env_filter);

    let log_format = std::env::var("LOG_FORMAT").unwrap_or_else(|_| "json".to_string());

    if log_format == "json" {
        let suts_formatter = SutsFormatter::new(
            "orchestrator-service".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
            cfg.env.clone(),
            cfg.node_name.clone(),
            cfg.tenant_id.clone(),
        );
        subscriber
            .with(fmt::layer().event_format(suts_formatter))
            .init();
    } else {
        subscriber.with(fmt::layer().compact()).init();
    }

    info!(
        event = "SYSTEM_STARTUP",
        service.version = env!("CARGO_PKG_VERSION"),
        node.name = %cfg.node_name,
        mode = if cfg.upstream_url.is_some() { "EDGE" } else { "MASTER" },
        "💠 SENTIRIC ORCHESTRATOR v6.6.0 (ENTERPRISE SRE GOVERNOR) Booting..."
    );

    let (tx, _) = broadcast::channel::<String>(100);
    let tx = Arc::new(tx);

    let docker = DockerAdapter::new(&cfg.docker_socket, cfg.node_name.clone(), tx.clone())?;
    let mut sys_mon = SystemMonitor::new(cfg.node_name.clone());

    let mut initial_ap = HashMap::new();
    for svc in &cfg.auto_pilot_services {
        initial_ap.insert(svc.clone(), true);
    }

    let state = Arc::new(AppState {
        docker: docker.clone(),
        auto_pilot_config: Mutex::new(initial_ap),
        services_cache: Mutex::new(HashMap::new()),
        node_stats_cache: Mutex::new(NodeStats::default()),
        cluster_cache: Mutex::new(HashMap::new()),
        tx: tx.clone(),
        update_locks: Mutex::new(HashSet::new()),
    });

    // 1. SYSTEM MONITOR & OTONOM KORUMA
    let mon_state = state.clone();
    let mon_node = cfg.node_name.clone();
    let mon_tx = tx.clone();

    tokio::spawn(async move {
        // İlk açılışta hemen prune yapmaması için başlangıç süresini 1 saat geriye alıyoruz.
        let mut last_prune_time = Instant::now() - Duration::from_secs(3600);

        loop {
            let stats = sys_mon.snapshot();
            let mut node_cache = mon_state.node_stats_cache.lock().await;
            *node_cache = stats.clone();
            drop(node_cache);

            // [SRE OTONOM KORUMA]: Disk %85'i geçerse ve son 1 saatte temizlenmediyse Auto-Prune tetikle
            let disk_pct = if stats.disk_total > 0 {
                (stats.disk_used as f64 / stats.disk_total as f64) * 100.0
            } else {
                0.0
            };

            if disk_pct > 85.0 && last_prune_time.elapsed().as_secs() > 3600 {
                warn!(event="AUTO_PRUNE_TRIGGERED", disk_usage_pct=%disk_pct, "🚨 Disk space critical (>85%). Triggering autonomous system prune.");

                let docker_clone = mon_state.docker.clone();
                tokio::spawn(async move {
                    let _ = docker_clone.prune_system().await;
                });

                last_prune_time = Instant::now();
            }

            let svcs = mon_state
                .services_cache
                .lock()
                .await
                .values()
                .cloned()
                .collect();
            let report = ClusterReport {
                node: mon_node.clone(),
                stats,
                services: svcs,
                timestamp: chrono::Utc::now().to_rfc3339(),
            };

            mon_state
                .cluster_cache
                .lock()
                .await
                .insert(mon_node.clone(), report);
            let cluster_map = mon_state.cluster_cache.lock().await.clone();
            let _ = mon_tx.send(
                serde_json::json!({ "type": "cluster_update", "data": cluster_map }).to_string(),
            );

            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    // 2. DOCKER SCAN & GOVERNANCE LOOP
    let scan_state = state.clone();
    let scan_node = cfg.node_name.clone();
    let poll_interval = cfg.poll_interval;

    tokio::spawn(async move {
        let client = scan_state.docker.get_client();
        let mut loop_counter = 0;
        let mut stats_cache: HashMap<String, ContainerStatsCache> = HashMap::new();
        let mut env_cache: HashMap<String, Vec<String>> = HashMap::new();

        loop {
            loop_counter += 1;
            let do_update_check = loop_counter % 12 == 0;
            let node_total_ram = scan_state.node_stats_cache.lock().await.ram_total;

            if let Ok(containers) = client
                .list_containers(Some(ListContainersOptions::<String> {
                    all: true,
                    ..Default::default()
                }))
                .await
            {
                let ap_guard = scan_state.auto_pilot_config.lock().await;
                let mut cache = scan_state.services_cache.lock().await;

                for c in containers {
                    let name = c
                        .names
                        .unwrap_or_default()
                        .first()
                        .cloned()
                        .unwrap_or_default()
                        .replace("/", "");
                    if name.is_empty() {
                        continue;
                    }

                    let is_auto_pilot = *ap_guard.get(&name).unwrap_or(&false);
                    let container_id = c.id.clone().unwrap_or_default();
                    let status_str = c.status.unwrap_or_default();
                    let is_up = status_str.to_lowercase().contains("up");

                    let mut cpu_percent = 0.0;
                    let mut mem_usage_mb = 0;
                    let gpu_mem_usage_mb = 0;
                    let mut net_rx_mbs = 0.0;
                    let mut net_tx_mbs = 0.0;
                    let mut disk_read_mbs = 0.0;
                    let mut disk_write_mbs = 0.0;

                    if is_up {
                        if let Ok(stats) =
                            scan_state.docker.get_container_stats(&container_id).await
                        {
                            mem_usage_mb = stats.memory_stats.usage.unwrap_or(0) / 1024 / 1024;

                            let cpu_total = stats.cpu_stats.cpu_usage.total_usage;
                            let system_total = stats.cpu_stats.system_cpu_usage.unwrap_or(0);

                            let mut current_net_rx = 0;
                            let mut current_net_tx = 0;
                            if let Some(networks) = &stats.networks {
                                for net_stat in networks.values() {
                                    current_net_rx += net_stat.rx_bytes;
                                    current_net_tx += net_stat.tx_bytes;
                                }
                            }

                            let mut current_disk_read = 0;
                            let mut current_disk_write = 0;
                            if let Some(io_stats) = &stats.blkio_stats.io_service_bytes_recursive {
                                for stat in io_stats {
                                    match stat.op.to_lowercase().as_str() {
                                        "read" => current_disk_read += stat.value,
                                        "write" => current_disk_write += stat.value,
                                        _ => {}
                                    }
                                }
                            }

                            if let Some(cached) = stats_cache.get(&container_id) {
                                let elapsed = cached.last_update.elapsed().as_secs_f64().max(0.1);

                                let system_delta =
                                    (system_total.saturating_sub(cached.system_usage)) as f64;
                                let cpu_delta = (cpu_total.saturating_sub(cached.cpu_usage)) as f64;
                                let online_cpus = stats.cpu_stats.online_cpus.unwrap_or(1) as f64;
                                if system_delta > 0.0 {
                                    cpu_percent = (cpu_delta / system_delta) * online_cpus * 100.0;
                                }

                                net_rx_mbs = (current_net_rx.saturating_sub(cached.net_rx) as f64
                                    / elapsed)
                                    / 1_048_576.0;
                                net_tx_mbs = (current_net_tx.saturating_sub(cached.net_tx) as f64
                                    / elapsed)
                                    / 1_048_576.0;
                                disk_read_mbs = (current_disk_read.saturating_sub(cached.disk_read)
                                    as f64
                                    / elapsed)
                                    / 1_048_576.0;
                                disk_write_mbs =
                                    (current_disk_write.saturating_sub(cached.disk_write) as f64
                                        / elapsed)
                                        / 1_048_576.0;
                            }

                            stats_cache.insert(
                                container_id.clone(),
                                ContainerStatsCache {
                                    cpu_usage: cpu_total,
                                    system_usage: system_total,
                                    net_rx: current_net_rx,
                                    net_tx: current_net_tx,
                                    disk_read: current_disk_read,
                                    disk_write: current_disk_write,
                                    last_update: Instant::now(),
                                },
                            );
                        }
                    } else {
                        stats_cache.remove(&container_id);
                    }

                    if !env_cache.contains_key(&container_id) && is_up {
                        if let Ok(inspect) = client
                            .inspect_container(
                                &container_id,
                                None::<bollard::container::InspectContainerOptions>,
                            )
                            .await
                        {
                            if let Some(config) = inspect.config {
                                if let Some(env) = config.env {
                                    env_cache.insert(container_id.clone(), env);
                                }
                            }
                        }
                    }

                    let env_vars = env_cache.get(&container_id).cloned().unwrap_or_default();
                    let violations = Governor::audit_compliance(&name, &env_vars);

                    let is_locked = scan_state.update_locks.lock().await.contains(&name);
                    let health = if is_locked {
                        crate::core::domain::HealthStatus::Draining
                    } else {
                        Governor::evaluate_health(
                            &status_str,
                            mem_usage_mb,
                            node_total_ram,
                            &violations,
                        )
                    };

                    if is_auto_pilot && do_update_check {
                        let mut locks = scan_state.update_locks.lock().await;
                        if !locks.contains(&name) {
                            locks.insert(name.clone());
                            drop(locks);

                            let svc_name = name.clone();
                            let d_adapter = scan_state.docker.clone();
                            let state_clone = scan_state.clone();

                            tokio::spawn(async move {
                                let _ = d_adapter.check_and_update_service(&svc_name).await;
                                let mut release_locks = state_clone.update_locks.lock().await;
                                release_locks.remove(&svc_name);
                            });
                        }
                    }

                    let has_gpu =
                        name.contains("llm") || name.contains("stt") || name.contains("tts");
                    let progress = cache.get(&name).and_then(|s| s.update_progress.clone());

                    let svc = ServiceInstance {
                        name: name.clone(),
                        image: c.image.unwrap_or_default(),
                        status: status_str,
                        short_id: container_id.chars().take(12).collect(),
                        auto_pilot: is_auto_pilot,
                        node: scan_node.clone(),
                        cpu_usage: cpu_percent,
                        mem_usage: mem_usage_mb,
                        gpu_mem_usage: gpu_mem_usage_mb,
                        has_gpu,
                        net_rx_mbs,
                        net_tx_mbs,
                        disk_read_mbs,
                        disk_write_mbs,
                        update_progress: progress,
                        health,
                        violations,
                    };

                    cache.insert(name, svc);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(poll_interval)).await;
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
                let svcs: Vec<ServiceInstance> = up_state
                    .services_cache
                    .lock()
                    .await
                    .values()
                    .cloned()
                    .collect();
                let stats: NodeStats = up_state.node_stats_cache.lock().await.clone();
                let payload = ClusterReport {
                    node: node_name.clone(),
                    stats,
                    services: svcs,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };

                let trace_id = format!(
                    "tr-{:x}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_micros()
                );
                let span = tracing::info_span!("upstream_push", trace_id = %trace_id);

                let _ = http_client
                    .post(&upstream_url)
                    .header("x-trace-id", &trace_id)
                    .json(&payload)
                    .send()
                    .instrument(span)
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
