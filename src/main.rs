// sentiric-orchestrator/src/main.rs

use axum::{
    extract::{State, Query},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use bollard::Docker;
use bollard::container::{
    ListContainersOptions, StopContainerOptions, RemoveContainerOptions, 
    Config, CreateContainerOptions, StartContainerOptions
};
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Duration;
use tracing::{info, error, warn, debug}; // warn artık kullanılacak
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// --- Veri Modelleri ---

#[derive(Serialize, Clone, Debug)]
struct ServiceInstance {
    name: String,
    container_id: String,
    image: String,
    status: String,
    short_id: String, // local_sha yerine sadece short_id kullanacağız
    last_sync: String,
    auto_pilot: bool,
    is_updating: bool,
}

#[derive(Deserialize)]
struct ActionParams {
    service: String,
}

#[derive(Deserialize)]
struct ToggleParams {
    service: String,
    enabled: bool,
}

struct AppState {
    docker: Docker,
    // Servis durumu ve Auto-Pilot tercihleri
    auto_pilot_config: Mutex<HashMap<String, bool>>,
    // Anlık durum snapshot'ı (UI için)
    cache: Mutex<Vec<ServiceInstance>>,
}

// --- Ana Uygulama ---

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    info!("📟 Sentiric Orchestrator v0.4.1 (Configurable Auto-Pilot) starting...");

    let docker = Docker::connect_with_local_defaults()
        .expect("❌ Failed to connect to Docker socket.");

    // --- CONFIG LOADING (ENVIRONMENT) ---
    // Örnek: AUTO_PILOT_SERVICES="proxy-service,media-service"
    let auto_pilot_env = env::var("AUTO_PILOT_SERVICES").unwrap_or_default();
    let mut initial_config = HashMap::new();
    
    if !auto_pilot_env.is_empty() {
        for svc in auto_pilot_env.split(',') {
            let s = svc.trim().to_string();
            if !s.is_empty() {
                initial_config.insert(s.clone(), true);
                info!("⚙️ Config: Auto-Pilot ENABLED for '{}' via ENV.", s);
            }
        }
    }

    // Polling aralığı (Varsayılan 60 saniye)
    let poll_interval_sec: u64 = env::var("POLL_INTERVAL")
        .unwrap_or_else(|_| "60".to_string())
        .parse()
        .unwrap_or(60);

    let shared_state = Arc::new(AppState {
        docker: docker.clone(),
        auto_pilot_config: Mutex::new(initial_config),
        cache: Mutex::new(Vec::new()),
    });

    // 1. SCANNER TASK (Konteyner durumlarını izler - UI için hızlı refresh)
    let scanner_state = shared_state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            if let Err(e) = scan_services(&scanner_state).await {
                error!("❌ Scanner error: {}", e);
            }
        }
    });

    // 2. AUTO-PILOT TASK (Güncellemeleri kontrol eder - Yavaş refresh)
    let autopilot_state = shared_state.clone();
    tokio::spawn(async move {
        info!("🤖 Auto-Pilot Engine active. Scan Interval: {}s", poll_interval_sec);
        let mut interval = tokio::time::interval(Duration::from_secs(poll_interval_sec));
        // İlk açılışta hemen başlamasın, sistem otursun
        tokio::time::sleep(Duration::from_secs(10)).await;
        
        loop {
            interval.tick().await;
            run_autopilot_cycle(&autopilot_state).await;
        }
    });

    // 3. API ROUTER
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/status", get(status_api_handler))
        .route("/api/update", post(manual_update_handler))
        .route("/api/toggle-autopilot", post(toggle_autopilot_handler))
        .with_state(shared_state);

    let http_port: u16 = env::var("ORCHESTRTOR_SERVICE_HTTP_PORT")
        .unwrap_or_else(|_| "11080".to_string())
        .parse().unwrap_or(11080);
    
    let addr = SocketAddr::from(([0, 0, 0, 0], http_port));
    info!("🚀 Command Center Active: http://localhost:{}", http_port);
    
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}

// --- Logic ---

async fn scan_services(state: &Arc<AppState>) -> anyhow::Result<()> {
    let options = Some(ListContainersOptions::<String> { all: true, ..Default::default() });
    let containers = state.docker.list_containers(options).await?;
    
    let config_guard = state.auto_pilot_config.lock().await;
    let mut new_cache = Vec::new();

    for c in containers {
        let name = c.names.unwrap_or_default().first().cloned().unwrap_or_default().replace("/", "");
        if name.is_empty() || name.contains("orchestrator") { continue; }

        let auto_pilot = *config_guard.get(&name).unwrap_or(&false);
        
        // Image ID'yi (SHA) kısalt
        let image_id = c.image_id.unwrap_or_default().replace("sha256:", "");
        let short_id = if image_id.len() > 12 { image_id[0..12].to_string() } else { image_id };

        new_cache.push(ServiceInstance {
            name,
            container_id: c.id.unwrap_or_default(),
            image: c.image.unwrap_or_default(),
            status: c.status.unwrap_or_default(),
            short_id: short_id, // [FIXED]: local_sha kaldırıldı, short_id atandı.
            last_sync: chrono::Utc::now().format("%H:%M:%S").to_string(),
            auto_pilot,
            is_updating: false,
        });
    }

    // İsme göre sırala
    new_cache.sort_by(|a, b| a.name.cmp(&b.name));

    let mut cache_guard = state.cache.lock().await;
    *cache_guard = new_cache;
    Ok(())
}

async fn run_autopilot_cycle(state: &Arc<AppState>) {
    let targets: Vec<String>;
    {
        let guard = state.auto_pilot_config.lock().await;
        targets = guard.iter().filter(|(_, &v)| v).map(|(k, _)| k.clone()).collect();
    }

    if targets.is_empty() { return; }
    debug!("🤖 Auto-Pilot Cycle: Checking {} services...", targets.len());

    for svc_name in targets {
        // 1. Mevcut imaj ID'sini al
        let current_inspect = match state.docker.inspect_container(&svc_name, None).await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let current_image_id = current_inspect.image.clone().unwrap_or_default();
        let image_name = current_inspect.config.as_ref().and_then(|c| c.image.clone()).unwrap_or_default();

        // 2. Registry'den PULL yap (Sessizce)
        debug!("🤖 [Auto-Pilot] Pulling manifest for: {}", svc_name);
        let mut pull_stream = state.docker.create_image(
            Some(CreateImageOptions { from_image: image_name.clone(), ..Default::default() }),
            None, None
        );
        
        while let Some(res) = pull_stream.next().await {
            if let Err(e) = res {
                warn!("⚠️ [Auto-Pilot] Registry Check Failed for {}: {}", svc_name, e);
                break; 
            }
        }

        // 3. Pull sonrası local image ID'yi kontrol et
        let new_image_inspect = match state.docker.inspect_image(&image_name).await {
            Ok(i) => i,
            Err(_) => continue,
        };
        
        let new_image_id = new_image_inspect.id.unwrap_or_default();

        if current_image_id != new_image_id {
            info!("🚀 [Auto-Pilot] Update Detected for {}! (Old -> New)", svc_name);
            if let Err(e) = perform_update(&state.docker, &svc_name).await {
                error!("❌ [Auto-Pilot] Update Failed for {}: {}", svc_name, e);
            }
        }
    }
}

/// Çekirdek Güncelleme Mantığı (Atomic Recreation)
async fn perform_update(docker: &Docker, svc_name: &str) -> Result<String, String> {
    // 1. Config Kopyala (Identity Preservation)
    let inspect = docker.inspect_container(svc_name, None).await.map_err(|e| e.to_string())?;
    let config = Config {
        image: inspect.config.as_ref().and_then(|c| c.image.clone()),
        env: inspect.config.as_ref().and_then(|c| c.env.clone()),
        labels: inspect.config.as_ref().and_then(|c| c.labels.clone()),
        host_config: inspect.host_config.clone(),
        networking_config: inspect.network_settings.as_ref().and_then(|n| {
            Some(bollard::container::NetworkingConfig { endpoints_config: n.networks.clone().unwrap_or_default() })
        }),
        ..Default::default()
    };

    // 2. Stop & Remove
    let _ = docker.stop_container(svc_name, Some(StopContainerOptions { t: 5 })).await;
    let _ = docker.remove_container(svc_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;

    // 3. Re-Create & Start
    docker.create_container(Some(CreateContainerOptions { name: svc_name.to_string(), platform: None }), config)
        .await.map_err(|e| format!("Create failed: {}", e))?;
    
    docker.start_container(svc_name, None::<StartContainerOptions<String>>)
        .await.map_err(|e| format!("Start failed: {}", e))?;

    Ok(format!("{} updated successfully.", svc_name))
}

// --- Handlers ---

async fn index_handler() -> Html<&'static str> { Html(include_str!("index.html")) }

async fn status_api_handler(State(state): State<Arc<AppState>>) -> Json<Vec<ServiceInstance>> {
    let guard = state.cache.lock().await;
    Json(guard.clone())
}

async fn manual_update_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ActionParams>
) -> impl IntoResponse {
    info!("🔧 Manual Update Requested: {}", params.service);
    
    let inspect = match state.docker.inspect_container(&params.service, None).await {
        Ok(c) => c,
        Err(_) => return (axum::http::StatusCode::NOT_FOUND, "Service not found".to_string()),
    };
    let image = inspect.config.and_then(|c| c.image).unwrap_or_default();
    
    let mut stream = state.docker.create_image(
        Some(CreateImageOptions { from_image: image, ..Default::default() }), None, None
    );
    while let Some(_) = stream.next().await {}

    match perform_update(&state.docker, &params.service).await {
        Ok(msg) => (axum::http::StatusCode::OK, msg),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn toggle_autopilot_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ToggleParams>,
) -> Json<bool> {
    let mut guard = state.auto_pilot_config.lock().await;
    guard.insert(payload.service.clone(), payload.enabled);
    info!("🎛️ Auto-Pilot Modified for {}: {}", payload.service, payload.enabled);
    Json(payload.enabled)
}