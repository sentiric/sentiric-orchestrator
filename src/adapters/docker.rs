use bollard::Docker;
use bollard::container::{StopContainerOptions, RemoveContainerOptions, Config, CreateContainerOptions, StartContainerOptions};
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;
use anyhow::Result;
use tracing::{info, error}; // warn kaldırıldı

#[derive(Clone)]
pub struct DockerAdapter {
    client: Docker,
    // node_name: String, // Kullanılmıyorsa struct'tan çıkaralım temiz olsun
}

impl DockerAdapter {
    pub fn new(socket: &str, _node_name: String) -> Result<Self> {
        let client = Docker::connect_with_unix(socket, 120, bollard::API_DEFAULT_VERSION)
            .or_else(|_| Docker::connect_with_local_defaults())
            .map_err(|e| anyhow::anyhow!("Docker Bağlantı Hatası: {}", e))?;
        
        Ok(Self { client })
    }

    pub fn get_client(&self) -> Docker {
        self.client.clone()
    }

    /// Servisi güncelle (Atomic: Pull -> Stop -> Remove -> Create -> Start)
    pub async fn update_service(&self, svc_name: &str) -> Result<String> {
        info!("🔄 [ATOMIC UPDATE] İşlem Başlatılıyor: {}", svc_name);
        let docker = &self.client;

        // 1. Inspect
        let inspect = docker.inspect_container(svc_name, None).await
            .map_err(|e| anyhow::anyhow!("Servis bulunamadı: {}", e))?;
        
        let image_name = inspect.config.as_ref().and_then(|c| c.image.clone())
            .ok_or_else(|| anyhow::anyhow!("Imaj tanımı yok"))?;

        info!("📥 Pulling Latest Image: {}", image_name);

        // 2. Pull (Hata verirse durur, mevcut konteyner bozulmaz)
        let mut stream = docker.create_image(Some(CreateImageOptions { 
            from_image: image_name.clone(), ..Default::default() 
        }), None, None);
        
        while let Some(res) = stream.next().await {
            if let Err(e) = res { 
                error!("❌ Pull Hatası (Update İptal): {}", e);
                return Err(anyhow::anyhow!("İmaj çekilemedi.")); 
            }
        }

        // 3. Config Preservation
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

        // 4. Swap
        info!("🛑 Stopping: {}", svc_name);
        let _ = docker.stop_container(svc_name, Some(StopContainerOptions { t: 10 })).await;
        
        info!("🗑️ Removing: {}", svc_name);
        let _ = docker.remove_container(svc_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
        
        info!("✨ Re-Creating: {}", svc_name);
        docker.create_container(Some(CreateContainerOptions { name: svc_name.to_string(), platform: None }), config).await?;
        
        info!("🚀 Starting: {}", svc_name);
        docker.start_container(svc_name, None::<StartContainerOptions<String>>).await?;

        Ok(format!("✅ {} başarıyla güncellendi.", svc_name))
    }
}