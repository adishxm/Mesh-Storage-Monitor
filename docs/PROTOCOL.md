# Mesh Storage Network — Wire Protocol Specification

**Protocol Version:** `/mesh/1.0.0`  
**Base Transport:** libp2p (Noise + Yamux + TCP/QUIC)  

---

## 1. Sub-Protocols & Message Schemas

### A. Discovery & Routing
- `/mesh/kad/1.0.0`: Kademlia Distributed Hash Table for peer routing and provider records.
- `mdns`: Multicast DNS on local Wi-Fi for zero-configuration discovery of anchor/peer nodes.

### B. PubSub Event Stream
- **Topic:** `mesh-events`
- **Engine:** libp2p Gossipsub v1.1
- **Payload:** Authenticated JSON events (`upload_complete`, `shard_stored`, `node_pause`, `repair_triggered`).

### C. Shard Operations (Request-Response)
- **Protocol ID:** `/mesh/req-resp/1.0.0`
- **Messages:**
  1. `Store`: Uploads a shard to a target peer.
  2. `Retrieve`: Fetches an encrypted shard by its SHA-256 hash.
  3. `AuditChallenge`: Verifies presence of shard with a 16-byte random nonce.
  4. `Pair`: Establishes reciprocal peer relationship using signed invitation credentials.

---

## 2. Session Handshake
1. TCP / QUIC transport connection established.
2. Noise handshake completes: Ed25519 public keys exchanged; mutual authentication achieved.
3. Node validates peer against `trusted_peers.json` allowlist.
4. If peer is not allowlisted and does not present a valid invitation nonce, connection terminates with `StreamReset`.
