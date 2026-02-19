use bollard::Docker;
// UYARI DÜZELTME: Sadece kullanılanları import et
use bollard::container::{StopContainerOptions, RemoveContainerOptions, Config, CreateContainerOptions, StartContainerOptions};
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;
use anyhow::Result;
use tracing::info;

#[derive(Clone)]
pub struct DockerAdapter {
    client: Docker,
}

impl DockerAdapter {
    pub fn new(socket: &str) -> Result<Self> {
        let client = Docker::connect_with_unix(socket, 120, bollard::API_DEFAULT_VERSION)
            .or_else(|_| Docker::connect_with_local_defaults())
            .map_err(|e| anyhow::anyhow!("Docker Connect Error: {}", e))?;
        Ok(Self { client })
    }

    pub fn get_client(&self) -> Docker {
        self.client.clone()
    }

    /// Servisi güncelle (Pull -> Stop -> Remove -> Create -> Start)
    pub async fn update_service(&self, svc_name: &str) -> Result<String> {
        info!("🔄 Atomic update initiated for: {}", svc_name);
        let docker = &self.client;

        // 1. Inspect
        let inspect = docker.inspect_container(svc_name, None).await?;
        let image_name = inspect.config.as_ref().and_then(|c| c.image.clone())
            .ok_or_else(|| anyhow::anyhow!("Image definition not found"))?;

        // 2. Pull
        let mut stream = docker.create_image(Some(CreateImageOptions { 
            from_image: image_name.clone(), ..Default::default() 
        }), None, None);
        
        while let Some(res) = stream.next().await {
            if let Err(e) = res { return Err(anyhow::anyhow!("Pull failed: {}", e)); }
        }

        // 3. Recreate Config (Identity Preservation)
        let config = Config {
            image: Some(image_name),
            env: inspect.config.as_ref().and_then(|c| c.env.clone()),
            labels: inspect.config.as_ref().and_then(|c| c.labels.clone()),
            host_config: inspect.host_config.clone(),
            networking_config: inspect.network_settings.as_ref().and_then(|n| {
                Some(bollard::container::NetworkingConfig { endpoints_config: n.networks.clone().unwrap_or_default() })
            }),
            ..Default::default()
        };

        // 4. Swap
        let _ = docker.stop_container(svc_name, Some(StopContainerOptions { t: 10 })).await;
        let _ = docker.remove_container(svc_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
        docker.create_container(Some(CreateContainerOptions { name: svc_name.to_string(), platform: None }), config).await?;
        docker.start_container(svc_name, None::<StartContainerOptions<String>>).await?;

        Ok(format!("{} updated successfully.", svc_name))
    }
}