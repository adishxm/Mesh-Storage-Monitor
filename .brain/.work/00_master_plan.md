# Master Plan

## Goal
Build a permissioned global peer-to-peer encrypted storage network from the current prototype.

## MVP
Three real devices enroll, contribute bounded storage, distribute encrypted Reed-Solomon shards, survive one node offline, reconstruct the original file, restart/reconcile, and expose status through CLI/web.

## Production
Add native Android, internet connectivity, relay/NAT traversal, SaaS tenancy, proof-of-storage, repair workers, reciprocity credits, signed releases, observability, security review, backups, and staged rollout.

## Ordered gates
1. Repository baseline and canonical Rust decision.
2. Pure storage core and versioned formats.
3. Reliable single Rust node.
4. Three-node LAN MVP.
5. Native Android node.
6. Internet relay/NAT traversal.
7. SaaS control plane and tenant isolation.
8. Audits, repair, reciprocity, and abuse resistance.
9. Web/CLI product completion.
10. Production hardening and release.
