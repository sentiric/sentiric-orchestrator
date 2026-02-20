use bollard::Docker;
use bollard::container::{StopContainerOptions, RemoveContainerOptions, Config, CreateContainerOptions, StartContainerOptions, InspectContainerOptions, RestartContainerOptions, LogsOptions, LogOutput}; // LogOutput eklendi
use bollard::image::CreateImageOptions;
use futures_util::{StreamExt, Stream};
use anyhow::Result;
use tracing::{info, error, debug};
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
    
    // YENİ: Servisi Başlat
    pub async fn start_service(&self, svc_id: &str) -> Result<()> {
        info!("▶️ Starting: {}", svc_id);
        self.client.start_container(svc_id, None::<StartContainerOptions<String>>).await?;
        Ok(())
    }

    // YENİ: Servisi Durdur
    pub async fn stop_service(&self, svc_id: &str) -> Result<()> {
        info!("🛑 Stopping: {}", svc_id);
        self.client.stop_container(svc_id, Some(StopContainerOptions { t: 10 })).await?;
        Ok(())
    }
    
    // YENİ: Servisi Yeniden Başlat
    pub async fn restart_service(&self, svc_id: &str) -> Result<()> {
        info!("🔄 Restarting: {}", svc_id);
        self.client.restart_container(svc_id, Some(RestartContainerOptions { t: 10 })).await?;
        Ok(())
    }
    
    // YENİ & DÜZELTİLDİ: Log Akışı (Stream)
    pub fn get_log_stream(&self, svc_id: &str) -> impl Stream<Item = Result<LogOutput, bollard::errors::Error>> {
        let options = Some(LogsOptions::<String>{
            follow: true,
            stdout: true,
            stderr: true,
            tail: "100".to_string(), // Son 100 satırı göster
            ..Default::default()
        });
        self.client.logs(svc_id, options)
    }

    /// Servisi güncelle (Atomic: Pull -> Compare -> (Stop -> Remove -> Create -> Start))
    /// Return: true (güncellendi), false (değişiklik yok), Err (hata)
    pub async fn check_and_update_service(&self, svc_name: &str) -> Result<bool> {
        let docker = &self.client;

        // 1. Mevcut Konteyneri İncele
        let inspect = docker.inspect_container(svc_name, None::<InspectContainerOptions>).await
            .map_err(|e| anyhow::anyhow!("Servis bulunamadı: {}", e))?;
        
        let current_image_id = inspect.image.clone().unwrap_or_default();
        
        let image_name = inspect.config.as_ref().and_then(|c| c.image.clone())
            .ok_or_else(|| anyhow::anyhow!("Imaj tanımı yok"))?;

        if svc_name.contains("orchestrator") { return Ok(false); }

        debug!("🔍 [{}] Checking for updates on image: {}", svc_name, image_name);

        let mut stream = docker.create_image(Some(CreateImageOptions { 
            from_image: image_name.clone(), ..Default::default() 
        }), None, None);
        
        while let Some(res) = stream.next().await {
            if let Err(e) = res { 
                error!("❌ [{}] Pull Hatası: {}", svc_name, e);
                return Err(anyhow::anyhow!("Registry erişim hatası.")); 
            }
        }

        let new_image_inspect = docker.inspect_image(&image_name).await
            .map_err(|e| anyhow::anyhow!("Imaj inspect hatası: {}", e))?;
        
        let new_image_id = new_image_inspect.id.clone().unwrap_or_default();

        if current_image_id == new_image_id {
            let c_short = if current_image_id.len() > 12 { &current_image_id[..12] } else { &current_image_id };
            debug!("✅ [{}] Zaten güncel. (ID: {})", svc_name, c_short);
            return Ok(false);
        }

        let c_short = if current_image_id.len() > 12 { &current_image_id[..12] } else { &current_image_id };
        let n_short = if new_image_id.len() > 12 { &new_image_id[..12] } else { &new_image_id };

        info!("🚀 [{}] GÜNCELLEME TESPİT EDİLDİ! Eski: {} -> Yeni: {}", svc_name, c_short, n_short);

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

        info!("✅ [{}] Başarıyla güncellendi.", svc_name);
        Ok(true)
    }

    pub async fn force_update_service(&self, svc_name: &str) -> Result<String> {
        match self.check_and_update_service(svc_name).await {
            Ok(updated) => Ok(if updated { "Güncellendi.".into() } else { "Zaten güncel, yeniden başlatıldı.".into() }),
            Err(e) => Err(e)
        }
    }
}