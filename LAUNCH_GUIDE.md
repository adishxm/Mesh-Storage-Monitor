# Live Network Launch & Multi-Device Verification Guide

> **Step-by-step operational manual for launching the decentralized Mesh Storage Network live across physical devices, connecting peers, uploading a real image, verifying cryptographic sharding, and testing fault-tolerant reconstruction.**

---

## Architecture Topology (3-Node Example)

```
        ┌────────────────────────────────────────────────────────┐
        │                 Local Wi-Fi / LAN Network              │
        └───────────────────────────┬────────────────────────────┘
                                    │
       ┌────────────────────────────┼────────────────────────────┐
       ▼                            ▼                            ▼
┌──────────────┐             ┌──────────────┐             ┌──────────────┐
│   Device A   │             │   Device B   │             │   Device C   │
│ (Desktop PC) │             │   (Laptop)   │             │ (Phone/Mac)  │
│ 192.168.1.10 │             │ 192.168.1.15 │             │ 192.168.1.20 │
│  Node 1      │◄──Noise────►│  Node 2      │◄──Noise────►│  Node 3      │
│  Port 4001   │   Swarm     │  Port 4001   │   Swarm     │  Port 4001   │
│  UI :3000    │             │  UI :3000    │             │  UI :3000    │
└──────┬───────┘             └──────┬───────┘             └──────┬───────┘
       │                            │                            │
       ▼                            ▼                            ▼
 [Shard 0 (Data)]             [Shard 1 (Data)]            [Shard 2 (Parity)]
```

---

## Prerequisites & Checklist

1. **At least 2 physical devices** (e.g., Windows PC + Laptop/Mac/Linux) connected to the **same local Wi-Fi or router network**.
2. **Rust toolchain** installed on devices that will compile from source (`cargo` and `rustc 1.80+`).
3. **Firewall / Network Ports**:
   - **TCP Port `4001`**: libp2p P2P transport (Noise + Yamux swarm).
   - **TCP Port `3000`**: Axum HTTP REST API & Web Dashboard.
   *(Make sure your OS firewall allows incoming traffic on these ports).*
4. **Identify Each Device's Local LAN IP**:
   - **Windows**: Run `ipconfig` (look for `IPv4 Address`, e.g., `192.168.1.10`).
   - **macOS / Linux**: Run `ip a` or `ifconfig` (look for `inet`, e.g., `192.168.1.15`).

---

## Step 1: Launch Node 1 on Device A (Anchor Node)

On your primary machine (Device A, e.g. IP `192.168.1.10`):

### 1.1 Open Terminal & Set Authentication Key
Because we are binding to network interfaces (`0.0.0.0`), set an API key for network security:

* **PowerShell (Windows)**:
  ```powershell
  $env:MESH_API_KEY = "mesh_secret_lan_key_987"
  ```
* **Bash (macOS / Linux)**:
  ```bash
  export MESH_API_KEY="mesh_secret_lan_key_987"
  ```

### 1.2 Start the Node Daemon
```powershell
cargo run -p mesh-node -- --bind 0.0.0.0 --port 4001 --api-port 3000 --quota 5.0
```

### 1.3 Note the Console Output
Check the terminal logs:
```text
[INFO] Local Peer ID: 12D3KooWJ8eR49zXy1...
[INFO] Swarm listening on address: /ip4/127.0.0.1/tcp/4001
[INFO] Swarm listening on address: /ip4/192.168.1.10/tcp/4001
[INFO] Starting HTTP API server on 0.0.0.0:3000
```

Copy your **Device A Multiaddress**:
```text
/ip4/192.168.1.10/tcp/4001/p2p/12D3KooWJ8eR49zXy1...
```

### 1.4 Open the Visual Monitoring Dashboard
Open your web browser and navigate to:
```text
http://localhost:3000/
```
(Or from any browser on the LAN: `http://192.168.1.10:3000/`)

You should see the **Mesh Storage Monitor** interface:
- **Node State**: ONLINE (Green dot).
- **Peer ID**: Shows short ID.
- **Reciprocity Tier**: Tier 1 Unrestricted.
- **Connected Peers**: `0 active`.

---

## Step 2: Launch Node 2 on Device B (Second Device)

On your second machine (Device B, e.g. laptop at `192.168.1.15`):

### 2.1 Set Authentication Key & Start Daemon
* **Terminal on Device B**:
  ```bash
  export MESH_API_KEY="mesh_secret_lan_key_987"
  cargo run -p mesh-node -- --bind 0.0.0.0 --port 4001 --api-port 3000 --quota 5.0
  ```

*(If testing both nodes on the **same single computer**, use different ports: `--port 4002 --api-port 3001`).*

### 2.2 Note Device B's Multiaddress
Device B prints its own identity in console:
```text
[INFO] Local Peer ID: 12D3KooWLq8A7B2cN4...
[INFO] Swarm listening on address: /ip4/192.168.1.15/tcp/4001
```

Device B Multiaddress:
```text
/ip4/192.168.1.15/tcp/4001/p2p/12D3KooWLq8A7B2cN4...
```

---

## Step 3: Connect & Pair the Nodes

> **Why Pairing is Required:** Mesh Storage uses Noise mutual handshake allowlisting. Connections from unrecognized nodes are instantly dropped to protect against unauthorized data tampering and Sybil swarm flooding.

### Option A: Using the Web Dashboard (Easiest)
1. Open Device A's dashboard in your browser (`http://localhost:3000`).
2. Scroll to the **Connected & Trusted Peers** card on the left.
3. In the input box labeled **"Pair New Peer Multiaddress"**, paste Device B's multiaddress:
   ```text
   /ip4/192.168.1.15/tcp/4001/p2p/12D3KooWLq8A7B2cN4...
   ```
4. Click the purple **"Pair"** button.
5. Watch the top-right notification: **"Successfully paired with peer!"**
6. Within 1-2 seconds:
   - Device A's dashboard displays: `1 active` peer.
   - Device B's dashboard automatically updates to show Device A connected.

### Option B: Using curl / PowerShell
Run this from any terminal on the LAN:
```bash
curl -X POST http://192.168.1.10:3000/api/v1/pair \
     -H "Content-Type: application/json" \
     -H "X-Mesh-Api-Key: mesh_secret_lan_key_987" \
     -d '{"multiaddr": "/ip4/192.168.1.15/tcp/4001/p2p/12D3KooWLq8A7B2cN4..."}'
```

### Option C: Using Cryptographic Signed Invitations
1. On Device A's dashboard, locate the **Invitations & Cluster Enrollment** card.
2. Click **"Generate Single-Use Invite Token"**.
3. Copy the base64 invitation token generated.
4. On Device B's dashboard, paste the token into **"Join via Token"** and click **"Join Cluster"**.

---

## Step 4: Live Test — Uploading a Real Image

Let's upload an actual photo (e.g. `sample.jpg` or `diagram.png`).

### 4.1 Via the Web Dashboard (Device A)
1. In Device A's dashboard, find the **Upload & Sharding Studio** card (right column).
2. Fill in the parameters:
   - **File ID (Name / Key)**: `sample-image.jpg`
   - **Passphrase**: `my-secure-mesh-password`
   - **Salt Key**: `unique-salt-99`
   - **Data (k)**: `1` *(for a 2-node cluster, use $k=1, m=1$; for 3+ nodes, use $k=2, m=1$)*
   - **Parity (m)**: `1`
3. Click **"Select Local File"** and choose your image file (`.jpg` or `.png`).
4. Click the gradient button: **"Encrypt, Shard & Distribute"**.

### 4.2 What Happens Under the Hood
1. **FastCDC Chunking**: The image bytes are split into content-defined chunks with a rolling Gear hash.
2. **Reed-Solomon Galois Coding**: For each chunk, the engine computes $k$ data shards and $m$ parity shards.
3. **Argon2id + AES-256-GCM**: Shards are encrypted with deterministic IV derivation (`derive_shard_iv`), ensuring the Merkle tree root hash (CID) never mutates.
4. **P2P Transport**: Device A stores Shard 0 locally, and streams Shard 1 across the libp2p Noise channel to Device B.
5. **Success Toast**: The dashboard displays the Merkle Root CID and chunk manifest.

### 4.3 Alternative: Upload Via Terminal (mesh-cli)
```powershell
cargo run -p mesh-cli -- upload sample.jpg \
      --file-id "sample-image.jpg" \
      --passphrase "my-secure-mesh-password" \
      --salt "unique-salt-99" \
      --k 1 --m 1 \
      --node-url "http://127.0.0.1:3000"
```

---

## Step 5: Verify Shard Distribution & Storage on Disks

Now verify that the sharding is genuine and decentralized across both machines:

### 5.1 Inspect Disks Directly
* **On Device A**:
  Open file explorer at `./data_4001/shards/` (or your custom data dir).
  You will see encrypted shard files with `.mshr` extensions:
  ```text
  ./data_4001/shards/a8f1b94d...mshr
  ```
* **On Device B**:
  Open file explorer at `./data_4001/shards/` (on Device B).
  You will see the corresponding parity shard:
  ```text
  ./data_4001/shards/c49e2107...mshr
  ```

Neither device alone has the full image plaintext—each holds only an encrypted mathematical shard!

### 5.2 Check Dashboard Telemetry
On both dashboards:
- Look at **Locally Stored Shards**: The shard hashes appear in the list.
- Look at **Credits Contributed / Consumed**: The reciprocity credit meters reflect the uploaded bytes.
- Look at **Storage Quota**: The progress meter reflects used bytes against the 5GB ceiling.

---

## Step 6: Download & Reconstruct the Image from Device B

Now verify decentralized retrieval: retrieve the image from **Device B**, which must fetch the missing shard over the network from Device A, decode the Galois field matrix, decrypt using AES-GCM, and reconstruct the file.

### 6.1 Via Web Dashboard on Device B
1. Open Device B's browser at `http://localhost:3000/`.
2. In the **Download & Reconstruction Studio** card:
   - **File ID to Retrieve**: `sample-image.jpg`
   - **Passphrase**: `my-secure-mesh-password`
   - **Salt Key**: `unique-salt-99`
   - **Data (k)**: `1`
   - **Parity (m)**: `1`
3. Click **"Download & Reconstruct"**.
4. The browser triggers a file download: `sample-image.jpg`.
5. Open the downloaded image file.

> **Verification Check:** The image opens in your photo viewer with **pixel-for-pixel fidelity**, zero compression artifacts, and zero corruption!

### 6.2 Via CLI on Device B
```powershell
cargo run -p mesh-cli -- download "sample-image.jpg" \
      --passphrase "my-secure-mesh-password" \
      --salt "unique-salt-99" \
      --k 1 --m 1 \
      --node-url "http://127.0.0.1:3000" \
      --out "restored_sample.jpg"
```

---

## Step 7: Chaos & Fault-Tolerance Test (Offline Node Survival)

To test the system's resilience to peer loss:

### 7.1 Setup a 3-Node Cluster with $k=2, m=1$
Upload an image with $k=2$ data shards and $m=1$ parity shard (total = 3 shards distributed across Device A, Device B, and Device C).

### 7.2 Simulate Node Failure
1. On Device B, press `Ctrl + C` in the terminal to kill the daemon.
2. In Device A's dashboard, Device B drops offline.

### 7.3 Reconstruct from Remaining Surviving Nodes
1. On Device A or Device C, trigger the download for the image.
2. Even though Device B is dead and its shard is completely unreachable:
   - The engine retrieves the 2 remaining shards from Device A and Device C.
   - Because $2 \ge k$, the Reed-Solomon Cauchy decoder reconstructs the missing data chunk.
   - Decryption succeeds and the image is completely restored!

---

## Step 8: Live Monitoring Tools

While the network is running, monitor health using these integrated tools:

### 8.1 Real-Time ANSI Terminal UI
Run in any terminal to see live gauges:
```powershell
cargo run -p terminal-ui -- --node-url http://127.0.0.1:3000
```
Displays:
* Live ASCII Storage Quota meter
* Reciprocity Tier meter and Fair-Share ratio
* Connected peer addresses and audit challenge counters

### 8.2 Prometheus Metrics Exposition
Fetch raw Prometheus text metrics:
```bash
curl http://127.0.0.1:3000/metrics
```
Metrics exposed include:
- `mesh_storage_used_bytes`
- `mesh_peers_connected`
- `mesh_reciprocity_contributed_bytes`
- `mesh_audit_challenges_passed_total`

---

## Common Gotchas & Troubleshooting

| Symptom | Cause | Solution |
| :--- | :--- | :--- |
| **Connection refused on `:3000`** | Daemon is bound to loopback only | Ensure `--bind 0.0.0.0` was passed when starting `mesh-node`. |
| **`401 Unauthorized` on API call** | Missing or incorrect API key | Pass `-H "X-Mesh-Api-Key: <KEY>"` or `Authorization: Bearer <KEY>`. |
| **`400 Bad Request` on download** | Passphrase in query string | Pass credentials via POST JSON body or `X-Mesh-Passphrase` header. |
| **Peers fail to connect / Noise drop** | Untrusted Peer ID | Perform the `/api/v1/pair` handshake first to add to `trusted_peers.json`. |
| **Windows blocks incoming traffic** | Windows Firewall | Run terminal as Admin or allow TCP ports `4001` and `3000` in Windows Defender Firewall. |
