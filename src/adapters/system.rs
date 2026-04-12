// src/adapters/system.rs
use crate::core::domain::NodeStats;
use std::process::Command;
use std::time::Instant;
use sysinfo::{Disks, Networks, System};

pub struct SystemMonitor {
    sys: System,
    networks: Networks,
    disks: Disks,
    node_name: String,
    last_update: Instant,
    last_net_rx: u64,
    last_net_tx: u64,
}

impl SystemMonitor {
    pub fn new(node_name: String) -> Self {
        Self {
            sys: System::new_all(),
            networks: Networks::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list(),
            node_name,
            last_update: Instant::now(),
            last_net_rx: 0,
            last_net_tx: 0,
        }
    }

    pub fn snapshot(&mut self) -> NodeStats {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.networks.refresh_list();
        self.disks.refresh_list();

        let elapsed = self.last_update.elapsed().as_secs_f64().max(0.1);
        self.last_update = Instant::now();

        // 1. AĞ İSTATİSTİKLERİ (SANAL KARTLARI GÖZ ARDI ET)
        let mut current_rx = 0;
        let mut current_tx = 0;
        for (interface_name, data) in &self.networks {
            let name = interface_name.to_lowercase();
            // Docker köprülerini, sanal arayüzleri ve loopback'i atlıyoruz (Çift sayımı önler)
            if name.starts_with("veth")
                || name.starts_with("br-")
                || name.starts_with("docker")
                || name.starts_with("lo")
            {
                continue;
            }
            current_rx += data.total_received();
            current_tx += data.total_transmitted();
        }

        let rx_delta = current_rx.saturating_sub(self.last_net_rx);
        let tx_delta = current_tx.saturating_sub(self.last_net_tx);

        self.last_net_rx = current_rx;
        self.last_net_tx = current_tx;

        let net_rx_mbs = (rx_delta as f64 / elapsed) / 1_048_576.0;
        let net_tx_mbs = (tx_delta as f64 / elapsed) / 1_048_576.0;

        // 2. DİSK İSTATİSTİKLERİ (SANAL DİSKLERİ GÖZ ARDI ET)
        let mut disk_total_bytes = 0;
        let mut disk_used_bytes = 0;

        for disk in &self.disks {
            // [KRİTİK DÜZELTME]: Cross-platform OsStr dönüşümü
            let fs_type = disk.file_system().to_string_lossy().to_lowercase();
            let mount_point = disk.mount_point().to_string_lossy().to_lowercase();

            // Snap (squashfs), RAM disk (tmpfs), Docker (overlay) ve /boot partitionlarını filtrele
            if fs_type.contains("squashfs")
                || fs_type.contains("tmpfs")
                || fs_type.contains("overlay")
                || fs_type.contains("devtmpfs")
                || fs_type.contains("efivarfs")
                || mount_point.starts_with("/boot")
            {
                continue;
            }

            disk_total_bytes += disk.total_space();
            disk_used_bytes += disk.total_space().saturating_sub(disk.available_space());
        }

        // GB cinsine çevir
        let disk_total_gb = disk_total_bytes / 1_073_741_824;
        let disk_used_gb = disk_used_bytes / 1_073_741_824;

        let (gpu_util, gpu_mem_used, gpu_mem_total) = self.get_gpu_metrics();

        NodeStats {
            name: self.node_name.clone(),
            cpu_usage: self.sys.global_cpu_usage(),
            ram_used: self.sys.used_memory() / 1024 / 1024,
            ram_total: self.sys.total_memory() / 1024 / 1024,
            disk_used: disk_used_gb,
            disk_total: disk_total_gb,
            gpu_usage: gpu_util,
            gpu_mem_used,
            gpu_mem_total,
            net_rx_mbs,
            net_tx_mbs,
            last_seen: chrono::Utc::now().to_rfc3339(),
            status: "ONLINE".to_string(),
        }
    }

    fn get_gpu_metrics(&self) -> (f32, u64, u64) {
        let output = Command::new("nvidia-smi")
            .args([
                "--query-gpu=utilization.gpu,memory.used,memory.total",
                "--format=csv,noheader,nounits",
            ])
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
