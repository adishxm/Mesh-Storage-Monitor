# System Execution & Verification Guide

This document contains step-by-step instructions to compile, test, run, and verify the decentralized P2P Mesh Storage Network across all components: pure Rust storage engine, canonical libp2p node daemon, operator CLI, terminal telemetry, multi-tenant SaaS control plane, Android native bridge, and legacy Node test suite.

---

## 1. Automated Test Suites & Verification

The project is structured as a multi-crate Cargo workspace alongside an audited legacy Node reference suite.

### A. Run Full Rust Workspace Tests
Executes all unit, integration, deterministic repair, and cryptographic tests across all 6 workspace crates:

```powershell
# In PowerShell:
cargo test --workspace
```

* **Observed Result**: **120 tests passed, 0 failed**.
  - `mesh-core` (62 tests): FastCDC chunking, Reed-Solomon Galois $GF(2^8)$ erasure coding, Argon2id/AES-256-GCM encryption, deterministic IVs, Merkle DAG verification, Sybil subnet limits, and `.mbak` disaster recovery.
  - `mesh-node` (33 tests): P2P state machine, LAN 3-node cluster recovery, proof-of-storage audits, self-healing repair, loopback authentication guards, and Prometheus telemetry.
  - `mesh-control-plane` (23 tests): Bearer JWT authentication, anti-spoofing header guards, PostgreSQL migrations, row-level locking (`FOR UPDATE`), and tenant quota isolation.
  - `mesh-cli` (9 tests): Streaming request handling, passphrase resolution, and command parsing.
  - `terminal-ui` (3 tests): Live ASCII gauge rendering and ANSI progress meters.

### B. Zero-Warning Linter & Formatting Gates
Enforce strict compiler and code quality standards:

```powershell
# Clippy check (0 warnings allowed)
cargo clippy --workspace --all-targets -- -D warnings

# Code formatting check
cargo fmt --all -- --check
```

### C. Legacy Node Core Engine Test & Audit
Verify the reference JavaScript implementation and ensure zero npm vulnerabilities:

```powershell
# Run legacy core reconstruction test
npm test --prefix node_version

# Verify zero known CVEs (Multer 2.x upgraded)
npm audit --prefix node_version
```

* **Observed Result**: 4/4 tests passed (normal decode, missing shard recovery, corrupted shard reconstruction, and excessive corruption fail-closed); **0 vulnerabilities found**.

---

## 2. Testing Locally (Multi-Process on Single Machine)

Running multiple local nodes verifies P2P discovery, Noise handshake authentication, Kademlia routing, and decentralized shard distribution.

### Step A: Start Node 1 (Anchor Node)
By default, the HTTP API binds to **loopback only** (`127.0.0.1:3000`) for secure local development:

```powershell
cargo run -p mesh-node -- --port 4001 --api-port 3000 --quota 1.5
```

Terminal log indicators:
* `Local Peer ID: <PEER_ID_1>` (e.g., `12D3KooWN1Xy...`)
* `Starting HTTP API server on 127.0.0.1:3000`
* `Swarm listening on address: /ip4/127.0.0.1/tcp/4001`

### Step B: Start Node 2 (Peer Node)
Start the second node on distinct P2P and API ports:

```powershell
cargo run -p mesh-node -- --port 4002 --api-port 3001 --quota 1.5
```

Terminal log indicators:
* `Local Peer ID: <PEER_ID_2>` (e.g., `12D3KooWR45c...`)
* `Starting HTTP API server on 127.0.0.1:3001`
* `Swarm listening on address: /ip4/127.0.0.1/tcp/4002`

### Step C: Authenticated Pairing Exchange
Nodes enforce cryptographic **Noise transport allowlisting** and reject connections from unknown peers. Pair Node 1 with Node 2:

* **Using PowerShell**:
  ```powershell
  $body = @{ multiaddr = "/ip4/127.0.0.1/tcp/4002/p2p/<PEER_ID_2>" } | ConvertTo-Json
  Invoke-RestMethod -Uri "http://127.0.0.1:3000/api/v1/pair" -Method Post -ContentType "application/json" -Body $body
  ```

* **Using curl**:
  ```bash
  curl -X POST -H "Content-Type: application/json" \
       -d '{"multiaddr": "/ip4/127.0.0.1/tcp/4002/p2p/<PEER_ID_2>"}' \
       http://127.0.0.1:3000/api/v1/pair
  ```

### Step D: Verify Mesh Status
Confirm that both nodes have registered each other in their routing tables:

```bash
curl http://127.0.0.1:3000/api/v1/status
curl http://127.0.0.1:3001/api/v1/status
```

### Step E: Upload & Shard Distribution
Upload a sample file to Node 1 with Reed-Solomon parameters $k=1, m=1$:

1. Create a sample payload:
   ```powershell
   "Decentralized mesh storage payload test." > test.txt
   ```
2. Upload via multipart form:
   ```bash
   curl -F "file_id=doc-alpha" \
        -F "passphrase=correct-horse-battery" \
        -F "salt=mysalt123" \
        -F "k=1" \
        -F "m=1" \
        -F "file=@test.txt" \
        http://127.0.0.1:3000/api/v1/upload
   ```

3. Inspect shard distribution:
   ```bash
   curl http://127.0.0.1:3000/api/v1/shards
   curl http://127.0.0.1:3001/api/v1/shards
   ```
   Data and parity shards are distributed across `./data_4001/shards/` and `./data_4002/shards/`.

### Step F: Secure Download & Reconstruction

> **Security Guard:** Query-string passphrases (e.g. `?passphrase=...`) are **strictly rejected with HTTP 400 Bad Request** to prevent credential exposure in access logs, proxies, and browser histories.

Use one of the secure retrieval methods:

* **Option 1: POST with JSON Body (Recommended for scripts/web)**:
  ```bash
  curl -X POST http://127.0.0.1:3001/api/v1/download/doc-alpha \
       -H "Content-Type: application/json" \
       -d '{"passphrase":"correct-horse-battery","salt":"mysalt123","k":1,"m":1}' \
       --output reconstructed.txt
  ```

* **Option 2: GET with Secure Headers**:
  ```bash
  curl http://127.0.0.1:3001/api/v1/download/doc-alpha \
       -H "X-Mesh-Passphrase: correct-horse-battery" \
       -H "X-Mesh-Salt: mysalt123" \
       --output reconstructed.txt
  ```

* **Option 3: Using the Streaming CLI Client**:
  ```powershell
  cargo run -p mesh-cli -- download doc-alpha \
        --passphrase correct-horse-battery \
        --salt mysalt123 \
        --node-url http://127.0.0.1:3001 \
        --out reconstructed.txt
  ```

Verify content identity:
```powershell
Get-Content reconstructed.txt
```

---

## 3. Testing Across Physical LAN Devices (e.g. PC + Laptop/Phone)

When deploying across different physical devices on a local area network:

### Step A: Interface Binding & Mandatory Authentication
By default, the daemon binds to `127.0.0.1`. When exposing to the LAN, pass `--bind 0.0.0.0` or `-b <YOUR_LAN_IP>`.

```powershell
# Set an explicit API key for network access:
$env:MESH_API_KEY = "mesh_secret_lan_key_987"
cargo run -p mesh-node -- --bind 0.0.0.0 --port 4001 --api-port 3000
```

> **Security Note:** If `--bind 0.0.0.0` is used without `MESH_API_KEY`, the daemon generates an **ephemeral cryptographic key**, prints a prominent security alert banner in the console, and refuses unauthenticated non-loopback requests.

### Step B: Pairing Across LAN
On Device B (e.g. laptop at `192.168.1.15`), instruct the node to pair with Device A (`192.168.1.10`):

```bash
curl -X POST http://192.168.1.15:3000/api/v1/pair \
     -H "Content-Type: application/json" \
     -H "X-Mesh-Api-Key: mesh_secret_lan_key_987" \
     -d '{"multiaddr": "/ip4/192.168.1.10/tcp/4001/p2p/<DEVICE_A_PEER_ID>"}'
```

### Step C: Authenticated Remote Operations
All requests from remote devices must provide the key:

```bash
curl -H "X-Mesh-Api-Key: mesh_secret_lan_key_987" http://192.168.1.10:3000/api/v1/status
```

---

## 4. Multi-Tenant SaaS Control Plane Verification

The `mesh-control-plane` crate provides organization-level isolation and device lifecycle management.

### A. Bearer JWT Authentication & Anti-Spoofing
The control plane requires standard Bearer tokens:

```bash
# Correct: Authorized Bearer JWT
curl -H "Authorization: Bearer <VALID_JWT>" http://127.0.0.1:8080/api/v1/control/tenants

# Blocked: Client-provided headers (x-oidc-sub, x-oidc-tenant, etc.) without INTERNAL_GATEWAY_SECRET are rejected with 401 Unauthorized
```

### B. Persistent Repository Selection
* **PostgreSQL SaaS Mode (Production)**:
  Set `DATABASE_URL` to automatically run database migrations and use row-locked transactional invite consumption (`FOR UPDATE`):
  ```powershell
  $env:DATABASE_URL = "postgres://mesh_user:password@localhost:5432/mesh_control"
  cargo run -p mesh-control-plane
  ```
* **Single-Process Persistent Prototype**:
  Set `CONTROL_PLANE_DB_PATH` to use disk-persisted atomic JSON snapshots (suitable for solo testing):
  ```powershell
  $env:CONTROL_PLANE_DB_PATH = "./control_plane_db.json"
  cargo run -p mesh-control-plane
  ```

---

## 5. Android Native Packaging & Instrumentation Testing

The Android subsystem bridges the pure Rust core to Kotlin/Compose using `android-bridge`.

### A. Compile Native `.so` Libraries
Use `cargo-ndk` to cross-compile the JNI bridge for all target mobile architectures:

```bash
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
      -o android/app/src/main/jniLibs \
      build --release -p android-bridge
```

### B. Run Android Instrumentation Tests
Executes on an emulator or physical connected test device:

```bash
cd android
./gradlew connectedAndroidTest
```

---

## 6. Verification Summary Checklist

| Component | Target Command | Acceptance Criteria |
| :--- | :--- | :--- |
| **Full Workspace** | `cargo test --workspace` | 120 tests passed, 0 failures |
| **Clippy Linter** | `cargo clippy --workspace --all-targets -- -D warnings` | 0 warnings |
| **Code Style** | `cargo fmt --all -- --check` | 0 formatting diffs |
| **Legacy Core Node** | `npm test --prefix node_version` | 4 tests passed |
| **Dependency CVEs** | `npm audit --prefix node_version` | 0 vulnerabilities |
| **API Auth Guard** | Non-loopback request without `MESH_API_KEY` | Rejection with `401 Unauthorized` |
| **Download Security**| `GET /download/:id?passphrase=...` | Rejection with `400 Bad Request` |
| **Control Plane Auth**| Public request with spoofed `x-oidc-*` headers | Rejection with `401 Unauthorized` |
