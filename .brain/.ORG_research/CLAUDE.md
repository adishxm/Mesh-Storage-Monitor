# CLAUDE.md

## Project

"mesh-storage": a decentralized, peer-to-peer storage network running on my
local Wi-Fi. This PC and any phone/laptop on the same network run the same
binary as peers. Files are content-defined-chunked, Reed-Solomon
erasure-coded, AES-256-GCM encrypted, and spread across peers via libp2p.
There is no central server — every peer is equal; this PC is just the
well-known "anchor" address others dial first.

Full rationale, worked examples, security model, and operations playbook
live in `docs/ARCHITECTURE.md`. Read it before starting a milestone if
anything below is ambiguous — it is the source of truth.

## Fixed architecture decisions — do not change without asking first

| Concern | Decision |
|---|---|
| Language | Rust for `/core`, `/node`, `/mesh-cli` |
| Networking | libp2p — Kademlia DHT, mDNS, Noise transport, gossipsub, ping |
| Identity | Ed25519 keypair = libp2p PeerId. No CA, no TLS certs. |
| Access control | `trusted_peers.json` allowlist per node; unlisted PeerIds rejected after handshake |
| Brute-force defense | 5 rejected connections from one IP in 60s → OS firewall ban for 30 min |
| Chunking | FastCDC, ~2MB average chunk size |
| Integrity | SHA-256 + Merkle DAG; root hash verified on every read |
| Erasure coding | `reed-solomon-erasure` crate, configurable (k,m); k+m ≤ peer count |
| Encryption | AES-256-GCM per shard; Argon2id (passphrase→master key) + HKDF (per-file key) |
| Audits | Periodic Merkle-proof challenge/response; failure → `degraded` → repair |
| Repair | Reconstruct from any k of (k+m) shards, re-place on a healthy peer |
| Lifecycle | "stop" = pause, no repair triggered. "leave" = permanent, triggers repair. Keep these as separate commands. |
| Deletion | `pending_deletion` flag → 30 days → per-node daily purge job |
| Dashboards | React (`/web-dashboard`) + ink/ratatui (`/terminal-ui`) — both read any peer's local status API + gossipsub feed |

## Repository layout

```
/core            Rust crate: chunking, Merkle DAG, RS encode/decode, crypto. No networking.
/node            Rust binary: wraps /core with libp2p. Runs on every device. Anchor vs peer = config flag.
/mesh-cli        Rust CLI wrapping /node's local status API.
/web-dashboard   React + Vite + recharts.
/terminal-ui     ink (Node) or ratatui (Rust).
/docs            ARCHITECTURE.md — full spec.
```

## Build order

Work through these in order. Do not start milestone N+1 until N's "Done
when" passes. Write tests alongside implementation, not after.

1. `/core` library: FastCDC + Merkle DAG + Reed-Solomon + AES-256-GCM /
   Argon2id / HKDF. Pure functions, no I/O beyond temp files.
   Done when: a 20MB sample file round-trips byte-identical at (k=2,m=1);
   deleting any `m` shards still reconstructs; a flipped bit in one shard
   is caught by Merkle verification before decode runs.

2. Single-node networking: wrap `/core` in a libp2p node; Axum status API
   (`/status`, `/peers`, `/shards`).
   Done when: two `/node` instances on different ports on this PC discover
   each other via mDNS and appear in each other's `/peers`.

3. Placement + manifests: (k,m) shard placement, manifest replication.
   Done when: a 20MB file uploaded at (k=1,m=1) on instance #1 downloads
   correctly via instance #2.

4. Multi-device pairing: QR/code pairing exchanging multiaddr + PeerId;
   `trusted_peers.json`; firewall auto-ban on unlisted-peer floods.
   Done when: a second physical device on the same Wi-Fi pairs and appears
   in `/peers`; an unpaired third process is rejected, then IP-banned after
   5 attempts.

5. Self-healing: ping-based liveness, `degraded` state, repair job.
   Done when: stopping one peer triggers reconstruction and re-placement of
   its shards within one repair cycle.

6. Integrity audits: periodic Merkle-proof challenge/response feeding the
   repair pipeline.
   Done when: manually corrupting a stored shard on disk is detected by the
   next audit cycle, without anyone downloading the file.

7. Dashboards: `/web-dashboard` + `/terminal-ui` consuming the gossipsub
   feed from any peer's local API.
   Done when: pointing the dashboard at any peer (not just the anchor)
   shows live upload/download/repair events.

8. Lifecycle: stop vs. leave; `pending_deletion` → 30-day → purge.
   Done when: "leave" triggers full repair before exit; a manifest flagged
   30+ days ago is purged on the next daily tick, on whichever node is next
   online.

9. Stretch: CRDT metadata for synced-folder semantics; Tauri Mobile build
   of `/core` + `/node`.

## Conventions

- `cargo fmt` and `cargo clippy -- -D warnings` must pass before any commit.
- Every `/core` function gets a unit test in the same file (`#[cfg(test)]`).
- `/node` integration tests run N in-process libp2p nodes on `127.0.0.1`
  with random ports — never require real network access for tests.
- `/core` errors: `thiserror`. `/node`, `/mesh-cli` errors: `anyhow`.
- Gossipsub event payloads are JSON with a `type` field matching the event
  names in `docs/ARCHITECTURE.md` §7 — both dashboards depend on this shape.
- `/node` binds its API and libp2p listener to `0.0.0.0`, never `127.0.0.1`
  — other devices on the Wi-Fi must be able to reach it.

## My hardware

Anchor peer runs here: i5-210H, 16GB DDR5, RTX 4050 6GB, 512GB NVMe. The GPU
is unused by `/core`/`/node` for milestones 1-8 — don't add CUDA/GPU deps.
