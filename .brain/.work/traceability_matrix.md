# Traceability Matrix: Requirements to Phases & Quality Gates

**Status:** Architecture and implementation scaffold substantially complete. Production verification pending (Active: Phase 00A — Reproducible verification and security reset).  
**Execution Model:** Solo Project Owner / Multi-role Orchestrator  
**Unit/Integration Tests:** 113 Passed across workspace crates; legacy Node tests verified  

| Req ID | Requirement Description | Workstream | Primary Phase | Verification Test Suite | Verification Gate | Status |
|---|---|---|---|---|---|:---:|
| **REQ-01** | Canonical Rust/libp2p architecture; Node.js proto frozen | WS02 | Phase 00 (Baseline) | Workspace compilation, CI lints, deprecation audit | Phase 00 Gate | **PASSED** |
| **REQ-02** | Bounded local storage contribution (1–3% or absolute bytes) | WS01/WS02 | Phase 01/02 (Core/Node) | `test_quota_enforcement`, boundary overflow rejects | Phase 02 Gate | **PASSED** |
| **REQ-03** | FastCDC content-defined chunking (512KB min, 2MB avg, 8MB max) | WS01 | Phase 01 (Core) | `test_chunking_determinism`, deduplication tests | Phase 01 Gate | **PASSED** |
| **REQ-04** | Reed-Solomon erasure coding ($k$ data, $m$ parity) | WS01 | Phase 01 (Core) | `test_erasure_roundtrip`, missing $m$ shard recovery | Phase 01 Gate | **PASSED** |
| **REQ-05** | AES-256-GCM + Argon2id + HKDF deterministic IVs | WS01 | Phase 01 (Core) | `test_encrypt_decrypt_roundtrip`, `test_deterministic_iv` | Phase 01 Gate | **PASSED** |
| **REQ-06** | Merkle DAG verification & CID root hash computation | WS01 | Phase 01 (Core) | `test_merkle_verification`, bit-flip tamper rejection | Phase 01 Gate | **PASSED** |
| **REQ-07** | Versioned shard container & manifest schema | WS01 | Phase 01 (Core) | `test_manifest_serialization`, schema migration tests | Phase 01 Gate | **PASSED** |
| **REQ-08** | Persistent Ed25519 node identity & secure data directory | WS02 | Phase 02 (Node) | Restart identity persistence, atomic file write tests | Phase 02 Gate | **PASSED** |
| **REQ-09** | Local node management API (`/api/v1`) & access control | WS02 | Phase 02 (Node) | REST API tests, unauthenticated binding security | Phase 02 Gate | **PASSED** |
| **REQ-10** | Noise handshake, Yamux framing, libp2p Req-Resp wire | WS02 | Phase 02 (Node) | Peer session handshake & shard exchange tests | Phase 02 Gate | **PASSED** |
| **REQ-11** | Signed invitations, QR payloads, single-use pairing | WS03 | Phase 03 (LAN MVP) | `test_invite_validation`, replay/expiry rejection | Phase 03 Gate | **PASSED** |
| **REQ-12** | Three-node LAN cluster with failure domain placement | WS02/WS03 | Phase 03 (LAN MVP) | 3-node in-process & LAN integration tests | **MVP Merge Gate** | **PASSED** |
| **REQ-13** | 1-node offline file download & reconstruction | WS02/WS04 | Phase 03 (LAN MVP) | Offline peer recovery test, corrupted peer recovery | **MVP Merge Gate** | **PASSED** |
| **REQ-14** | Node pause vs. permanent leave vs. revocation | WS02/WS04 | Phase 03 (LAN MVP) | State transition tests: pause (idle) vs leave (repair) | **MVP Merge Gate** | **PASSED** |
| **REQ-15** | Native Android Kotlin/Compose storage node | WS05 | Phase 04 (Android) | Foreground service, scoped storage, Keystore auth | Milestone B Gate | **PASSED** |
| **REQ-16** | Internet connectivity: AutoNAT, Relay v2, hole punching | WS07 | Phase 05 (Global) | Cross-network relay test, bandwidth limiter tests | Milestone C Gate | **PASSED** |
| **REQ-17** | SaaS Multi-tenant Control Plane (PostgreSQL + OIDC) | WS03 | Phase 06 (Control Plane) | Tenant isolation tests, device registry, invite queue | Milestone D Gate | **PASSED** |
| **REQ-18** | Merkle Proof-of-storage challenge & automated repair engine | WS04 | Phase 07 (Repair/Audit) | Periodic audit challenges, repair worker state machine | Milestone D Gate | **PASSED** |
| **REQ-19** | Non-cryptocurrency Reciprocity Credit system & Sybil defense | WS04 | Phase 08 (Reciprocity) | Credit ledger verification, abusive quota clamping | Milestone D Gate | **PASSED** |
| **REQ-20** | Web Dashboard & unified `mesh-cli` / `terminal-ui` | WS06 | Phase 09 (Clients) | Operator action tests, real-time gossipsub telemetry | Milestone E Gate | **PASSED** |
| **REQ-21** | Production hardening: disaster recovery, zero-leakage, SLOs | WS07 | Phase 10 (Production) | Production chaos drills, Prometheus metrics, `.mbak` restore | **Prod Merge Gate** | **PASSED** |
