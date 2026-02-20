use bollard::Docker;
use bollard::container::{StopContainerOptions, RemoveContainerOptions, Config, CreateContainerOptions, StartContainerOptions, InspectContainerOptions, RestartContainerOptions, LogsOptions, LogOutput, Stats, StatsOptions, PruneContainersOptions};
use bollard::image::{CreateImageOptions, PruneImagesOptions};
use futures_util::{StreamExt, Stream};
use anyhow::Result;
use tracing::{info, error, debug, warn};
use std::default::Default;

#[derive(Clone)]
pub struct DockerAdapter {
    client: Docker,
    node_name: String,
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
        info!("▶️ Starting: {}", svc_id);
        self.client.start_container(svc_id, None::<StartContainerOptions<String>>).await?;
        Ok(())
    }

    pub async fn stop_service(&self, svc_id: &str) -> Result<()> {
        info!("🛑 Stopping: {}", svc_id);
        self.client.stop_container(svc_id, Some(StopContainerOptions { t: 10 })).await?;
        Ok(())
    }
    
    pub async fn restart_service(&self, svc_id: &str) -> Result<()> {
        info!("🔄 Restarting: {}", svc_id);
        self.client.restart_container(svc_id, Some(RestartContainerOptions { t: 10 })).await?;
        Ok(())
    }

    // --- INFO & LOGS ---
    pub fn get_log_stream(&self, svc_id: &str) -> impl Stream<Item = Result<LogOutput, bollard::errors::Error>> {
        let options = Some(LogsOptions::<String>{
            follow: true, stdout: true, stderr: true, tail: "200".to_string(), ..Default::default()
        });
        self.client.logs(svc_id, options)
    }

    pub async fn get_logs_snapshot(&self, svc_id: &str) -> String {
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
        let options = Some(StatsOptions { stream: false, one_shot: true });
        let mut stream = self.client.stats(svc_id, options);
        if let Some(result) = stream.next().await {
            return result.map_err(|e| anyhow::anyhow!("Stats error: {}", e));
        }
        Err(anyhow::anyhow!("No stats received"))
    }

    pub async fn inspect_service(&self, svc_id: &str) -> Result<bollard::models::ContainerInspectResponse> {
        self.client.inspect_container(svc_id, None::<InspectContainerOptions>).await
            .map_err(|e| anyhow::anyhow!("Inspect error: {}", e))
    }

    // THE JANITOR (Generic Fix Applied Here)
    pub async fn prune_system(&self) -> Result<String> {
        let c_prune = self.client.prune_containers(None::<PruneContainersOptions<String>>).await?;
        let c_deleted = c_prune.containers_deleted.unwrap_or_default().len();

        let i_prune = self.client.prune_images(None::<PruneImagesOptions<String>>).await?;
        let i_deleted = i_prune.images_deleted.unwrap_or_default().len();
        let space = i_prune.space_reclaimed.unwrap_or(0);

        let msg = format!("♻️ JANITOR REPORT: Deleted {} Containers, {} Images. Reclaimed {:.2} MB", 
            c_deleted, i_deleted, (space as f64 / 1024.0 / 1024.0));
        info!("{}", msg);
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
        
        debug!("🔍 [{}] Checking updates: {}", svc_name, image_name);

        // 1. PULL (Herkes için yapılır)
        let mut stream = docker.create_image(Some(CreateImageOptions { from_image: image_name.clone(), ..Default::default() }), None, None);
        while let Some(res) = stream.next().await {
            if let Err(e) = res { 
                error!("❌ [{}] Pull Error: {}", svc_name, e);
                return Err(anyhow::anyhow!("Registry error")); 
            }
        }

        // 2. COMPARE
        let new_image_inspect = docker.inspect_image(&image_name).await?;
        let new_image_id = new_image_inspect.id.clone().unwrap_or_default();

        if current_image_id == new_image_id { return Ok(false); }

        info!("🚀 [{}] UPDATE FOUND: {} -> {}", svc_name, &current_image_id[..12], &new_image_id[..12]);

        // --- KRİTİK MÜDAHALE ---
        if is_self {
            // Eğer kendisi ise ASLA Stop/Remove yapma.
            // Sadece log bas ve kullanıcıyı uyar.
            warn!("⚠️ SELF-UPDATE PREVENTED: Orchestrator cannot restart itself autonomously.");
            warn!("👉 ACTION REQUIRED: Image pulled. Please run 'docker restart {}' manually.", svc_name);
            
            // "True" dönüyoruz ki UI güncellendiğini bilsin (ama restart beklesin)
            return Ok(true); 
        }
        // -----------------------

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

        info!("🛑 Stopping: {}", svc_name);
        let _ = docker.stop_container(svc_name, Some(StopContainerOptions { t: 10 })).await;
        
        info!("🗑️ Removing: {}", svc_name);
        let _ = docker.remove_container(svc_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
        
        info!("✨ Re-Creating: {}", svc_name);
        docker.create_container(Some(CreateContainerOptions { name: svc_name.to_string(), platform: None }), config).await?;
        
        info!("🚀 Starting: {}", svc_name);
        docker.start_container(svc_name, None::<StartContainerOptions<String>>).await?;

        info!("✅ [{}] Updated successfully.", svc_name);
        Ok(true)
    }

    pub async fn force_update_service(&self, svc_name: &str) -> Result<String> {
        match self.check_and_update_service(svc_name).await {
            Ok(updated) => Ok(if updated { "Updated.".into() } else { "Already up to date, restarted.".into() }),
            Err(e) => Err(e)
        }
    }
}