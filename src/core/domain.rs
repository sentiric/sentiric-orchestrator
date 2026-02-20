use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServiceInstance {
    pub name: String,
    pub image: String,
    pub status: String,
    pub short_id: String,
    pub auto_pilot: bool,
    pub node: String,
    pub cpu_usage: f64,
    pub mem_usage: u64, // MB
    pub has_gpu: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NodeStats {
    pub name: String,
    pub cpu_usage: f32,
    pub ram_used: u64, // MB
    pub ram_total: u64, // MB
    pub gpu_usage: f32,
    pub gpu_mem_used: u64,
    pub gpu_mem_total: u64,
    pub last_seen: String, // ISO8601
    pub status: String,
}

// YENİ: Edge Node'lardan gelen rapor paketi
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClusterReport {
    pub node: String,
    pub stats: NodeStats,
    pub services: Vec<ServiceInstance>,
    pub timestamp: String,
}

#[derive(Deserialize)]
pub struct ActionParams {
    pub service: String,
}

#[derive(Deserialize)]
pub struct ToggleParams {
    pub service: String, 
    pub enabled: bool 
}