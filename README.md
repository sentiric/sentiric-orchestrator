# 💠 SENTIRIC ORCHESTRATOR v5.3 (NEXUS)

[![Status](https://img.shields.io/badge/status-active-neon_green.svg)]()
[![Protocol](https://img.shields.io/badge/protocol-Dual_Stack_(HTTP/gRPC)-blue.svg)]()
[![Architecture](https://img.shields.io/badge/arch-x86__64%20%7C%20arm64-blueviolet.svg)]()

**Sentiric Orchestrator**, Sentiric Mesh ekosistemi için tasarlanmış; otonom konteyner yönetimi, canlı telemetri izleme ve merkezi raporlama sağlayan **Edge Cluster Manager** servisidir.

Observer servisinden farklı olarak, bu servis **Kontrol Uçağı (Control Plane)** üzerinde çalışır ve sistemin genel sağlığından sorumludur.

---

## 🔌 Ağ Topolojisi ve Portlar (Layer 11 Standard)

Sentiric Anayasası gereği port dağılımı şöyledir:

| Port | Protokol | Servis | Açıklama |
| :--- | :--- | :--- | :--- |
| **11080** | `HTTP/WS` | **Nexus Portal** | Web UI, REST API ve Upstream JSON Raporlama. |
| **11081** | `gRPC` | **Mesh Grid** | *(Rezerve)* Node-to-Node şifreli komut hattı (Protobuf). |
| **11082** | `HTTP` | **Metrics** | Prometheus Scrape Endpoint (`/metrics`). |

---

## 🚀 Özellikler

### 1. 🎛️ Nexus Dashboard (UI)
*   **Canlı İzleme:** WebSocket üzerinden <50ms gecikme ile CPU/RAM/GPU takibi.
*   **X-Ray Vision:** Konteynerlerin `ENV`, `Mounts` ve `Network` detaylarını arayüzden inceleme.
*   **Live Terminal:** Konteyner loglarını (tail -f) web üzerinden izleme.

### 2. 🤖 Auto-Pilot & Self-Healing
*   Belirlenen kritik servisleri (`AUTO_PILOT_SERVICES`) sürekli izler.
*   Registry'de (GHCR) yeni imaj varsa: **Pull -> Atomic Recreate -> Health Check** döngüsünü işletir.
*   Docker Compose etiketlerini ve ağ ayarlarını korur.

### 3. 📡 Upstream Uplink
*   Orchestrator, topladığı tüm verileri (Node Stats + Service List) belirlenen `UPSTREAM_ORCHESTRATOR_URL` adresine periyodik olarak postalar (JSON).
*   Bu sayede Merkezi Yönetim Paneli (Master Node), uçtaki binlerce node'un durumunu bilir.

### 4. 🧹 The Janitor
*   Disk şişmesini önlemek için "Dangling Images" ve "Stopped Containers" temizliğini tek tıkla yapar.

---

## 🛠️ Kurulum (Infrastructure)

`sentiric-infrastructure` içinde kullanım standardı:

```yaml
  orchestrator-service:
    image: ghcr.io/sentiric/sentiric-orchestrator:latest
    container_name: orchestrator-service
    network_mode: host # Host metrikleri ve doğrudan erişim için ZORUNLU
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro # Docker API Erişimi
    environment:
      # --- Identity ---
      - ENV=production
      - NODE_NAME=${NODE_HOSTNAME} # Örn: GCP-IOWA-GW-01
      
      # --- Network ---
      - HOST=0.0.0.0
      - HTTP_PORT=11080
      - GRPC_PORT=11081 # Gelecek kullanım için rezerve
      
      # --- Auto-Pilot ---
      # Otomatik güncellenecek servisler (Virgülle ayrılmış)
      # Başlangıçta otomaik olarak takip edilecek servisler yazılabilir
      # Yada UI aracılığı ile AutoPilot her servis için manul seçilebilir.
      # - AUTO_PILOT_SERVICES=media-service,observer-service,proxy-service
      - POLL_INTERVAL=30 # 30 saniyede bir registry kontrolü
      
      # --- Upstream (Reporting) ---
      # Bu node'un rapor göndereceği Ana Merkez (Master Orchestrator)
      # Boş bırakılırsa "Standalone" modda çalışır.
      # Format: HTTP Post URL'i
      - UPSTREAM_ORCHESTRATOR_URL=http://master-node-ip:11080/api/ingest/node-report
      
    restart: always
```

## 🧠 AI & Debugging
*   **AI Export:** Arayüzdeki "AI EXPORT" butonu, sistemin o anki tüm röntgenini (Loglar, Hatalar, Versiyonlar) tek bir `.md` dosyası olarak indirir. Bu dosya LLM'lere (Claude/GPT) analiz için verilebilir.

---

## ⚖️ Lisans
Copyright © 2026 Sentiric Technologies.
