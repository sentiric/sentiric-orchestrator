use sysinfo::System;
use std::process::Command;
use crate::core::domain::NodeStats;

pub struct SystemMonitor {
    sys: System,
    node_name: String,
}

impl SystemMonitor {
    pub fn new(node_name: String) -> Self {
        Self {
            sys: System::new_all(),
            node_name,
        }
    }

    pub fn snapshot(&mut self) -> NodeStats {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();

        let (gpu_util, gpu_mem_used, gpu_mem_total) = self.get_gpu_metrics();

        NodeStats {
            name: self.node_name.clone(),
            cpu_usage: self.sys.global_cpu_usage(),
            ram_used: self.sys.used_memory() / 1024 / 1024,
            ram_total: self.sys.total_memory() / 1024 / 1024,
            gpu_usage: gpu_util,
            gpu_mem_used,
            gpu_mem_total,
            last_seen: chrono::Utc::now().to_rfc3339(),
            status: "ONLINE".to_string(),
        }
    }

    fn get_gpu_metrics(&self) -> (f32, u64, u64) {
        let output = Command::new("nvidia-smi")
            .args(&["--query-gpu=utilization.gpu,memory.used,memory.total", "--format=csv,noheader,nounits"])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                let parts: Vec<&str> = s.trim().split(',').collect();
                if parts.len() >= 3 {
                    let usage = parts[0].trim().parse::<f32>().unwrap_or(0.0);
                    let mem = parts[1].trim().parse::<u64>().unwrap_or(0);
                    let total = parts[2].trim().parse::<u64>().unwrap_or(0);
                    return (usage, mem, total);
                }
            }
        }
        (0.0, 0, 0)
    }
}