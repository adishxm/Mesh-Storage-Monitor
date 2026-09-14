# Local Mesh Storage Network — Build Specification

## 1. Goal

A self-hosted, distributed storage network running on your home Wi-Fi.
Your PC is the **coordinator + first storage node**. Any other device on
the same Wi-Fi (laptop, phone via browser, another PC) can join as an
**additional storage node**. Files you upload get chunked, erasure-coded
with Reed-Solomon, encrypted, and spread across nodes. You watch
everything live through a terminal feed and a React dashboard.

---

## 2. Your Network Topology

| Role | Device | Specs | Notes |
|---|---|---|---|
| Coordinator + Node 0 | Your PC | i5-210H, 16GB DDR5, RTX 4050 6GB, 512GB NVMe | Runs the API server, scheduler, WebSocket hub, and a local storage agent |
| Node 1..N | Any phone/laptop/PC on same Wi-Fi | varies | Runs only the lightweight storage agent (or a web-based agent in a browser) |

All traffic stays on the LAN (`192.168.x.x`). No internet exposure needed
for the MVP. Devices find the coordinator via:
- **mDNS/Bonjour** (auto-discovery, `bonjour-service` npm package), or
- **manual pairing**: coordinator shows a QR code / pairing code in its
  terminal, the joining device scans/enters it to register.

---

## 3. Components & Tech Stack

```
/coordinator        Node.js (Express + ws + better-sqlite3)
/node-agent         Node.js, runs on every device (including your PC)
/web-dashboard       React + Vite + socket.io-client + recharts
/terminal-dashboard  Node.js + ink (React-for-CLI) — your "live terminal"
/shared              crypto utils, Reed-Solomon wrapper, types
```

**Key libraries:**
- Reed-Solomon: `reed-solomon-erasure.wasm` (fast, WASM-based)
- Hashing: built-in `crypto` (SHA-256) for content addressing
- Encryption: `crypto` AES-256-GCM, `argon2` for key derivation, HKDF for
  per-file keys
- Node identity / signing: `tweetnacl` (Ed25519)
- Realtime: `ws` on backend, `socket.io-client` or native WebSocket on
  frontend
- Metadata DB: `better-sqlite3` (single file, zero setup)
- Terminal UI: `ink` — lets you write the live terminal view in JSX,
  reusing concepts from your React dashboard

> Your RTX 4050 / GPU isn't needed for the core storage engine. Keep it
> in reserve for a later optional feature (e.g., local embeddings for
> semantic file search via a local model).

---

## 4. Data Flow — Upload (worked example: your 20MB file)

1. **Chunking**: split file into 2MB chunks → 10 chunks.
2. **Erasure coding per chunk**: Reed-Solomon with `k=2` data shards +
   `m=1` parity shard → 3 shards of ~1MB each per chunk (30 shards total).
   - Any 2 of the 3 shards reconstruct the chunk → tolerates **1 node
     being offline/deleted** per chunk.
3. **Hashing**: each shard is SHA-256 hashed → this hash becomes its
   content address.
4. **Encryption**: each shard is encrypted client-side with AES-256-GCM
   using a key derived from your master passphrase (Argon2id → HKDF).
   Nodes only ever store opaque encrypted blobs.
5. **Placement**: coordinator picks 3 nodes (one per shard) based on
   available quota, prioritizing nodes with the most free contributed
   space.
6. **Manifest**: coordinator records, in SQLite:
   `file_id → [chunk_1..10] → [shard_hash, node_id, size]`

### Download
1. Client fetches manifest from coordinator.
2. Requests shards in parallel from the listed nodes.
3. If a node is offline and a shard is missing, Reed-Solomon
   reconstructs it from the remaining `k=2` shards.
4. Decrypt shards → reassemble chunks → reassemble file.

---

## 5. Storage Contribution (1–2% rule)

Each node agent, on first run, scans free disk space and lets the user
pick a contribution % (default 1.5%):

| Node | Total | At 1.5% |
|---|---|---|
| Laptop (1TB) | 1,000 GB | ~15 GB |
| Mobile (256GB) | 256 GB | ~3.8 GB |
| Computer (4TB) | 4,000 GB | ~60 GB |

Total network capacity ≈ **79 GB** — plenty for a prototype. The agent
enforces this quota locally (rejects writes past the cap) and reports
`used / quota` to the coordinator every heartbeat.

---

## 6. Security System

### 6.1 Node identity
- On first run, each node agent generates an **Ed25519 keypair**.
- The public key is registered with the coordinator during pairing.
- Every node↔coordinator and node↔node request is **signed**; the
  coordinator verifies the signature before processing.

### 6.2 User authentication
- Login password hashed with **Argon2id**.
- Successful login issues a short-lived **access token** (signed JWT,
  ~15 min) + refresh token.
- Rate limiting: 5 failed attempts / 15 minutes per IP, then that IP is
  temporarily blacklisted at the coordinator's firewall layer.

### 6.3 "Hash-gated" storage API (your brute-force requirement)
Each node agent's local storage API requires every request to carry an
**HMAC-SHA256 signature** (computed over method + path + timestamp +
nonce, using a per-node shared secret issued at pairing).

- Valid signature → request proceeds.
- Invalid signature → request is dropped immediately (no error detail
  leaked), and a per-source-IP failure counter increments.
- After **5 invalid signatures in 60 seconds**, that source IP is added
  to a local block rule (via the OS firewall — `netsh advfirewall` on
  Windows, `iptables`/`ufw` on Linux) for 30 minutes — effectively
  making the node "crash/unreachable" to that attacker, exactly as you
  described.

### 6.4 Data-at-rest security
- All shards are encrypted **before** they leave the uploading device.
- Storage nodes never hold an encryption key — they store ciphertext +
  hash only.
- Per-file keys derived via HKDF from your master key, so compromising
  one node never exposes plaintext, and even compromising the
  coordinator's metadata DB doesn't expose file contents.

### 6.5 Transport security
- Even on LAN, all HTTP/WebSocket traffic runs over **HTTPS/WSS** using
  a locally-trusted certificate generated once via `mkcert` — avoids
  plaintext traffic sniffing on shared Wi-Fi.

---

## 7. Node Lifecycle

### Joining
1. Device opens `http://<coordinator-ip>:port/join` (or scans pairing
   QR shown in the terminal).
2. Generates Ed25519 keypair, registers public key + declared storage
   quota.
3. Coordinator adds it to the node registry, starts sending heartbeats
   every 30s.

### Going offline / repair
- If a node misses heartbeats for > 5 minutes, coordinator marks it
  `degraded`.
- Coordinator scans manifests for chunks with shards on that node and,
  for each, reconstructs the missing shard from the remaining `k`
  shards and re-places it on a healthy node — keeping every chunk at
  full redundancy.

### Account deletion → 30-day retention
1. User requests account deletion → account + all their file manifests
   flagged `pending_deletion` with a timestamp.
2. Data stays fully intact and recoverable for **30 days** (a "restore
   account" action simply clears the flag).
3. A daily scheduled job (`node-cron`) checks for accounts past 30 days
   and:
   - Sends delete commands (signed, per §6.1) to every node holding any
     shard for that account.
   - Removes the manifest rows once all nodes confirm deletion.
   - Logs the full purge event to the terminal feed.

---

## 8. Live Monitoring — React App + Terminal

Both surfaces subscribe to the **same WebSocket event stream** from the
coordinator. Event types:

```
node_joined, node_left, node_degraded, node_recovered,
upload_started, upload_progress, upload_complete,
download_started, download_progress, download_complete,
shard_repaired, account_deletion_scheduled, account_purged
```

- **React dashboard**: node list with live storage bars (recharts),
  file browser with upload/download, network health map, activity feed.
- **Terminal (ink)**: a scrolling, color-coded log of the same events —
  this is what shows up in your terminal as "network updates," and
  since `ink` is React-based, you can share UI logic/components between
  the two.

---

## 9. Build Roadmap (recommended order)

1. **Phase 1** — Coordinator + single node agent (your PC only).
   Plain file storage, no RS, no encryption yet. Confirm upload/download
   works end-to-end.
2. **Phase 2** — Add Reed-Solomon chunking/encoding (k=2, m=1) and
   SHA-256 content addressing.
3. **Phase 3** — Add AES-256-GCM encryption + key derivation (Argon2id +
   HKDF).
4. **Phase 4** — Bring a second/third device onto the Wi-Fi as nodes via
   pairing; verify placement and reconstruction when one node is offline.
5. **Phase 5** — React dashboard with live WebSocket updates.
6. **Phase 6** — Terminal (`ink`) live feed.
7. **Phase 7** — Full security layer: Ed25519 node identity, HMAC
   request signing, firewall auto-block on brute attempts, HTTPS/WSS via
   mkcert.
8. **Phase 8** — 30-day deletion lifecycle + repair/self-healing
   scheduler.

---

## 10. Prompt to give your coding agent

> Build a local-network distributed storage prototype called
> "mesh-storage". Structure: `/coordinator` (Node.js + Express + ws +
> better-sqlite3), `/node-agent` (Node.js, runs on each device),
> `/web-dashboard` (React + Vite + socket.io-client + recharts),
> `/terminal-dashboard` (Node.js + ink). The coordinator runs on my PC
> and acts as Node 0. Other devices on the same Wi-Fi join as Node 1..N
> via a pairing code shown in the terminal.
>
> Implement in this order: (1) basic file upload/download stored as
> plain files on Node 0 with a SQLite manifest; (2) chunk files into
> 2MB pieces and encode each chunk with Reed-Solomon k=2/m=1 using
> `reed-solomon-erasure.wasm`, SHA-256-address each shard; (3) AES-256-GCM
> encrypt every shard client-side using a key derived via Argon2id +
> HKDF from a user passphrase; (4) node-agent pairing flow generating
> Ed25519 keypairs, registering quota (1.5% of free disk by default);
> (5) HMAC-signed requests between coordinator and node agents, with
> auto-block of any IP after 5 bad signatures in 60 seconds via the OS
> firewall; (6) heartbeat-based health checks and an automatic repair job
> that reconstructs and re-places shards from degraded nodes; (7) a
> WebSocket event stream (node_joined/left, upload/download progress,
> shard_repaired, account_purged) consumed by both the React dashboard
> and the ink terminal UI; (8) account deletion flow that flags data
> `pending_deletion`, retains it 30 days, then purges it across all nodes
> via a daily cron job.
>
> Use HTTPS/WSS with a local mkcert certificate for all traffic, even on
> LAN. Build and test each phase before moving to the next.

---

## 11. Open Design Decisions for Next Session

- Exact RS parameters once you know how many nodes you'll realistically
  run (k/m ratio scales with node count).
- Whether the React dashboard runs only on your PC or is accessible from
  other devices too (affects CORS/cert setup).
- Whether phone "nodes" run a native agent or just a browser tab (browser
  tabs can't hold persistent storage reliably — may need a small installed
  app or PWA with the File System Access API).
