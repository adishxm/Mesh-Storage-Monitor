# Master Plan

## Goal
Build a permissioned global peer-to-peer encrypted storage network from the current prototype.

## MVP
Three real devices enroll, contribute bounded storage, distribute encrypted Reed-Solomon shards, survive one node offline, reconstruct the original file, restart/reconcile, and expose status through CLI/web.

## Production
Add native Android, internet connectivity, relay/NAT traversal, SaaS tenancy, proof-of-storage, repair workers, reciprocity credits, signed releases, observability, security review, backups, and staged rollout.

## Status: Architecture and implementation scaffold substantially complete. Phase 00B authentication & verification active.

## Ordered gates & Honest Phase Status
1. [/] Phase 00 — design/baseline: implemented; reproducible CI & pinned toolchain active (`CI-configured`)
2. [/] Phase 01 — core: implemented; verified locally across 62 tests; CI runner confirmation pending
3. [/] Phase 02 — single node: implemented; API authentication & loopback binding hardened
4. [ ] Phase 03 — LAN MVP: test suite passes; real 3-device demonstration pending
5. [/] Phase 04 — Android: `cargo-ndk` pipeline, ABI targets, and instrumentation test scaffolded; physical APK build pending
6. [ ] Phase 05 — global network: libp2p components present; cross-network WAN proof pending
7. [/] Phase 06 — control plane: `PostgresRepository` with migrations & row locking added; `FilePersistentRepository` designated as single-process persistent prototype; Bearer JWT token verification implemented
8. [ ] Phase 07 — audits/repair: algorithm and tests present; operational worker pending
9. [ ] Phase 08 — reciprocity: accounting model and tests present; adversarial validation pending
10. [/] Phase 09 — clients: CLI/dashboard present; credential & authentication guards active
11. [ ] Phase 10 — production: checklist present; production gate not passed (Phase 00B active)
