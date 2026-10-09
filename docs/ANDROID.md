# Native Android Node Architecture & Operating Model

**Target OS:** Android 14+ (API 34+)  
**App Stack:** Kotlin + Jetpack Compose + Native Rust Core (via JNI / Uniffi)  

---

## 1. Operating Requirements & Lifecycle

Mobile devices present strict battery and memory management constraints. An Android node cannot behave like an unconstrained server.

### Foreground Service Model
- **Foreground Service Type:** `dataSync` or `specialUse` (with explicit user notification).
- **Persistent Notification:** Shows live active storage stats, network mode (Wi-Fi only vs Mobile), and pause button.
- **Wake Lock Strategy:** Partial wake locks only during active shard transfer or Merkle audit verification.

### Storage Architecture
- **App-Specific Internal/External Storage:** Default shard storage in `context.getExternalFilesDir(null)` or internal sandbox.
- **Storage Access Framework (SAF):** User can optionally select an SD card or specific folder.
- **Quota Clamping:** Enforces strict 1–3% storage ceiling, automatically backing off if host device free storage drops below 10%.

### Network & Power Policy
- Option to operate **Only when charging** or **Only on unmetered Wi-Fi**.
- Graceful pause on network disconnection without forfeiting node trust.
