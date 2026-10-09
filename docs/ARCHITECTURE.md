# Decentralized Mesh Storage Network — v2 Architecture & Operations Documentation

**Status:** Supersedes the v1 prototype spec. Every component below is chosen
because it removes a single point of failure, a custom-crypto risk, or an
operational ambiguity that the v1 design either had or left unanswered.

---

## 1. Design Philosophy

v1 had a coordinator: one machine that held the file manifest database, ran
the local certificate authority, and hosted the dashboard's data feed. That
machine was the brain. v2 has no brain — only peers. Every node:

- holds its own slice of the network's metadata,
- authenticates itself using nothing but its own keypair (no CA),
- can answer "what does the network look like from here" independently,
- and can leave or rejoin without anyone's permission or notice.

The five guarantees a storage network owes its user — **confidentiality,
integrity, availability, authentication, auditability** — are each backed by
a specific, named mechanism in this document, not by "the coordinator says
it's fine."

---

## 2. Network Topology

| Role | Device | Specs |
|---|---|---|
| Anchor peer (well-known address) | Your PC | i5-210H, 16GB DDR5, RTX 4050 6GB, 512GB NVMe |
| Peer 1..N | Any laptop/phone/PC on the same Wi-Fi | varies |

**"Anchor peer" is not a privileged role.** It is simply the address every
other device dials first, the way an IPFS bootstrap node works. Your PC
holds no special keys, no central database, and no authority over the
network. It's an anchor in the sense of "first phone number you call," not
"the office."

Discovery on the LAN uses **mDNS** for same-subnet peers and falls back to
**Kademlia DHT** lookups for anything mDNS doesn't catch (useful once you're
testing across VLANs or guest networks).

---

## 3. Core Architecture

### 3.1 Peer Identity & Transport Security (replaces v1's HMAC + planned CA)

Each node generates an **Ed25519 keypair** on first run. The public key
*is* the node's identity (its libp2p `PeerId`) — there is nothing to issue,
sign, or revoke. When two peers connect, the **Noise protocol** handshake
does three things in one step: each side proves it holds the private key
matching its claimed identity, both sides authenticate each other, and the
channel is encrypted (ChaCha20-Poly1305) for the rest of the session.

This is a deliberate refinement over the "mTLS + local CA" idea: a CA is
itself a small centralization smell, and it's unnecessary here — libp2p's
self-certifying identities give you the same mutual-auth + encrypted-channel
guarantee with zero extra infrastructure.

**Access control** is layered on top, separately: every node keeps a local
`trusted_peers.json` — the list of PeerIds it has paired with. A connection
from an unlisted PeerId is rejected immediately after the handshake, before
any request is processed. Repeated connection attempts from unlisted peers
(same source IP, multiple tries) trigger an **OS-level firewall ban**
(`iptables`/`ufw` on Linux, `netsh advfirewall` on Windows) for that IP —
this is your "brute attack → crashed/unreachable" behavior, now enforced at
two layers instead of one custom HMAC check.

### 3.2 Content-Defined Chunking + Merkle DAG (replaces fixed 2MB chunks)

Files are split using **FastCDC** (a rolling-hash content-defined chunker),
producing variable-size chunks averaging ~2MB. The benefit over fixed-size
chunking: if you re-upload a slightly modified file, only the genuinely
changed chunks produce different hashes — everything else is recognized as
already-stored and deduplicated automatically.

Every chunk, shard, and the file itself gets a content address (SHA-256
hash, formatted as a CID-style identifier). These hashes form a **Merkle
DAG**: the file's root hash is derived from its chunk hashes, which are
derived from their shard hashes. Verifying a downloaded file means
recomputing the root hash and comparing — any single-bit corruption anywhere
in the tree is immediately detectable.

### 3.3 Reed-Solomon Erasure Coding

Each chunk is split into `k` data shards + `m` parity shards using the
`reed-solomon-erasure` crate (Rust, AVX2-accelerated). Any `k` of the
`(k+m)` shards reconstruct the chunk — you can lose any `m` shard-holders
entirely.

**The placement rule that matters:** `k + m` should not exceed your node
count, so no single node ever holds two shards of the same chunk (otherwise
that node's loss is correlated across both shards it holds).

| Node count | Recommended (k, m) | Tolerates losing | Storage overhead |
|---|---|---|---|
| 2 | (1, 1) — pure mirror | 1 node | 100% |
| 3 | (2, 1) | 1 node | 50% |
| 5 | (3, 2) | 2 nodes | 67% |
| 8 | (5, 3) | 3 nodes | 60% |

**Worked example — your 20MB test file, 3-node network, (k=2, m=1):**
FastCDC produces roughly 8–12 chunks (~2MB avg). Each chunk → 3 shards of
~0.7–1MB. Each chunk's 3 shards are placed on 3 different nodes — exactly
your node count, so every node holds exactly one shard per chunk. Any single
node going offline still leaves `k=2` shards per chunk available everywhere.

### 3.4 Encryption (data-at-rest, independent of transport security)

Transport security (3.1) protects data *in flight* between peers who are
both legitimate network members. It does **not** mean a storage node can
read what it's storing — that's a separate guarantee:

Each shard is encrypted with **AES-256-GCM** before it ever leaves the
uploading device. The key comes from your passphrase via **Argon2id**
(memory-hard, brute-force resistant), then **HKDF** derives a unique subkey
per file. A storage node — even a fully trusted, correctly-behaving peer —
holds only ciphertext + hash. There is nothing to leak.

### 3.5 Proactive Integrity Audits (new in v2 — replaces "wait for heartbeat failure")

v1 only discovered problems when a node went offline. v2 adds periodic
**challenge-response audits**: every few hours, a node that placed a shard
elsewhere sends the holder a random nonce and asks for `hash(shard ||
nonce)` — proof the holder still has the actual bytes, without transferring
them. A correct, fast response proves possession. A failed or missing
response marks that node `degraded` and queues a repair — catching silent
data loss (disk corruption, accidental deletion outside the app) *before*
you ever try to download the file.

### 3.6 Self-Healing / Repair Engine

Liveness is tracked via libp2p's `ping` protocol. A peer that fails pings
*or* integrity audits beyond a threshold is marked `degraded`. The repair
engine then: finds every chunk with a shard on that peer, reconstructs the
missing shard from the remaining `k` shards, and places it on a healthy
peer with available quota — restoring full `(k,m)` redundancy.

### 3.7 CRDT-Based Metadata (future phase — for true multi-device sync)

Everything above treats this as a "store a blob, retrieve a blob" network.
If you later want the network to behave like a synced folder — where
multiple devices can add, rename, or delete files concurrently — file-tree
metadata needs to be a **CRDT** (e.g., an OR-Set with tombstones for
deletions). This avoids "last write wins" data loss when two devices edit
the same logical folder while briefly disconnected from each other. This is
the same class of solution used by offline-first collaborative apps; it's
listed here as a defined future phase, not part of the core storage engine.

---

## 4. Security & Trust Model — Summary Table

| Guarantee | Mechanism | Section |
|---|---|---|
| Confidentiality | AES-256-GCM per shard; Argon2id+HKDF keys; nodes hold ciphertext only | 3.4 |
| Integrity | SHA-256 content hashes + Merkle DAG, verified on every retrieval | 3.2 |
| Availability | Reed-Solomon (k,m); tolerates `m` simultaneous node losses | 3.3 |
| Authentication | Ed25519 PeerIDs + libp2p Noise mutual handshake (no CA) | 3.1 |
| Auditability | Periodic Merkle-proof challenges detect silent loss/corruption | 3.5 |
| Access control | Per-node `trusted_peers.json` allowlist + auto IP-ban on repeated unknown connections | 3.1 |

This is the same primitive set used by production systems in this space —
the difference between this and a fully-audited commercial network is years
of third-party security review, not a missing mechanism.

---

## 5. Storage Contribution Model

Unchanged from v1's principle, re-verified against the new architecture:
each node's agent scans free disk on first run and the user sets a
contribution percentage (default 1.5%), enforced locally as a hard quota.
The node reports `used/quota` during its periodic pings, which feeds both
the repair engine's placement decisions and the dashboard's storage bars.

| Node | Total | At 1.5% |
|---|---|---|
| Laptop (1TB) | 1,000 GB | ~15 GB |
| Mobile (256GB) | 256 GB | ~3.8 GB |
| Computer (4TB) | 4,000 GB | ~60 GB |

---

## 6. Operations Playbook

### 6.1 Cold start (first time ever)

1. Your PC starts its node-agent in **anchor mode** — same binary, just
   configured with a fixed address other peers will dial first.
2. It generates its Ed25519 identity, prints its multiaddr + PeerId as a QR
   code/pairing code in the terminal.
3. Each additional device runs the same binary, scans/enters the pairing
   code, connects, and exchanges PeerIds — both sides add each other to
   `trusted_peers.json`.
4. Each peer now has the others in its local routing table. The DHT is
   "born" at this point — there's no separate "create the network" step.

### 6.2 Normal start (every time after)

Every node, including your PC, just starts. Each loads its persisted
identity + routing table + manifest cache from disk and reconnects to
whichever cached peers are reachable. **Order doesn't matter** — your PC is
no longer required to start first.

### 6.3 If your PC (the anchor) is offline

Peers that already paired don't need it — they have each other in their
routing tables and talk directly. The anchor's *only* special property is
being the address a brand-new device dials for its first-ever pairing; once
paired, that device never needs the anchor again. Practically, at 2-3 nodes
your PC is also likely your largest contributor, so its shards being
unavailable may push some chunks to their reconstruction limit until it
returns — but the *network* (routing, peer discovery, other nodes' data) is
unaffected.

### 6.4 Graceful stop vs. permanent leave

These must be distinct actions:

- **Stop** (Ctrl-C / normal shutdown): broadcast a `node_pausing` event to
  reachable peers, persist routing table + manifest cache + held shards to
  disk, exit. **No repair is triggered.** This is what you use between test
  sessions.
- **Leave** (explicit "remove this node permanently"): broadcast
  `node_leaving`, and this *does* trigger the repair engine to reconstruct
  and re-place every shard this node held, before it's considered safe to
  decommission the device.

For prototype testing, stop your peer nodes first, your PC last — purely so
your terminal/dashboard (likely running on your PC) captures the other
nodes' exit events before it exits itself.

### 6.5 If every node stops — "network dark" vs. "data broken"

These are different things. Network uptime is the union of all peers'
uptime — at 2-3 nodes, periods where nobody is running are the *normal*
default state between sessions, not a failure mode. As long as every node
did a **stop** (6.4) rather than a **leave**, each one is holding its shards
and routing table dormant on disk. The moment any single node restarts, the
network exists again from that node's perspective, and the rest reassembles
as peers reconnect.

What *actually* weakens the system is **permanent departures relative to
your redundancy floor** (table in 3.3) — e.g., with (k=2,m=1) across 3
nodes, one permanent "leave" already puts every chunk at the bare minimum;
a second permanent loss without repair completing in between loses data.
This is a function of `k+m` vs. surviving node count, not of anyone being
powered off at any given moment.

### 6.6 Account deletion → 30-day retention → purge

1. User-initiated deletion flags the account's manifests `pending_deletion`
   with a timestamp — data and shards are untouched, fully recoverable by
   clearing the flag.
2. A daily job on each node checks its locally-held manifests for entries
   past 30 days in `pending_deletion`.
3. For each, the node deletes its local shard copies and removes the
   manifest entry, logging the purge event.
4. Because this check runs **per-node** (not from a central scheduler),
   purging doesn't depend on any one machine being online on day 30 — each
   node purges on its own schedule whenever it's next active.

---

## 7. Monitoring

There is no "global dashboard" in the v1 sense, because there is no global
authority to ask. Instead:

- **Per-node local status API** (`GET /status`, `/peers`, `/shards` on
  `localhost:<port>`): every node, including your PC, exposes its own view —
  which peers it can currently reach, what it's storing, recent audit
  results.
- **`mesh-cli status` / `nodes` / `files` / `peers`**: a thin CLI wrapping
  the local status API — works over SSH on any headless node.
- **Event stream via gossipsub**: every node publishes
  `node_pausing/leaving`, `upload/download progress`, `shard_repaired`,
  `audit_failed`, `account_purged` to a shared pub/sub topic. The React
  dashboard and the terminal UI can subscribe through **whichever node
  they're connected to** — point them at your phone instead of your PC and
  you get a live (if partial — that node's-eye-view) picture even with your
  PC off.
- **Optional `/metrics` (Prometheus format)** per node for historical
  graphs (storage over time, audit pass rate, repair frequency) — trivial to
  add since the events are already structured.

---

## 8. Tech Stack & Repository Layout

```
/core            Rust crate: FastCDC chunking, Merkle DAG, Reed-Solomon
                 (reed-solomon-erasure), AES-256-GCM, Argon2id+HKDF.
                 Pure logic, zero networking — fully unit-testable alone.

/node            Rust binary (the only thing that runs on every device).
                 Wraps /core with libp2p: identity, Noise transport,
                 Kademlia DHT, mDNS, gossipsub, ping. Exposes the local
                 status API (Axum) for mesh-cli and dashboards.

/mesh-cli        Thin Rust CLI calling the local status API.

/web-dashboard   React + Vite + recharts. Connects to any /node's
                 status API + gossipsub-relayed WebSocket feed.

/terminal-ui     ink (Node.js) or ratatui (Rust) — same event feed as
                 web-dashboard, for terminal-only monitoring.
```

A single Rust binary for `/node` (rather than splitting "coordinator" vs.
"agent" as in v1) is itself a v2 simplification: anchor vs. peer is now just
a config flag, not a different program.

---

## 9. Build Roadmap

The order below front-loads the hard, pure-logic pieces — fully testable
without any networking — before introducing the complexity of a live P2P
network. This is the sequence that catches algorithmic bugs cheaply, before
they're tangled up with network timing issues.

1. **`/core` library**: FastCDC chunking → Merkle DAG → Reed-Solomon
   encode/decode → AES-256-GCM + Argon2id/HKDF. Unit tests: encode a file,
   decode it back byte-identical; corrupt a shard, confirm detection;
   delete `m` shards, confirm reconstruction.
2. **Single-node networking**: wrap `/core` in a libp2p node; identity,
   local status API. Run two instances on your PC (different ports), verify
   mDNS discovery + DHT routing between them.
3. **Storage placement + manifests**: implement (k,m) shard placement and
   manifest replication across peers. Upload/download across the two local
   instances at (k=1, m=1).
4. **Multi-device**: package `/node` for a second physical device; pairing
   flow via QR'd multiaddr+PeerId; `trusted_peers.json` allowlist + firewall
   auto-ban on unlisted-peer connection attempts.
5. **Self-healing**: ping-based liveness, `degraded` state, repair job.
6. **Integrity audits**: periodic Merkle-proof challenge/response, feeding
   the same repair pipeline.
7. **Dashboards**: `/web-dashboard` and `/terminal-ui`, both consuming the
   gossipsub event feed from any node.
8. **Lifecycle**: stop vs. leave, 30-day `pending_deletion` → purge.
9. **Stretch**: CRDT metadata layer for synced-folder semantics; mobile
   builds via Tauri Mobile wrapping `/core` + `/node`.

---

## Appendix A — First Local Test, Step by Step

1. Start instance #1 of `/node` on your PC in anchor mode. It binds to your
   LAN IP (`0.0.0.0`, **not** `127.0.0.1` — otherwise other devices can't
   reach it), prints its pairing QR.
2. Start instance #2 — either a second `/node` process on a different port
   on the same PC, or your phone on the same Wi-Fi. Pair using the QR code;
   both write each other to `trusted_peers.json`.
3. **For this first test, set (k=1, m=1)** even if you plan (k=2,m=1) later
   — with exactly 2 peers, (k=2,m=1) needs 3 shard-slots across 2 nodes,
   meaning one node holds 2 shards of the same chunk, so killing *that*
   specific node drops you below the reconstruction threshold while killing
   the *other* one doesn't. (k=1,m=1) — pure mirroring — makes either node's
   loss recoverable, which is the predictable result you want for a first
   run.
4. Upload your 20MB test file. Watch the terminal stream chunk/shard/place
   events; watch storage usage tick up on both peers.
5. Ctrl-C (stop, not leave) one peer. Confirm the other shows it as
   unreachable in `mesh-cli peers`.
6. Download the file with that peer still stopped — confirm successful
   reconstruction from the surviving mirror.
7. Restart the stopped peer — confirm it reconnects from cached routing
   info without needing to re-pair.
8. **First-run gotchas**: your OS firewall will prompt to allow the `/node`
   binary's inbound connections — accept it on every device, or peers on
   other devices won't be able to dial in at all.

---

## Appendix B — Agent Kickoff Prompt (Phase 1 of the Roadmap)

> Build a Rust crate called `core` implementing: (1) FastCDC content-defined
> chunking producing ~2MB average chunks; (2) a Merkle DAG over chunk and
> shard hashes (SHA-256) with root-hash verification; (3) Reed-Solomon
> encode/decode via the `reed-solomon-erasure` crate with configurable (k,m);
> (4) AES-256-GCM encryption of each shard, with key derivation via Argon2id
> (passphrase → master key) and HKDF (master key + file ID → per-file key).
> No networking in this crate — pure functions operating on in-memory bytes
> and temp files.
>
> Write unit tests that: take a ~20MB sample file, run it through the full
> chunk→encode→encrypt pipeline at (k=2,m=1), then decrypt→decode→reassemble
> and assert byte-identical output; separately, delete one shard from each
> chunk before decoding and assert reconstruction still succeeds; separately,
> flip a bit in one shard and assert Merkle verification detects it before
> decode is attempted.
>
> Once these tests pass, stop — networking is a separate phase.
