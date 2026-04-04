# 🧬 Nexus Orchestration & Governance Logic

Bu belge, Sentiric Orchestrator'ın (Nexus) otonom konteyner yönetimi ve mimari denetim algoritmalarını açıklar.

## 1. Mimari Tasarım (Hexagonal)
Orchestrator, Control Plane'de çalışır ve kesin sınırlarla ayrılmıştır:
* **Core (Domain):** `ServiceInstance` ve `NodeStats` modellerini barındırır. İş mantığı buradadır.
* **Adapters:** 
  * `DockerAdapter`: Bollard üzerinden `/var/run/docker.sock` ile konuşur. Container yaşam döngüsünü (Drain/Kill/Create) yönetir.
  * `SystemAdapter`: `sysinfo` ve `nvidia-smi` üzerinden donanım telemetrisini toplar.
* **Ports/API:** Web UI için WebSocket ve JSON Raporlama için HTTP client.

## 2. Auto-Pilot ve Self-Healing Mantığı
* **Yoklama (Polling):** Orchestrator, `AUTO_PILOT_SERVICES` listesindeki servisleri periyodik olarak GHCR (Registry) ile kıyaslar.
* **Atomic Update (Zero-Downtime):** Yeni bir `SHA256` digest tespit edilirse süreç başlar: `Pull` -> `Create New Config` -> `Stop Old (Graceful Drain: 60s)` -> `Remove Old` -> `Start New`.
* **Kilitlenme Koruması (Deadlock Prevention):** Orkestratör kendini ASLA otonom olarak güncellemez. Kendini güncellemesi (İntihar riski) dışarıdan yapılmalıdır.

## 3. Resource Guards (Kaynak Koruyucuları)
Sistem sağlığını korumak için sert eşikler (Thresholds) uygulanır:
* **Memory (OOM):** Bir konteyner node'un RAM kapasitesinin %80'ini aşarsa `HealthStatus::RiskOom` statüsüne geçer.
* **GPU Hiyerarşisi:** GPU kullanan servisler (LLM, STT, TTS) yeniden başlatılırken öncelikli donanım kilitlerini (`devices` rezervasyonu) kaybetmemelidir.