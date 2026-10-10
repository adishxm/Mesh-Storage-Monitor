# Master Plan

## Goal
Build a permissioned global peer-to-peer encrypted storage network from the current prototype.

## MVP
Three real devices enroll, contribute bounded storage, distribute encrypted Reed-Solomon shards, survive one node offline, reconstruct the original file, restart/reconcile, and expose status through CLI/web.

## Production
Add native Android, internet connectivity, relay/NAT traversal, SaaS tenancy, proof-of-storage, repair workers, reciprocity credits, signed releases, observability, security review, backups, and staged rollout.

## Status: Architecture and implementation scaffold substantially complete. Production verification pending.

## Ordered gates & Honest Phase Status
1. [/] Phase 00 — design/baseline: implemented; reproducible CI & pinned toolchain active
2. [/] Phase 01 — core: implemented; requires reproducible CI verification
3. [/] Phase 02 — single node: implemented; security/API hardening in progress
4. [ ] Phase 03 — LAN MVP: test code present; requires real-device demonstration
5. [ ] Phase 04 — Android: scaffold/bridge present; native packaging and device test pending
6. [ ] Phase 05 — global network: libp2p components present; cross-network proof pending
7. [ ] Phase 06 — control plane: in-memory prototype; PostgreSQL integration pending
8. [ ] Phase 07 — audits/repair: algorithm and tests present; operational worker pending
9. [ ] Phase 08 — reciprocity: prototype accounting present; adversarial validation pending
10. [/] Phase 09 — clients: CLI/dashboard present; credential handling hardening in progress
11. [ ] Phase 10 — production: checklist present; production gate not passed (Phase 00A active)
