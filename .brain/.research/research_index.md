# Research Index & System Analysis

**Last Updated:** 2026-10-09  
**Status:** Canonical Baseline Established  
**Orchestration Lead:** Autonomous System Orchestrator  

---

## 1. Vision & Core Philosophy

Mesh Storage Monitor transforms commodity devices (laptops, desktops, home servers, tablets, Android smartphones) into a **permissioned, privacy-preserving, peer-to-peer storage cloud**.

### Key Tenets
1. **Configurable Contribution (1–3%)**: Every participating node reserves a bounded, user-controlled slice of local storage (typically 1–3% of available capacity) to host encrypted shards for the mesh.
2. **End-to-End Client Encryption**: Data is encrypted using AES-256-GCM before any shard leaves the client device. Neither peer storage nodes nor SaaS control plane services ever receive plaintext files or decryption passphrases.
3. **Content-Defined Chunking**: FastCDC rolling-hash chunking splits files into deduplication-friendly, content-addressed chunks (~2MB average target).
4. **Reed-Solomon Erasure Coding**: Chunks are coded into $k$ data shards and $m$ parity shards ($n = k + m$). Retrieval of *any* $k$ valid shards reconstructs the full chunk.
5. **Merkle DAG Verification**: Shards, chunks, and manifests form a cryptographic Merkle DAG. Every shard is hash-verified before decryption.
6. **Failure-Domain-Aware Placement**: Redundant shards must never be co-located on the same physical host, IP subnet, or failure zone.
7. **Graceful Pause vs. Permanent Leave**: A temporary offline state or clean pause does not trigger premature re-encoding churn; an explicit permanent leave initiates deterministic re-placement and repair.
8. **Reciprocity Credits**: Storage allowances and bandwidth prioritization are earned through verified storage contribution and uptime, eliminating reliance on speculative token economics.

---

## 2. Research Material Index

| Resource | Path | Core Focus | Status |
|---|---|---|---|
| Project Context & Charter | `.brain/.ORG_research/MESH_STORAGE_MONITOR_PROJECT_CONTEXT.txt` | Orchestration roles, non-negotiable rules, workstreams, gates | Active Canon |
| Complete Master Roadmap | `.brain/.ORG_research/Mesh-Storage-Monitor-complete-roadmap.md` | Phase 00 through Phase 10 detailed scope, milestones A–F | Active Canon |
| Architecture Specification v2 | `.brain/.ORG_research/mesh-storage-network-spec-v2.md` | libp2p stack, Ed25519 identities, FastCDC, Merkle DAG | Active Reference |
| System Report | `.brain/.ORG_research/SYSTEM_REPORT.md` | Deterministic IV formulation, crypto pipeline, firewall rules | Active Reference |
| Operational Test Guide | `.brain/.ORG_research/RUN_TESTS.md` | Node testing procedures and multi-node execution | Active Reference |
| Mobile Operational Guide | `.brain/.ORG_research/TERMUX_GUIDE.md` | Android mobile peer operation via Termux / native service | Active Reference |
| Architecture Constraints | `.brain/.ORG_research/CLAUDE.md` | Build orders, conventions, and baseline constraints | Active Reference |

---

## 3. Technology Stack Decisions

| Layer | Canonical Choice | Reference / Deprecated | Rationale |
|---|---|---|---|
| **Node Daemon** | **Rust (`mesh-node`)** | Node.js (`node_version/`) | High performance, low memory footprint, safe concurrency, unified codebase for desktop, server, and Android (via JNI/Rust core). Node.js is retained strictly as a prototype reference. |
| **Storage Core** | **Rust (`mesh-core`)** | Node.js `core.js` | FastCDC (AVX2-accelerated), `reed-solomon-erasure` crate, `aes-gcm`, `argon2id`, `hkdf`, `sha2`. Zero network I/O in core. |
| **P2P Transport** | **libp2p (v0.53+)** | Custom TCP JSON socket | Industry standard: Noise handshake (Ed25519), Yamux multiplexing, mDNS LAN discovery, Kademlia DHT routing, Gossipsub event pubsub, Request-Response protocols. Future: AutoNAT, Circuit Relay v2, WebRTC/QUIC hole punching. |
| **Wire Protocol** | **Protobuf / Serde JSON** | Ad-hoc strings | Versioned schemas in `/protocol` crate with explicit version tags, request IDs, and status codes. |
| **Android Client** | **Native Kotlin + Compose + Rust Core** | Termux / Web browser | Native Android Foreground Service, Scoped Storage, Keystore-backed key protection, battery optimization management. |
| **Control Plane** | **PostgreSQL + Queue Workers** | Ephemeral JSON | Multi-tenant isolation, ACID consistency for device registry, signed invitations, audit logs, and repair job state machine. |
| **CLI & TUI** | **`mesh-cli` & `terminal-ui`** | Ad-hoc curl scripts | Rust clap CLI and Ratatui terminal dashboard communicating with node API. |

---

## 4. Security & Cryptographic Invariants

1. **Identity Separation**:
   - Device Identity: Ed25519 keypair generated on-device, private key never leaves storage/Keystore.
   - User Identity: Web / SaaS session token (OIDC/JWT) used for control-plane management.
   - Devices can be revoked independently without invalidating user accounts.
2. **Cryptographic Primitives**:
   - Passphrase Key Derivation: Argon2id (16MB memory, 1 iteration, 1 thread).
   - Per-File Key: HKDF-SHA256(`master_key`, `salt`, `file_id`).
   - Deterministic Shard IV: HKDF-SHA256(`file_key`, info = `"iv-{chunk_idx}-{shard_idx}"`, length = 12 bytes). Ensures identical ciphertext upon repair to prevent Merkle root mutation.
   - Symmetric Cipher: AES-256-GCM producing authenticated payload `[12-byte IV] + [16-byte Tag] + [Ciphertext]`.
3. **Invitation Security**:
   - Invitations are cryptographically signed by the organization/owner, single-use, bounded by short TTL (e.g., 15 minutes), containing rendezvous addresses and capability constraints.
   - No private keys or long-term secrets are embedded in invitations or QR payloads.
