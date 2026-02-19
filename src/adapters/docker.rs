use bollard::Docker;
use bollard::container::{StopContainerOptions, RemoveContainerOptions, Config, CreateContainerOptions, StartContainerOptions};
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;
use anyhow::Result;
use tracing::{info, error, warn};

#[derive(Clone)]
pub struct DockerAdapter {
    client: Docker,
    node_name: String, // Loglama için node ismini tutalım
}

impl DockerAdapter {
    pub fn new(socket: &str, node_name: String) -> Result<Self> {
        let client = Docker::connect_with_unix(socket, 120, bollard::API_DEFAULT_VERSION)
            .or_else(|_| Docker::connect_with_local_defaults())
            .map_err(|e| anyhow::anyhow!("Docker Bağlantı Hatası: {}", e))?;
        
        // Bağlantıyı test et (Ping)
        // Not: new() async olmadığı için ping'i burada yapamıyoruz ama client oluştuysa genelde iyidir.
        Ok(Self { client, node_name })
    }

    pub fn get_client(&self) -> Docker {
        self.client.clone()
    }

    /// Servisi güncelle (Atomic: Pull -> Stop -> Remove -> Create -> Start)
    pub async fn update_service(&self, svc_name: &str) -> Result<String> {
        info!("🔄 [ATOMIC UPDATE] Başlatılıyor: {}", svc_name);
        let docker = &self.client;

        // 1. Mevcut Konfigürasyonu Yedekle (Snapshot)
        let inspect = docker.inspect_container(svc_name, None).await
            .map_err(|e| anyhow::anyhow!("Servis bulunamadı veya erişilemiyor: {}", e))?;
        
        let image_name = inspect.config.as_ref().and_then(|c| c.image.clone())
            .ok_or_else(|| anyhow::anyhow!("Imaj tanımı bulunamadı"))?;

        info!("📥 Pulling Image: {}", image_name);

        // 2. Yeni İmajı Çek (PULL) - Bu başarısız olursa işlem iptal edilir, servis bozulmaz.
        let mut stream = docker.create_image(Some(CreateImageOptions { 
            from_image: image_name.clone(), ..Default::default() 
        }), None, None);
        
        while let Some(res) = stream.next().await {
            if let Err(e) = res { 
                error!("❌ Pull Hatası: {}", e);
                return Err(anyhow::anyhow!("İmaj çekilemedi, işlem iptal edildi. Mevcut servis çalışmaya devam ediyor.")); 
            }
        }

        // 3. Konfigürasyonu Hazırla (Identity Preservation)
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

        // 4. Kritik Bölge (Swap)
        info!("🛑 Stopping old container: {}", svc_name);
        let _ = docker.stop_container(svc_name, Some(StopContainerOptions { t: 10 })).await;
        
        info!("🗑️ Removing old container: {}", svc_name);
        let _ = docker.remove_container(svc_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
        
        info!("✨ Creating new container: {}", svc_name);
        match docker.create_container(Some(CreateContainerOptions { name: svc_name.to_string(), platform: None }), config).await {
            Ok(_) => {
                info!("🚀 Starting new container: {}", svc_name);
                docker.start_container(svc_name, None::<StartContainerOptions<String>>).await?;
                Ok(format!("✅ {} başarıyla güncellendi ve yeniden başlatıldı.", svc_name))
            },
            Err(e) => {
                // Burası felaket senaryosudur. Eski silindi, yeni yaratılamadı.
                // Manuel müdahale gerekebilir ama biz hatayı net dönelim.
                error!("🔥 FATAL: Konteyner yaratılamadı! Servis şu an kapalı: {}", e);
                Err(anyhow::anyhow!("Kritik Hata: Konteyner yaratılamadı: {}", e))
            }
        }
    }
}