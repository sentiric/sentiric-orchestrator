# 🏗️ ORCHESTRATOR NEXUS ARCHITECTURE

## Mimari: Hexagonal (Ports & Adapters)

### 1. Core (Domain)
- `ServiceInstance`: Bir mikroservisin anlık durumu.
- `NodeStats`: Fiziksel sunucunun sağlığı.

### 2. Adapters
- **Docker Adapter:** Bollard kütüphanesi ile Docker Socket (`/var/run/docker.sock`) üzerinden konuşur.
- **System Adapter:** `sysinfo` ve `nvidia-smi` üzerinden donanım verisi toplar.

### 3. API
- **HTTP/WebSocket (Axum):** UI ve anlık veri akışı için.
- **gRPC (Tonic):** Gelecekteki Node-to-Node iletişim için (Mesh).

## Veri Akışı
1. `Scanner Loop` 5 saniyede bir Docker'ı tarar.
2. Değişiklik varsa `Broadcast Channel` üzerinden WebSocket'e basar.
3. UI (React/Vanilla JS) bu veriyi alıp `Grid` üzerinde gösterir.