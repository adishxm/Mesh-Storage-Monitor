# Mesh-Storage-Monitor

> **Decentralized Peer-to-Peer Mesh Storage Network & Visual Monitor**

A secure, decentralized, peer-to-peer storage network running on a local network (LAN) with no central coordinator. Every peer is equal. Files are split using Content-Defined Chunking (FastCDC), encoded with Reed-Solomon erasure coding, encrypted at-rest using AES-256-GCM, and distributed across trusted peers using `libp2p`.

---

## Architecture Overview

The system consists of the following components:

```
                  ┌──────────────────────────────────────────────┐
                  │              Web Dashboard / CLI             │
                  └──────────────────────┬───────────────────────┘
                                         │ HTTP / REST
                                         ▼
                  ┌──────────────────────────────────────────────┐
                  │                 Axum Router                  │
                  └──────────────────────┬───────────────────────┘
                                         │ Channel (Command/Event)
                                         ▼
                  ┌──────────────────────────────────────────────┐
                  │                libp2p Swarm                  │
                  │  (Noise, Yamux, Kademlia, Gossipsub, mDNS)   │
                  └──────────────────────┬───────────────────────┘
                                         │
                        ┌────────────────┴────────────────┐
                        ▼                                 ▼
           ┌─────────────────────────┐       ┌─────────────────────────┐
           │     core :: crypto      │       │     core :: erasure     │
           │  Argon2id + AES-256-GCM │       │      Reed-Solomon       │
           └─────────────────────────┘       └─────────────────────────┘
```

1. **`/core`**: Core library implementing:
   * **FastCDC** (Content-Defined Chunking) to chunk files with a ~2MB target size.
   * **Reed-Solomon** erasure coding ($k$ data + $m$ parity shards).
   * **AES-256-GCM** data-at-rest encryption using Argon2id master keys and deterministic IV derivation via HKDF-SHA256 (prevents hash/CID mutation on repair).
   * **Merkle DAG** verification for tamper-proof chunk and shard integrity checks.
2. **`/node`**: Core daemon wrapping the `/core` logic with networking:
   * **Axum REST API** for status, list peers, upload, download, and pairing.
   * **libp2p Network Behaviour** with mDNS (auto-discovery), Kademlia DHT (routing), Gossipsub (event notifications), Ping, Identify, and a custom JSON Request-Response protocol for transfers and audits.
   * **Access Control & Anti-Flood**: Checks connections against a `trusted_peers.json` allowlist. Exceeding 5 connection attempts in 60s from an untrusted IP triggers a 30-minute in-memory ban and spawns a Windows Firewall rule (`netsh advfirewall`) to block the brute-forcer.

---

## Directory Structure

```
├── core/                       # Pure logic Rust library
│   ├── src/
│   │   ├── chunking.rs         # FastCDC content-defined chunking implementation
│   │   ├── crypto.rs           # Encryption (AES-GCM-256, Argon2id, HKDF)
│   │   ├── erasure.rs          # Reed-Solomon shard encoding/decoding wrapper
│   │   ├── merkle.rs           # Merkle tree building & verification
│   │   └── lib.rs              # Library exports and milestone tests
│   └── Cargo.toml
│
├── node/                       # Rust executable: P2P Node & Axum API
│   ├── src/
│   │   ├── main.rs             # Application entrypoint & CLI argument parsing
│   │   ├── state.rs            # Node state, trusted peers, storage tracking
│   │   ├── network.rs          # Swarm management, protocols, commands, uploads/downloads
│   │   └── api.rs              # Axum HTTP server endpoints
│   └── Cargo.toml
│
├── mesh-cli/                   # Command Line tool wrapping Node status API (stub)
├── terminal-ui/                # Ratatui dashboard stub
├── Cargo.toml                  # Cargo workspace definition
└── Cargo.lock
```

---

## Technical Specifications

| Component | Technology / Decision |
|---|---|
| **Core Language** | Rust (v1.96.0+) |
| **Networking** | `libp2p` (TCP, Noise transport, Yamux multiplexer) |
| **Discovery** | mDNS (local LAN auto-discovery), Kademlia DHT (peer routing) |
| **Access Control** | PeerID-based `trusted_peers.json` allowlist. Untrusted peers disconnected immediately. |
| **Brute Force Defense** | 5 failed attempts in 60s -> 30-minute in-memory ban & OS Firewall block (`netsh advfirewall`) |
| **Chunking** | FastCDC (1MB Min, 2MB Avg, 4MB Max chunk size) |
| **Erasure Coding** | Reed-Solomon Erasure (`k` data, `m` parity shards, where $k+m \le$ network peer count) |
| **Data Encryption** | AES-256-GCM + Argon2id (key derivation) + deterministic HKDF-SHA256 per-shard IVs |
| **Integrity Checks** | Root-hash verification using SHA-256 Merkle DAG |

---

## REST API Interface

Every running Node runs an Axum HTTP API (default: `http://localhost:3000`) for management and local client integration.

### `GET /status`
Returns status of the local node.
* **Response**:
```json
{
  "peer_id": "12D3KooW...",
  "listen_addresses": ["/ip4/192.168.1.10/tcp/4001"],
  "peers": ["12D3KooW..."],
  "storage_used": 1048576,
  "storage_quota": 1610612736,
  "shards": ["3a5c1e...", "bd49f3..."],
  "trusted_peers": ["12D3KooW...", "12D3KooW..."]
}
```

### `GET /peers`
Returns a list of connected and trusted peer IDs.
* **Response**: `["12D3KooW...", "12D3KooW..."]`

### `GET /shards`
Returns a list of shard hashes stored locally on this node.
* **Response**: `["3a5c1e...", "bd49f3..."]`

### `POST /pair`
Accepts a multiaddr to pair with another peer.
* **Request Body**:
```json
{
  "multiaddr": "/ip4/192.168.1.15/tcp/4001/p2p/12D3KooW..."
}
```
* **Response Status**: `200 OK` (successfully paired) or `400 Bad Request` / `500 Internal Server Error`.

### `POST /upload`
Uploads and distributes a file across the mesh.
* **Content-Type**: `multipart/form-data`
* **Form Fields**:
  * `file_id`: Unique identifier/string name for the file.
  * `passphrase`: User passphrase for Argon2id key derivation.
  * `salt`: Cryptographic salt string.
  * `k`: Data shard count (e.g. `2`).
  * `m`: Parity shard count (e.g. `1`).
  * `file`: Binary file payload.
* **Response**: JSON file manifest detailing the Merkle tree.

### `GET /download/:file_id`
Downloads and reconstructs a file from the mesh.
* **Path Parameters**: `file_id` (the name used during upload)
* **Query Parameters**:
  * `passphrase`: Passphrase used for encryption.
  * `salt`: Salt used for encryption.
  * `k`: Data shard count.
  * `m`: Parity shard count.
* **Response**: Plaintext file payload.
