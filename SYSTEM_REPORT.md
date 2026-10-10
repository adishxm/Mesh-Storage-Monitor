# Mesh Storage Network: Technical System Report

This report summarizes the design, cryptographic protocol, network architecture, and security properties of the decentralized Mesh Storage Network implemented in this workspace.

---

## 1. Data Pipeline & Core Storage Mechanics

The core data pipeline follows a strict mathematical transform:
```
[Plaintext File] 
       │ 
       ▼ (FastCDC Content-Defined Chunking)
   [Chunks] (~2MB average size)
       │ 
       ▼ (Reed-Solomon Erasure Coding)
 [Data & Parity Shards] (k data, m parity)
       │ 
       ▼ (AES-256-GCM Encryption + Deterministic IV)
 [Encrypted Shards] ───► Distributed to Peer Nodes via libp2p
```

### A. Content-Defined Chunking (FastCDC)
Traditional chunking splits files at fixed byte offsets, which suffers from the "insertion/deletion shift" problem (inserting 1 byte shifts all downstream offsets, changing every chunk hash).
* **Our Solution**: We implemented **FastCDC** using a rolling Gear hash. A sliding window hashes the content and triggers a split boundary when the hash matches a specific pattern. This ensures boundaries are locked to content patterns.
* **Size Constraints**: Target average chunk size is 2MB, with a canonical minimum of 512KB and a maximum of 8MB (defined as protocol constants in `core/src/chunking.rs`).

### B. Cryptographic System
1. **Master Key Derivation**: Uses **Argon2id** (the industry-standard memory-hard hashing algorithm) to derive a master key from a user passphrase and salt.
2. **Per-File Key Derivation**: Uses **HKDF-SHA256** with the master key and the unique `file_id` as context, ensuring that compromised file keys do not leak the master key.
3. **Symmetric Encryption**: Uses **AES-256-GCM** to provide both confidentiality and authenticated integrity (AEAD) for each shard.

### C. Deterministic IV Derivation (Critical Fix)
* **The Problem**: A standard random Initialization Vector (IV) for AES-GCM means that encrypting the same shard twice yields different ciphertexts. During self-healing/repair, if a node reconstructs a lost shard from parity and encrypts it using a random IV, the resulting shard ciphertext hash will change. This breaks the static Merkle root hash (CID) stored in the manifest.
* **Our Solution**: We derive a deterministic 12-byte IV for each shard using HKDF-SHA256:
  $$\text{IV} = \text{HKDF-Expand}(\text{File Key}, \text{info} = \text{"shard\_iv"} \parallel \text{chunk\_idx} \parallel \text{shard\_idx}, 12)$$
  This guarantees that shard ciphertexts and hashes are completely deterministic per file, preventing Merkle root mutation during node healing or re-placement.

### D. Reed-Solomon Erasure Coding
* We wrap the `reed-solomon-erasure` crate using a Cauchy matrix mapping.
* Chunks are split into $k$ data shards and $m$ parity shards.
* As long as any $k$ of the total $k+m$ shards are retrieved, the original chunk can be decoded, allowing the system to tolerate the loss of up to $m$ peer nodes.

### E. Merkle DAG and CID Verification
* A Merkle tree is computed over the SHA-256 hashes of all encrypted shards and chunk hashes.
* The root of this tree represents the unique **Content Identifier (CID)**.
* Before downloading or decoding, retrieval nodes verify individual shard hashes against the Merkle tree to detect tampering instantly.

---

## 2. Peer-to-Peer Network Architecture

The networking layer is implemented using `libp2p` (v0.53) in `/node` with the following behaviour stack:

```
┌─────────────────────────────────────────────────────────────┐
│                        libp2p Swarm                         │
├─────────────────────────────────────────────────────────────┤
│  [mDNS]          Auto-discovers trusted local LAN nodes     │
│  [Kademlia]      DHT for routing tables & peer address storage │
│  [Gossipsub]     Broadcasts events (upload, download, audit)│
│  [Ping/Identify] Liveness verification & protocol exchange  │
│  [Req-Resp]      Transfers shards (Store, Retrieve, Audit)  │
└─────────────────────────────────────────────────────────────┘
```

1. **Noise Transport**: All network communication is encrypted and authenticated at the transport layer using Noise (Ed25519 PeerIDs).
2. **mDNS (Multicast DNS)**: Discovers nodes automatically on the local Wi-Fi. Nodes only add discovered peers to Kademlia routing tables if they are present in the `trusted_peers.json` allowlist.
3. **Kademlia DHT**: Performs peer routing and routing table maintenance.
4. **Request-Response Protocol**: A custom JSON-serialized request-response protocol handles:
   * `Pair`: Exchanges multiaddresses and PeerIDs.
   * `Store`: Distributes shards to peer storage.
   * `Retrieve`: Fetches shards from holder nodes.
   * `AuditChallenge`: Triggers challenges to verify remote shard storage.
5. **Gossipsub**: Publishes network-wide real-time events (e.g., `upload_complete`, `download_complete`) to topic `"mesh-events"`.

---

## 3. Access Control & Security Safeguards

A major threat to local network P2P networks is unauthorized peer joining and connection flooding (DoS).

### A. Mutual Authenticated Allowlist
* The node loads a local `trusted_peers.json` allowlist from its data directory.
* When a connection is established (`ConnectionEstablished` event in the Swarm), the remote `PeerId` is checked against the allowlist.
* If the peer is not trusted, the connection is instantly terminated before any payload communication takes place.

### B. Two-Way Pairing Handshake
* To add trust, a node uses the `/pair` HTTP API.
* This dials the remote node, initiates the secure handshake, sends a `ShardRequest::Pair` containing its multiaddress, and adds the target to its own allowlist.
* The receiver accepts the request, inserts the dialer's PeerId into its own allowlist, and replies with a `PairAck`. Both nodes persist these updates to disk.

### C. Brute-Force and Flood Defense (Windows Firewall Integration)
* If an untrusted peer repeatedly attempts to connect, the system tracks its IP address.
* If **5 untrusted connection attempts are made within 60 seconds**:
  1. The node institutes a 30-minute in-memory ban on the IP.
  2. The node attempts to create an OS-level packet filter rule by executing:
     `netsh advfirewall firewall add rule name="MeshStorage Block <IP>" dir=in action=block remoteip=<IP>`
  3. If running without Administrator rights, the block gracefully falls back to the application-level in-memory block list.
