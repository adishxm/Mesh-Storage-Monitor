# Mesh Storage Monitor — Operations & Node Runbook

**Version:** 2.0  
**Target Environment:** Local Wi-Fi, Multi-Device LAN, Edge Cloud  

---

## 1. Quickstart — Running a Rust Node

### Building the Workspace
```bash
cargo build --workspace
cargo test --workspace
```

### Running Node Instances Locally (Multi-Process Testing)

#### Node 1 (Anchor / Primary Peer)
```bash
cargo run -p mesh-node -- --port 4001 --api-port 3000 --quota 5.0
```
- P2P Swarm listens on `0.0.0.0:4001`
- Management REST API on `127.0.0.1:3000`
- Data stored in `./data_4001`

#### Node 2 (Peer dialed to Node 1)
```bash
cargo run -p mesh-node -- --port 4002 --api-port 3001 --quota 5.0 --dial /ip4/127.0.0.1/tcp/4001/p2p/<PEER_ID_NODE_1>
```

#### Node 3 (Third Peer)
```bash
cargo run -p mesh-node -- --port 4003 --api-port 3002 --quota 5.0 --dial /ip4/127.0.0.1/tcp/4001/p2p/<PEER_ID_NODE_1>
```

---

## 2. Managing Node Lifecycle

### Pausing a Node (Temporary Offline)
A pause signals to peers that the node is temporarily unavailable (e.g. reboot, laptop lid closed):
```bash
curl -X POST http://localhost:3000/api/v1/pause
```
- Shards on disk are preserved.
- The network grants a grace period (default 15 minutes) before marking shards degraded.

### Leaving the Network (Permanent Departure)
An explicit leave safely moves held shards to other healthy peers before the node disconnects:
```bash
curl -X POST http://localhost:3000/api/v1/leave
```
- Trigger repair jobs for all held shards.
- Waits for redundancy confirmation.
- Revokes active storage assignment.

---

## 3. Uploading & Downloading Files

### Upload File
```bash
curl -X POST http://localhost:3000/api/v1/upload \
  -F "file=@test.txt" \
  -F "file_id=my-document-1" \
  -F "passphrase=securepassphrase123" \
  -F "salt=mysalt987654321" \
  -F "k=2" \
  -F "m=1"
```

### Download File
```bash
curl -G http://localhost:3000/api/v1/download/my-document-1 \
  --data-urlencode "passphrase=securepassphrase123" \
  --data-urlencode "salt=mysalt987654321" \
  --data-urlencode "k=2" \
  --data-urlencode "m=1" \
  -o restored_test.txt
```

---

## 4. Legacy Prototype Notice

The legacy Node.js scripts in `node_version/` (`app.js`, `core.js`, `network.js`) are frozen references. Do not launch them for production operation. Always use the canonical Rust implementation.
