use bollard::Docker;
use bollard::container::{StopContainerOptions, RemoveContainerOptions, Config, CreateContainerOptions, StartContainerOptions, InspectContainerOptions};
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;
use anyhow::Result;
use tracing::{info, error, debug}; // warn kaldırıldı, unused uyarısı için

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

    /// Servisi güncelle (Atomic: Pull -> Compare -> (Stop -> Remove -> Create -> Start))
    /// Return: true (güncellendi), false (değişiklik yok), Err (hata)
    pub async fn check_and_update_service(&self, svc_name: &str) -> Result<bool> {
        let docker = &self.client;

        // 1. Mevcut Konteyneri İncele
        let inspect = docker.inspect_container(svc_name, None::<InspectContainerOptions>).await
            .map_err(|e| anyhow::anyhow!("Servis bulunamadı: {}", e))?;
        
        // FIX: Option<String> -> String dönüşümü yapıldı
        let current_image_id = inspect.image.clone().unwrap_or_default();
        
        let image_name = inspect.config.as_ref().and_then(|c| c.image.clone())
            .ok_or_else(|| anyhow::anyhow!("Imaj tanımı yok"))?;

        // Orchestrator kendini güncellerse döngüye girer, bunu engelle
        if svc_name.contains("orchestrator") {
            return Ok(false);
        }

        debug!("🔍 [{}] Checking for updates on image: {}", svc_name, image_name);

        // 2. Yeni Imajı Çek (Pull)
        let mut stream = docker.create_image(Some(CreateImageOptions { 
            from_image: image_name.clone(), ..Default::default() 
        }), None, None);
        
        while let Some(res) = stream.next().await {
            if let Err(e) = res { 
                error!("❌ [{}] Pull Hatası: {}", svc_name, e);
                return Err(anyhow::anyhow!("Registry erişim hatası.")); 
            }
        }

        // 3. Imaj ID Kontrolü (Inspect Image)
        let new_image_inspect = docker.inspect_image(&image_name).await
            .map_err(|e| anyhow::anyhow!("Imaj inspect hatası: {}", e))?;
        
        // FIX: Option<String> -> String dönüşümü
        let new_image_id = new_image_inspect.id.clone().unwrap_or_default();

        // String karşılaştırması artık güvenli
        if current_image_id == new_image_id {
            // Loglarken slice almadan önce uzunluk kontrolü yapmak güvenlidir ama Docker ID'leri uzundur.
            // Yine de güvenli slice alalım.
            let c_short = if current_image_id.len() > 12 { &current_image_id[..12] } else { &current_image_id };
            debug!("✅ [{}] Zaten güncel. (ID: {})", svc_name, c_short);
            return Ok(false);
        }

        // Güvenli slice alımı
        let c_short = if current_image_id.len() > 12 { &current_image_id[..12] } else { &current_image_id };
        let n_short = if new_image_id.len() > 12 { &new_image_id[..12] } else { &new_image_id };

        info!("🚀 [{}] GÜNCELLEME TESPİT EDİLDİ! Eski: {} -> Yeni: {}", svc_name, c_short, n_short);

        // 4. Update Sequence
        // Config Preservation
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

    // Manual Force Update (API için)
    pub async fn force_update_service(&self, svc_name: &str) -> Result<String> {
        match self.check_and_update_service(svc_name).await {
            Ok(updated) => Ok(if updated { "Güncellendi.".into() } else { "Zaten güncel, yeniden başlatıldı.".into() }),
            Err(e) => Err(e)
        }
    }
}