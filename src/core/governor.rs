// src/core/governor.rs
use crate::core::domain::HealthStatus;

pub struct Governor;

impl Governor {
    pub fn audit_compliance(service_name: &str, env_vars: &[String]) -> Vec<String> {
        let mut violations = Vec::new();
        if !service_name.contains("-service") {
            return violations;
        }

        let has_tenant = env_vars.iter().any(|v| v.starts_with("TENANT_ID="));
        let has_mtls_cert = env_vars.iter().any(|v| v.starts_with("TLS_CERT_PATH="));
        let has_mtls_ca = env_vars
            .iter()
            .any(|v| v.starts_with("TLS_CA_PATH=") || v.starts_with("GRPC_TLS_CA_PATH="));

        if !has_mtls_cert || !has_mtls_ca {
            violations
                .push("[SOP-01] Missing mTLS certificates. Unencrypted traffic risk.".to_string());
        }
        if !service_name.contains("observer")
            && !service_name.contains("orchestrator")
            && !has_tenant
        {
            violations.push("[ARCH-03] Missing TENANT_ID. Strict isolation violated.".to_string());
        }
        violations
    }

    pub fn evaluate_health(
        status_str: &str,
        mem_mb: u64,
        node_total_ram_mb: u64,
        violations: &[String],
    ) -> HealthStatus {
        if !status_str.to_lowercase().contains("up") {
            return HealthStatus::Offline;
        }
        // [YENİ]: Violations artık sistemi Kırmızı yapmaz, Turuncu (Warning) yapar.
        if !violations.is_empty() {
            return HealthStatus::Warning;
        }
        if node_total_ram_mb > 0 {
            let usage_pct = (mem_mb as f64 / node_total_ram_mb as f64) * 100.0;
            if usage_pct > 80.0 {
                return HealthStatus::RiskOom;
            }
        }
        HealthStatus::Online
    }
}
