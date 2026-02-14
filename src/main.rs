// sentiric-orchestrator/src/main.rs

use axum::{response::Html, routing::get, Router};
use bollard::Docker;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Duration;
use tracing::{info, error, debug};

#[derive(serde::Serialize, Clone, Debug)]
struct ServiceInstance {
    name: String,
    container_id: String,
    image: String,
    status: String,
    local_sha: String,
    remote_sha: String,
    last_sync: String,
}

struct AppState {
    instances: Mutex<Vec<ServiceInstance>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Logger: RUST_LOG=debug cargo run diyerek detay görebilirsiniz
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();

    info!("📟 Sentiric Orchestrator v0.1.1 starting...");

    let shared_state = Arc::new(AppState {
        instances: Mutex::new(Vec::new()),
    });

    // 2. Docker Engine Connection
    let docker = Arc::new(Docker::connect_with_local_defaults()
        .expect("❌ Failed to connect to Docker socket."));

    // 3. Watcher Task (Her 10 saniyede bir tara - Test için hızlandırıldı)
    let watcher_state = shared_state.clone();
    let watcher_docker = docker.clone();
    tokio::spawn(async move {
        info!("🔍 Watcher activated: Auto-detecting all local containers.");
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            if let Err(e) = scan_local_services(&watcher_docker, &watcher_state).await {
                error!("❌ Scan error: {}", e);
            }
        }
    });

    // 4. API & UI
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/status", get(status_api_handler))
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

async fn scan_local_services(docker: &Docker, state: &Arc<AppState>) -> anyhow::Result<()> {
    use bollard::container::ListContainersOptions;

    let options = Some(ListContainersOptions::<String> {
        all: true, // Durmuş (exited) konteynerleri de gör
        ..Default::default()
    });

    let containers = docker.list_containers(options).await?;
    let mut current_instances = Vec::new();

    for container in containers {
        let name = container.names.unwrap_or_default().join(", ").trim_start_matches('/').to_string();
        
        // [ADIM 1 FIX]: Filtreyi kaldırıyoruz, her şeyi görelim.
        let instance = ServiceInstance {
            name: name.clone(),
            container_id: container.id.clone().unwrap_or_default(),
            image: container.image.unwrap_or_default(),
            status: container.status.unwrap_or_default(),
            // SHA'nın ilk 12 karakterini alalım (Docker tarzı)
            local_sha: container.image_id.unwrap_or_else(|| "unknown".into()).replace("sha256:", "")[..12].to_string(),
            remote_sha: "LATEST".into(), 
            last_sync: chrono::Utc::now().format("%H:%M:%S").to_string(),
        };
        current_instances.push(instance);
    }

    let mut guard = state.instances.lock().await;
    *guard = current_instances;
    info!("📊 Scan complete: {} containers detected.", guard.len());
    
    Ok(())
}

async fn index_handler() -> Html<&'static str> { Html(include_str!("index.html")) }
async fn status_api_handler(axum::extract::State(state): axum::extract::State<Arc<AppState>>) -> axum::Json<Vec<ServiceInstance>> {
    let guard = state.instances.lock().await;
    axum::Json(guard.clone())
}