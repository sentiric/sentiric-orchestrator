// src/adapters/docker.rs
use bollard::Docker;
use bollard::container::{
    StopContainerOptions, RemoveContainerOptions, Config, CreateContainerOptions, 
    StartContainerOptions, InspectContainerOptions, RestartContainerOptions, 
    LogsOptions, LogOutput, Stats, StatsOptions, PruneContainersOptions
};
use bollard::image::{CreateImageOptions, PruneImagesOptions};
use futures_util::{StreamExt, Stream};
use anyhow::Result;
use tracing::{info, error, debug, warn};
use std::default::Default;

#[derive(Clone)]
pub struct DockerAdapter {
    client: Docker,
    node_name: String, // Artık loglarda aktif olarak kullanılıyor
}

impl DockerAdapter {
    pub fn new(socket: &str, node_name: String) -> Result<Self> {
        let client = Docker::connect_with_unix(socket, 120, bollard::API_DEFAULT_VERSION)
            .or_else(|_| Docker::connect_with_local_defaults())
            .map_err(|e| anyhow::anyhow!("Docker Bağlantı Hatası: {}", e))?;
        
        Ok(Self { client, node_name })
    }

    pub fn get_client(&self) -> Docker {
        self.client.clone()
    }
    
    // --- LIFECYCLE ---
    pub async fn start_service(&self, svc_id: &str) -> Result<()> {
        info!(event="CONTAINER_START", node.name=%self.node_name, container.id=%svc_id, "▶️ Starting container: {}", svc_id);
        self.client.start_container(svc_id, None::<StartContainerOptions<String>>).await?;
        Ok(())
    }

    pub async fn stop_service(&self, svc_id: &str) -> Result<()> {
        info!(event="CONTAINER_STOP", node.name=%self.node_name, container.id=%svc_id, "🛑 Stopping container: {}", svc_id);
        self.client.stop_container(svc_id, Some(StopContainerOptions { t: 10 })).await?;
        Ok(())
    }
    
    pub async fn restart_service(&self, svc_id: &str) -> Result<()> {
        info!(event="CONTAINER_RESTART", node.name=%self.node_name, container.id=%svc_id, "🔄 Restarting container: {}", svc_id);
        self.client.restart_container(svc_id, Some(RestartContainerOptions { t: 10 })).await?;
        Ok(())
    }

    // --- INFO & LOGS ---
    pub fn get_log_stream(&self, svc_id: &str) -> impl Stream<Item = Result<LogOutput, bollard::errors::Error>> {
        debug!(event="STREAM_LOGS", node.name=%self.node_name, container.id=%svc_id, "📡 Opening live log stream for container: {}", svc_id);
        let options = Some(LogsOptions::<String>{
            follow: true, stdout: true, stderr: true, tail: "200".to_string(), ..Default::default()
        });
        self.client.logs(svc_id, options)
    }

    pub async fn get_logs_snapshot(&self, svc_id: &str) -> String {
        debug!(event="SNAPSHOT_LOGS", node.name=%self.node_name, container.id=%svc_id, "📸 Fetching log snapshot for container: {}", svc_id);
        let options = Some(LogsOptions::<String>{
            follow: false, stdout: true, stderr: true, tail: "50".to_string(), ..Default::default()
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
        let options = Some(StatsOptions { stream: false, one_shot: true });
        let mut stream = self.client.stats(svc_id, options);
        if let Some(result) = stream.next().await {
            return result.map_err(|e| anyhow::anyhow!("Stats error: {}", e));
        }
        Err(anyhow::anyhow!("No stats received"))
    }

    pub async fn inspect_service(&self, svc_id: &str) -> Result<bollard::models::ContainerInspectResponse> {
        debug!(event="INSPECT_CONTAINER", node.name=%self.node_name, container.id=%svc_id, "🔎 Inspecting container: {}", svc_id);
        self.client.inspect_container(svc_id, None::<InspectContainerOptions>).await
            .map_err(|e| anyhow::anyhow!("Inspect error: {}", e))
    }

    // --- THE JANITOR ---
    pub async fn prune_system(&self) -> Result<String> {
        info!(event="SYSTEM_PRUNE_START", node.name=%self.node_name, "🧹 Starting system prune...");
        let c_prune = self.client.prune_containers(None::<PruneContainersOptions<String>>).await?;
        let c_deleted = c_prune.containers_deleted.unwrap_or_default().len();

        let i_prune = self.client.prune_images(None::<PruneImagesOptions<String>>).await?;
        let i_deleted = i_prune.images_deleted.unwrap_or_default().len();
        let space = i_prune.space_reclaimed.unwrap_or(0);

        let msg = format!("Deleted {} Containers, {} Images. Reclaimed {:.2} MB", 
            c_deleted, i_deleted, (space as f64 / 1024.0 / 1024.0));
            
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

    // --- UPDATE ENGINE ---
    pub async fn check_and_update_service(&self, svc_name: &str) -> Result<bool> {
        let docker = &self.client;
        let inspect = docker.inspect_container(svc_name, None::<InspectContainerOptions>).await
            .map_err(|e| anyhow::anyhow!("Service not found: {}", e))?;
        
        let current_image_id = inspect.image.clone().unwrap_or_default();
        let image_name = inspect.config.as_ref().and_then(|c| c.image.clone())
            .ok_or_else(|| anyhow::anyhow!("No image defined"))?;

        // --- SELF-UPDATE PROTECTION ---
        let is_self = svc_name.contains("orchestrator"); 
        
        debug!(
            event="CHECK_UPDATES", 
            node.name=%self.node_name, 
            service=%svc_name, 
            image=%image_name, 
            "🔍 [{}] Checking updates for image: {}", svc_name, image_name
        );

        // 1. PULL
        let mut stream = docker.create_image(Some(CreateImageOptions { from_image: image_name.clone(), ..Default::default() }), None, None);
        while let Some(res) = stream.next().await {
            if let Err(e) = res { 
                error!(
                    event="IMAGE_PULL_FAIL", 
                    node.name=%self.node_name, 
                    service=%svc_name, 
                    error=%e, 
                    "❌ Pull Error for service [{}] with image [{}]: {}", svc_name, image_name, e
                );
                return Err(anyhow::anyhow!("Registry error")); 
            }
        }

        // 2. COMPARE
        let new_image_inspect = docker.inspect_image(&image_name).await?;
        let new_image_id = new_image_inspect.id.clone().unwrap_or_default();

        if current_image_id == new_image_id { 
            debug!(
                event="NO_UPDATE_NEEDED", 
                node.name=%self.node_name, 
                service=%svc_name, 
                "✅ Service [{}] is already running the latest image.", svc_name
            );
            return Ok(false); 
        }

        info!(
            event = "AUTO_PILOT_UPDATE_FOUND",
            node.name = %self.node_name,
            service = %svc_name,
            old.sha = %&current_image_id[..12.min(current_image_id.len())],
            new.sha = %&new_image_id[..12.min(new_image_id.len())],
            "🚀 UPDATE FOUND for service: [{}]", svc_name
        );

        if is_self {
            warn!(
                event = "SELF_UPDATE_PREVENTED",
                node.name = %self.node_name,
                service = %svc_name,
                "⚠️ SELF-UPDATE PREVENTED: Orchestrator [{}] cannot restart itself autonomously.", svc_name
            );
            return Ok(true); 
        }

        let config = Config {
            image: Some(image_name.clone()),
            env: inspect.config.as_ref().and_then(|c| c.env.clone()),
            labels: inspect.config.as_ref().and_then(|c| c.labels.clone()),
            host_config: inspect.host_config.clone(),
            networking_config: inspect.network_settings.as_ref().and_then(|n| {
                Some(bollard::container::NetworkingConfig { endpoints_config: n.networks.clone().unwrap_or_default() })
            }),
            ..Default::default()
        };

        info!(event="CONTAINER_RECREATING", node.name=%self.node_name, service=%svc_name, "🛑 Stopping & Removing old container for: [{}]", svc_name);
        let _ = docker.stop_container(svc_name, Some(StopContainerOptions { t: 10 })).await;
        let _ = docker.remove_container(svc_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
        
        info!(event="CONTAINER_CREATING", node.name=%self.node_name, service=%svc_name, "✨ Creating new container for: [{}]", svc_name);
        docker.create_container(Some(CreateContainerOptions { name: svc_name.to_string(), platform: None }), config).await?;
        
        info!(event="CONTAINER_STARTING", node.name=%self.node_name, service=%svc_name, "🚀 Starting new updated container: [{}]", svc_name);
        docker.start_container(svc_name, None::<StartContainerOptions<String>>).await?;

        info!(event="AUTO_PILOT_SUCCESS", node.name=%self.node_name, service=%svc_name, "✅ [{}] updated successfully.", svc_name);
        Ok(true)
    }

    pub async fn force_update_service(&self, svc_name: &str) -> Result<String> {
        info!(event="FORCE_UPDATE_TRIGGERED", node.name=%self.node_name, service=%svc_name, "⚡ Force update triggered for: [{}]", svc_name);
        match self.check_and_update_service(svc_name).await {
            Ok(updated) => Ok(if updated { "Updated.".into() } else { "Already up to date, restarted.".into() }),
            Err(e) => {
                error!(event="FORCE_UPDATE_FAIL", node.name=%self.node_name, service=%svc_name, error=%e, "❌ Force update failed for [{}]", svc_name);
                Err(e)
            }
        }
    }
}