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
use futures_util::StreamExt; // [FIX]: Trait in scope
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Duration;
use tracing::{info, error}; // [FIX]: Removed unused warn
use serde::Deserialize;

#[derive(serde::Serialize, Clone, Debug)]
struct ServiceInstance {
    name: String,
    container_id: String,
    image: String,
    status: String,
    local_sha: String,
    last_sync: String,
}

#[derive(Deserialize)]
struct DeployParams {
    service: String,
}

struct AppState {
    docker: Docker,
    instances: Mutex<Vec<ServiceInstance>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();

    info!("📟 Sentiric Orchestrator v0.3.5 starting (Native Docker Mode)...");

    let docker = Docker::connect_with_local_defaults()
        .expect("❌ Failed to connect to Docker socket.");

    let shared_state = Arc::new(AppState {
        docker: docker.clone(),
        instances: Mutex::new(Vec::new()),
    });

    // --- 1. WATCHER TASK (SCANNER) ---
    let watcher_state = shared_state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            if let Err(e) = scan_local_services(&watcher_state).await {
                error!("❌ Local scan error: {}", e);
            }
        }
    });

    // --- 2. API & UI ROUTES ---
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/status", get(status_api_handler))
        .route("/api/update", post(update_handler))
        .with_state(shared_state);

    let http_port: u16 = env::var("ORCHESTRTOR_SERVICE_HTTP_PORT")
        .unwrap_or_else(|_| "11080".to_string())
        .parse().unwrap_or(11080);
    
    let addr = SocketAddr::from(([0, 0, 0, 0], http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    info!("🚀 Command Center: http://localhost:{}", http_port);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn scan_local_services(state: &Arc<AppState>) -> anyhow::Result<()> {
    let options = Some(ListContainersOptions::<String> {
        all: true,
        ..Default::default()
    });

    let containers = state.docker.list_containers(options).await?;
    let mut current_instances = Vec::new();

    for container in containers {
        let name = container.names.unwrap_or_default().join("").trim_start_matches('/').to_string();
        if name.is_empty() { continue; }

        let instance = ServiceInstance {
            name,
            container_id: container.id.clone().unwrap_or_default(),
            image: container.image.unwrap_or_default(),
            status: container.status.unwrap_or_default(),
            local_sha: container.image_id.unwrap_or_else(|| "unknown".into())
                .replace("sha256:", "").get(..12).unwrap_or("unknown").to_string(),
            last_sync: chrono::Utc::now().format("%H:%M:%S").to_string(),
        };
        current_instances.push(instance);
    }

    let mut guard = state.instances.lock().await;
    *guard = current_instances;
    Ok(())
}

// --- HANDLERS ---

async fn index_handler() -> Html<&'static str> { Html(include_str!("index.html")) }

async fn status_api_handler(State(state): State<Arc<AppState>>) -> Json<Vec<ServiceInstance>> {
    let guard = state.instances.lock().await;
    Json(guard.clone())
}

/// Native Docker Update Logic
async fn update_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DeployParams>
) -> impl IntoResponse {
    let svc_name = params.service;
    info!("⚙️ Orchestrating update for: {}", svc_name);

    // 1. Inspect existing container config
    let container_info = match state.docker.inspect_container(&svc_name, None).await {
        Ok(info) => info,
        Err(e) => {
            error!("❌ Container lookup failed: {}", e);
            return (axum::http::StatusCode::NOT_FOUND, format!("Container {} not found", svc_name));
        }
    };

    let image_name = container_info.config.as_ref().and_then(|c| c.image.clone()).unwrap();

    // 2. Pull latest image from registry
    info!("📥 Pulling: {}", image_name);
    let mut pull_stream = state.docker.create_image(
        Some(CreateImageOptions {
            from_image: image_name.clone(),
            ..Default::default()
        }),
        None,
        None,
    );

    while let Some(pull_result) = pull_stream.next().await {
        if let Err(e) = pull_result {
            error!("❌ Pull failed: {}", e);
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Registry error: {}", e));
        }
    }

    // 3. Graceful Stop
    info!("🛑 Stopping: {}", svc_name);
    let _ = state.docker.stop_container(&svc_name, Some(StopContainerOptions { t: 10 })).await;

    // 4. Forced Removal
    info!("🗑️ Removing: {}", svc_name);
    if let Err(e) = state.docker.remove_container(&svc_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await {
        error!("❌ Cleanup failed: {}", e);
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Disk error: {}", e));
    }

    // 5. Recreate with identical config
    info!("🏗️ Recreating: {}", svc_name);
    let config = Config {
        image: Some(image_name.clone()),
        env: container_info.config.as_ref().and_then(|c| c.env.clone()),
        host_config: container_info.host_config.clone(),
        networking_config: container_info.network_settings.as_ref().and_then(|n| {
            Some(bollard::container::NetworkingConfig {
                endpoints_config: n.networks.clone().unwrap_or_default(),
            })
        }),
        ..Default::default()
    };

    match state.docker.create_container(Some(CreateContainerOptions { name: svc_name.clone(), platform: None }), config).await {
        Ok(_) => {
            // 6. Start
            if let Err(e) = state.docker.start_container(&svc_name, None::<StartContainerOptions<String>>).await {
                error!("❌ Boot failed: {}", e);
                return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Boot error: {}", e));
            }
            info!("✅ Successfully redeployed: {}", svc_name);
            (axum::http::StatusCode::OK, format!("{} re-deployed successfully.", svc_name))
        },
        Err(e) => {
            error!("❌ Orchestration failed: {}", e);
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Config error: {}", e))
        }
    }
}