# 🏛️ CLUSTER GOVERNANCE POLICY

| Status | ACTIVE |
| Owner | Sentiric Architecture Board |

## 1. Auto-Pilot Rules
- Orchestrator, "Auto-Pilot" modundaki servisleri her 60 saniyede bir registry (GHCR) ile kontrol eder.
- Eğer yeni bir `SHA256` digest varsa, **Atomic Update** başlatılır.
- **Atomic Update:** Pull -> Create New Config -> Stop Old -> Remove Old -> Start New.

## 2. Resource Guards
- **Memory:** Bir konteyner 4GB RAM'i aşarsa `Warning` statüsüne geçer.
- **GPU:** GPU kullanan servisler (LLM, STT) önceliklidir. Orchestrator bu servisleri restart ederken GPU erişimini korumalıdır.

## 3. Self-Update Limitation
- Orchestrator kendini **GÜNCELLEMEZ**.
- Kendini güncellemesi gerekiyorsa, dışarıdan bir `Watchtower` servisi veya `docker-compose pull && docker-compose up -d` komutu çalıştırılmalıdır.