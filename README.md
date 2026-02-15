# 📟 Sentiric Orchestrator

[![Status](https://img.shields.io/badge/status-active-success.svg)]()
[![Version](https://img.shields.io/badge/version-0.3.6-blue.svg)]()
[![License](https://img.shields.io/badge/license-AGPL--3.0-orange.svg)]()

**Sentiric Orchestrator**, Sentiric Mesh ekosistemi için tasarlanmış, Rust tabanlı, yüksek performanslı ve otonom bir **Konteyner Yaşam Döngüsü Yöneticisidir (Lifecycle Manager)**. 

Geleneksel araçların aksine, kaba kuvvet (brute-force) yerine **Native Docker API** kullanarak konteynerlerin imaj güncellemelerini yapar, konfigürasyonlarını (Environment, Volumes, Networks) korur ve en önemlisi **Docker Compose kimliğini (Identity Preservation)** asla bozmaz.

## 🎯 Temel Yetenekler

1.  **Native Docker Orchestration:** Dışarıdan kabuk komutu (Shell/Makefile) çalıştırmaz. Tüm işlemler `bollard` (Docker SDK) üzerinden atomik olarak yürütülür.
2.  **Identity Preservation:** Konteyner yeniden yaratılırken tüm `Docker-Compose` etiketlerini (labels) kopyalar. VS Code, Portainer gibi araçlarda sistem bütünlüğü korunur.
3.  **Zero-Trust Pull Model:** Dışarıya port açmaya gerek duymaz. İçeriden registry (GHCR) kontrolü yaparak güncellemeleri yönetir.
4.  **Embedded Command Center:** Kendi içinde gömülü, ultra hafif bir Web UI ile tüm node'daki servisleri anlık izlemenizi sağlar.
5.  **Fault-Tolerant Re-Deployment:** Yeni imaj başarılı bir şekilde çekilemezse (Pull Fail), mevcut çalışan konteynere dokunmaz; sistem kesintisini önler.

## 🛠️ Teknoloji Yığını

*   **Core:** Rust (Tokio & Axum)
*   **Engine:** Bollard (Native Docker Engine API)
*   **UI:** Vanilla JS + CSS (Embedded into binary)
*   **Protocol:** gRPC (Ingest) & HTTP (Portal)

## 🔌 Harmonik Bağlantı Standartları (Layer 11)

Sentiric Anayasası gereği bu servis aşağıdaki ağ topolojisine kilitlenmiştir:

*   **Statik IP:** `10.88.11.8`
*   **HTTP Portal:** `11080`
*   **gRPC Ingest:** `11081`
*   **Metrics:** `11082`

## 🚀 Hızlı Başlangıç (Infrastructure)

`sentiric-infrastructure` içinde bu servisi şu şekilde tanımlayın:

```yaml
orchestrator-service:
  image: ghcr.io/sentiric/sentiric-orchestrator:latest
  container_name: orchestrator-service
  volumes:
    - /var/run/docker.sock:/var/run/docker.sock
  environment:
    - RUST_LOG=info
    - SERVICE_IGNORE=true
    - ORCHESTRTOR_SERVICE_HTTP_PORT=11080
  networks:
    sentiric-net:
      ipv4_address: 10.88.11.8
  ports:
    - "11080:11080"
  restart: always
```

## 📖 Kullanım Rehberi

1.  **Dashboard:** `http://localhost:11080` adresinden mevcut konteynerleri ve SHA-ID'lerini izleyin.
2.  **Manual Update:** Bir servisi güncellemek için yanındaki **PULL & RESTART** butonuna basın.
3.  **AI Export:** Sağ üstteki export butonunu kullanarak tüm sistem durumunu analiz için LLM'lere besleyin.

## ⚖️ Lisans

Bu proje **GNU Affero General Public License v3.0 (AGPL-3.0)** ile lisanslanmıştır.

---
© 2026 Sentiric Team | The Iron Core v2.0 Standard
