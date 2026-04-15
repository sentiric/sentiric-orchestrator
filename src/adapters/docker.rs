// src/adapters/docker.rs
use anyhow::Result;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, LogOutput, LogsOptions,
    PruneContainersOptions, RemoveContainerOptions, RestartContainerOptions, StartContainerOptions,
    Stats, StatsOptions, StopContainerOptions,
};
use bollard::image::{CreateImageOptions, PruneImagesOptions};
use bollard::Docker;
use futures_util::{Stream, StreamExt};
use std::default::Default;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct DockerAdapter {
    client: Docker,
    node_name: String,
    tx: Arc<broadcast::Sender<String>>,
}

impl DockerAdapter {
    pub fn new(
        socket: &str,
        node_name: String,
        tx: Arc<broadcast::Sender<String>>,
    ) -> Result<Self> {
        let client = Docker::connect_with_unix(socket, 120, bollard::API_DEFAULT_VERSION)
            .or_else(|_| Docker::connect_with_local_defaults())
            .map_err(|e| anyhow::anyhow!("Docker Bağlantı Hatası: {}", e))?;

        Ok(Self {
            client,
            node_name,
            tx,
        })
    }

    pub fn get_client(&self) -> Docker {
        self.client.clone()
    }

    // --- LIFECYCLE ---
    pub async fn start_service(&self, svc_id: &str) -> Result<()> {
        info!(event="CONTAINER_START", node.name=%self.node_name, container.id=%svc_id, "▶️ Starting container: {}", svc_id);
        self.client
            .start_container(svc_id, None::<StartContainerOptions<String>>)
            .await?;
        Ok(())
    }

    pub async fn stop_service(&self, svc_id: &str) -> Result<()> {
        info!(event="CONTAINER_STOP", node.name=%self.node_name, container.id=%svc_id, "🛑 Stopping container: {}", svc_id);
        self.client
            .stop_container(svc_id, Some(StopContainerOptions { t: 10 }))
            .await?;
        Ok(())
    }

    pub async fn restart_service(&self, svc_id: &str) -> Result<()> {
        info!(event="CONTAINER_RESTART", node.name=%self.node_name, container.id=%svc_id, "🔄 Restarting container: {}", svc_id);
        self.client
            .restart_container(svc_id, Some(RestartContainerOptions { t: 10 }))
            .await?;
        Ok(())
    }

    // --- INFO & LOGS ---
    pub fn get_log_stream(
        &self,
        svc_id: &str,
    ) -> impl Stream<Item = Result<LogOutput, bollard::errors::Error>> {
        debug!(event="STREAM_LOGS", node.name=%self.node_name, container.id=%svc_id, "📡 Opening live log stream for container: {}", svc_id);
        let options = Some(LogsOptions::<String> {
            follow: true,
            stdout: true,
            stderr: true,
            tail: "200".to_string(),
            ..Default::default()
        });
        self.client.logs(svc_id, options)
    }

    pub async fn get_logs_snapshot(&self, svc_id: &str) -> String {
        debug!(event="SNAPSHOT_LOGS", node.name=%self.node_name, container.id=%svc_id, "📸 Fetching log snapshot for container: {}", svc_id);
        let options = Some(LogsOptions::<String> {
            follow: false,
            stdout: true,
            stderr: true,
            tail: "50".to_string(),
            ..Default::default()
        });

        let mut stream = self.client.logs(svc_id, options);
        let mut buffer = String::new();

        while let Some(Ok(output)) = stream.next().await {
            let bytes: Vec<u8> = match output {
                LogOutput::StdOut { message } => message.into(),
                LogOutput::StdErr { message } => message.into(),
                LogOutput::Console { message } => message.into(),
                LogOutput::StdIn { message } => message.into(),
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));
        }
        buffer
    }

    pub async fn get_container_stats(&self, svc_id: &str) -> Result<Stats> {
        debug!(event="FETCH_STATS", node.name=%self.node_name, container.id=%svc_id, "📊 Fetching stats for container: {}", svc_id);

        let options = Some(StatsOptions {
            stream: false,
            one_shot: true,
        });
        let mut stream = self.client.stats(svc_id, options);
        if let Some(result) = stream.next().await {
            return result.map_err(|e| anyhow::anyhow!("Stats error: {}", e));
        }
        Err(anyhow::anyhow!("No stats received"))
    }

    pub async fn inspect_service(
        &self,
        svc_id: &str,
    ) -> Result<bollard::models::ContainerInspectResponse> {
        debug!(event="INSPECT_CONTAINER", node.name=%self.node_name, container.id=%svc_id, "🔎 Inspecting container: {}", svc_id);
        self.client
            .inspect_container(svc_id, None::<InspectContainerOptions>)
            .await
            .map_err(|e| anyhow::anyhow!("Inspect error: {}", e))
    }

    // --- THE JANITOR ---
    pub async fn prune_system(&self) -> Result<String> {
        info!(event="SYSTEM_PRUNE_START", node.name=%self.node_name, "🧹 Starting system prune...");
        let c_prune = self
            .client
            .prune_containers(None::<PruneContainersOptions<String>>)
            .await?;
        let c_deleted = c_prune.containers_deleted.unwrap_or_default().len();

        let i_prune = self
            .client
            .prune_images(None::<PruneImagesOptions<String>>)
            .await?;
        let i_deleted = i_prune.images_deleted.unwrap_or_default().len();
        let space = i_prune.space_reclaimed.unwrap_or(0);

        let msg = format!(
            "Deleted {} Containers, {} Images. Reclaimed {:.2} MB",
            c_deleted,
            i_deleted,
            (space as f64 / 1024.0 / 1024.0)
        );

        info!(
            event = "SYSTEM_PRUNE_DONE",
            node.name = %self.node_name,
            deleted.containers = c_deleted,
            deleted.images = i_deleted,
            reclaimed.mb = (space as f64 / 1024.0 / 1024.0),
            "♻️ JANITOR REPORT: {}", msg
        );
        Ok(msg)
    }

    // --- UPDATE ENGINE & SRE AUTO-ROLLBACK ---
    pub async fn check_and_update_service(&self, svc_name: &str) -> Result<bool> {
        debug!(
            event="CHECK_UPDATES",
            node.name=%self.node_name,
            service=%svc_name,
            "🔍 Checking updates for service: {}", svc_name
        );

        let docker = &self.client;
        let inspect = docker
            .inspect_container(svc_name, None::<InspectContainerOptions>)
            .await
            .map_err(|e| anyhow::anyhow!("Service not found: {}", e))?;

        let current_image_id = inspect.image.clone().unwrap_or_default();
        let image_name = inspect
            .config
            .as_ref()
            .and_then(|c| c.image.clone())
            .ok_or_else(|| anyhow::anyhow!("No image defined"))?;

        let is_self = svc_name.contains("orchestrator");

        // [ARCH-COMPLIANCE FIX]: Eski konfigürasyonu Rollback için sakla
        let old_config = Config {
            image: Some(current_image_id.clone()), // Rollback'te eski Image ID kullanılır
            env: inspect.config.as_ref().and_then(|c| c.env.clone()),
            labels: inspect.config.as_ref().and_then(|c| c.labels.clone()),
            host_config: inspect.host_config.clone(),
            networking_config: inspect.network_settings.as_ref().map(|n| {
                bollard::container::NetworkingConfig {
                    endpoints_config: n.networks.clone().unwrap_or_default(),
                }
            }),
            ..Default::default()
        };

        // 1. PULL (Yeni imajı çek ve Progress bildir)
        let mut stream = docker.create_image(
            Some(CreateImageOptions {
                from_image: image_name.clone(),
                ..Default::default()
            }),
            None,
            None,
        );

        while let Some(res) = stream.next().await {
            match res {
                Ok(info) => {
                    let status = info.status.unwrap_or_default();
                    let progress = if let Some(det) = info.progress_detail {
                        if let (Some(curr), Some(tot)) = (det.current, det.total) {
                            if tot > 0 {
                                format!(
                                    "{} ({}%)",
                                    status,
                                    (curr as f64 / tot as f64 * 100.0) as u32
                                )
                            } else {
                                status.clone()
                            }
                        } else {
                            status.clone()
                        }
                    } else {
                        status.clone()
                    }
                    .replace("\n", "");

                    let _ = self.tx.send(
                        serde_json::json!({
                            "type": "update_progress",
                            "data": { "service": svc_name, "progress": progress }
                        })
                        .to_string(),
                    );
                }
                Err(e) => {
                    error!(event="IMAGE_PULL_FAIL", error=%e, "❌ Pull Error: {}", e);
                    let _ = self.tx.send(
                        serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": null } }).to_string(),
                    );
                    return Err(anyhow::anyhow!("Registry error"));
                }
            }
        }

        // 2. COMPARE (Versiyon karşılaştır)
        let new_image_inspect = docker.inspect_image(&image_name).await?;
        let new_image_id = new_image_inspect.id.clone().unwrap_or_default();

        if current_image_id == new_image_id {
            let _ = self.tx.send(
                serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": null } }).to_string(),
            );
            return Ok(false);
        }

        info!(event="AUTO_PILOT_UPDATE_FOUND", service=%svc_name, "🚀 UPDATE FOUND for service: [{}]", svc_name);

        if is_self {
            warn!(
                event = "SELF_UPDATE_PREVENTED",
                "⚠️ Orchestrator cannot restart itself."
            );
            let _ = self.tx.send(
                serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": null } }).to_string(),
            );
            return Ok(true);
        }

        let new_config = Config {
            image: Some(image_name.clone()),
            env: inspect.config.as_ref().and_then(|c| c.env.clone()),
            labels: inspect.config.as_ref().and_then(|c| c.labels.clone()),
            host_config: inspect.host_config.clone(),
            networking_config: inspect.network_settings.as_ref().map(|n| {
                bollard::container::NetworkingConfig {
                    endpoints_config: n.networks.clone().unwrap_or_default(),
                }
            }),
            ..Default::default()
        };

        // 3. ZERO-DOWNTIME GRACEFUL SHUTDOWN (Dökülme/Drain)
        info!(event="CONTAINER_DRAINING", service=%svc_name, "🛑 Sending SIGTERM for graceful drain: [{}]", svc_name);
        let _ = self.tx.send(serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": "DRAINING (60s)" } }).to_string());

        let stop_opts = Some(StopContainerOptions { t: 60 });
        match docker.stop_container(svc_name, stop_opts).await {
            Ok(_) => {
                info!(event="CONTAINER_STOP_SIGNALED", service=%svc_name, "🛑 Stop signal sent.")
            }
            Err(e) => {
                warn!(event="CONTAINER_STOP_ERROR", service=%svc_name, error=%e, "⚠️ Error while stopping container (maybe already stopped): {}", e)
            }
        }

        // [ARCH-COMPLIANCE FIX]: Race Condition Koruması. Gerçekten kapanmasını bekle.
        let mut wait_stream = docker.wait_container(
            svc_name,
            None::<bollard::container::WaitContainerOptions<String>>,
        );
        tokio::select! {
            _ = wait_stream.next() => {
                debug!(event="CONTAINER_HALTED", service=%svc_name, "Container execution halted completely.");
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(65)) => {
                warn!(event="CONTAINER_WAIT_TIMEOUT", service=%svc_name, "Timeout waiting for container to stop. Forcing removal.");
            }
        }

        let remove_opts = Some(RemoveContainerOptions {
            force: true,
            ..Default::default()
        });
        match docker.remove_container(svc_name, remove_opts).await {
            Ok(_) => {
                info!(event="CONTAINER_REMOVED", service=%svc_name, "💀 Old container completely removed.")
            }
            Err(e) => {
                warn!(event="CONTAINER_REMOVE_ERROR", service=%svc_name, error=%e, "⚠️ Error while removing container: {}", e)
            }
        }

        info!(event="CONTAINER_RECREATING", service=%svc_name, "✨ Creating updated container: [{}]", svc_name);
        let _ = self.tx.send(serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": "STARTING..." } }).to_string());

        if let Err(e) = docker
            .create_container(
                Some(CreateContainerOptions {
                    name: svc_name.to_string(),
                    platform: None,
                }),
                new_config,
            )
            .await
        {
            error!(event="CONTAINER_CREATE_ERROR", service=%svc_name, error=%e, "❌ Failed to create container: {}", e);
            let _ = self.tx.send(serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": null } }).to_string());
            return Err(anyhow::anyhow!("Container create failed"));
        }

        if let Err(e) = docker
            .start_container(svc_name, None::<StartContainerOptions<String>>)
            .await
        {
            error!(event="CONTAINER_START_ERROR", service=%svc_name, error=%e, "❌ Failed to start container: {}", e);
            let _ = self.tx.send(serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": null } }).to_string());
            return Err(anyhow::anyhow!("Container start failed"));
        }

        // [ARCH-COMPLIANCE FIX]: SRE Auto-Rollback Mekanizması
        let _ = self.tx.send(serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": "HEALTH CHECK (5s)..." } }).to_string());
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        if let Ok(verify_inspect) = docker
            .inspect_container(svc_name, None::<InspectContainerOptions>)
            .await
        {
            if let Some(state) = verify_inspect.state {
                if state.running != Some(true) {
                    error!(event="AUTO_ROLLBACK_TRIGGERED", service=%svc_name, "🚨 New version crashed instantly! Initiating Auto-Rollback to previous stable state.");
                    let _ = self.tx.send(serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": "ROLLBACK IN PROGRESS🚨" } }).to_string());

                    let _ = docker.remove_container(svc_name, remove_opts).await;
                    if docker
                        .create_container(
                            Some(CreateContainerOptions {
                                name: svc_name.to_string(),
                                platform: None,
                            }),
                            old_config,
                        )
                        .await
                        .is_ok()
                    {
                        let _ = docker
                            .start_container(svc_name, None::<StartContainerOptions<String>>)
                            .await;
                        info!(event="AUTO_ROLLBACK_SUCCESS", service=%svc_name, "♻️ Service rolled back to previous stable image.");
                    } else {
                        error!(event="AUTO_ROLLBACK_FAILED", service=%svc_name, "❌ Fatal Error: Failed to rollback service.");
                    }

                    let _ = self.tx.send(serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": null } }).to_string());
                    return Ok(false);
                }
            }
        }

        info!(event="AUTO_PILOT_SUCCESS", service=%svc_name, "✅ [{}] updated and verified successfully.", svc_name);
        let _ = self.tx.send(serde_json::json!({ "type": "update_progress", "data": { "service": svc_name, "progress": null } }).to_string());

        Ok(true)
    }

    pub async fn force_update_service(&self, svc_name: &str) -> Result<String> {
        info!(event="FORCE_UPDATE_TRIGGERED", node.name=%self.node_name, service=%svc_name, "⚡ Force update triggered for: [{}]", svc_name);
        match self.check_and_update_service(svc_name).await {
            Ok(updated) => Ok(if updated {
                "Updated.".into()
            } else {
                "Already up to date, restarted.".into()
            }),
            Err(e) => {
                error!(event="FORCE_UPDATE_FAIL", node.name=%self.node_name, service=%svc_name, error=%e, "❌ Force update failed for [{}]", svc_name);
                Err(e)
            }
        }
    }
}
