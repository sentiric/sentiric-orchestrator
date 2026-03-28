// src/core/governor.rs
use crate::core::domain::HealthStatus;

pub struct Governor;

impl Governor {
    /// Sentinel: Bir konteynerin çevre değişkenlerini (ENV) Sentiric Spec kurallarına göre denetler.
    pub fn audit_compliance(service_name: &str, env_vars: &[String]) -> Vec<String> {
        let mut violations = Vec::new();

        // Harici servisler (postgres, redis, rabbitmq) denetimden muaftır.
        if !service_name.contains("-service") {
            return violations;
        }

        let has_tenant = env_vars.iter().any(|v| v.starts_with("TENANT_ID="));
        let has_mtls_cert = env_vars.iter().any(|v| v.starts_with("TLS_CERT_PATH="));
        let has_mtls_ca = env_vars.iter().any(|v| v.starts_with("TLS_CA_PATH=") || v.starts_with("GRPC_TLS_CA_PATH="));

        // Kural 1: Tüm iç servisler mTLS sertifikası yollarına sahip olmalıdır.
        if !has_mtls_cert || !has_mtls_ca {
            violations.push("[SOP-01] Missing mTLS certificates (TLS_CERT_PATH or CA_PATH). Communication is unencrypted!".to_string());
        }

        // Kural 2: Observer hariç tüm servislerin TENANT_ID bilmesi zorunludur.
        if !service_name.contains("observer") && !has_tenant {
            violations.push("[ARCH-03] Missing TENANT_ID. Service cannot guarantee data isolation.".to_string());
        }

        violations
    }

    /// Resource Guard: Servisin donanım tüketimini analiz eder.
    pub fn evaluate_health(
        status_str: &str, 
        mem_mb: u64, 
        node_total_ram_mb: u64, 
        violations: &[String]
    ) -> HealthStatus {
        if !status_str.to_lowercase().contains("up") {
            return HealthStatus::Offline;
        }

        if !violations.is_empty() {
            return HealthStatus::Quarantined;
        }

        // Eğer bir servis tek başına Node RAM'inin %80'inden fazlasını kullanıyorsa risk altındadır (OOM)
        if node_total_ram_mb > 0 {
            let usage_pct = (mem_mb as f64 / node_total_ram_mb as f64) * 100.0;
            if usage_pct > 80.0 {
                return HealthStatus::RiskOom;
            }
        }

        HealthStatus::Online
    }
}