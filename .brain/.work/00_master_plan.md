# Master Plan

## Goal
Build a permissioned global peer-to-peer encrypted storage network from the current prototype.

## MVP
Three real devices enroll, contribute bounded storage, distribute encrypted Reed-Solomon shards, survive one node offline, reconstruct the original file, restart/reconcile, and expose status through CLI/web.

## Production
Add native Android, internet connectivity, relay/NAT traversal, SaaS tenancy, proof-of-storage, repair workers, reciprocity credits, signed releases, observability, security review, backups, and staged rollout.

## Status: COMPLETED (All 10 Ordered Gates Passed)

## Ordered gates
1. [x] Repository baseline and canonical Rust decision. (Phase 00)
2. [x] Pure storage core and versioned formats. (Phase 01)
3. [x] Reliable single Rust node. (Phase 02)
4. [x] Three-node LAN MVP. (Phase 03)
5. [x] Native Android node. (Phase 04)
6. [x] Internet relay/NAT traversal. (Phase 05)
7. [x] SaaS control plane and tenant isolation. (Phase 06)
8. [x] Audits, repair, reciprocity, and abuse resistance. (Phase 07 & 08)
9. [x] Web/CLI product completion. (Phase 09)
10. [x] Production hardening and release. (Phase 10)
