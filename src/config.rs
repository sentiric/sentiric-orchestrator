use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub env: String,
    pub node_name: String,
    pub host: String,
    pub http_port: u16,
    pub grpc_port: u16,
    pub docker_socket: String,
    pub poll_interval: u64,
    pub auto_pilot_services: Vec<String>,
}

impl AppConfig {
    pub fn load() -> Self {
        let ap_raw = env::var("AUTO_PILOT_SERVICES").unwrap_or_default();
        let ap_list = ap_raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Self {
            env: env::var("ENV").unwrap_or_else(|_| "production".into()),
            node_name: env::var("NODE_NAME").unwrap_or_else(|_| 
                hostname::get().map(|h| h.to_string_lossy().into_owned()).unwrap_or("NEXUS-NODE".into())
            ).to_uppercase(),
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            http_port: env::var("HTTP_PORT").unwrap_or("11080".to_string()).parse().unwrap_or(11080),
            grpc_port: env::var("GRPC_PORT").unwrap_or("11081".to_string()).parse().unwrap_or(11081),
            docker_socket: env::var("DOCKER_SOCKET").unwrap_or_else(|_| 
                if cfg!(target_os = "windows") { "//./pipe/docker_engine".into() } 
                else { "/var/run/docker.sock".into() }
            ),
            poll_interval: env::var("POLL_INTERVAL").unwrap_or("60".to_string()).parse().unwrap_or(60),
            auto_pilot_services: ap_list,
        }
    }
}