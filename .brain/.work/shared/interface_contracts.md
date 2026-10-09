# Canonical Shared Interface Contracts

**Version:** 2.0.0  
**Status:** Frozen for MVP Implementation  

---

## 1. Shard Container & Header (Wire & Disk Format)

Each encrypted shard stored on disk or transferred over the network conforms to a versioned container layout:

```text
+-----------------------+----------------------------------+
| Magic Bytes (4B)      | b"MSHS" (Mesh Shard)             |
| Format Version (2B)   | 0x0001 (Version 1)               |
| Header Length (2B)    | u16 length of JSON metadata      |
| Header JSON (var)     | ShardHeader payload              |
| Raw Payload (var)     | Encrypted AES-256-GCM bytes      |
+-----------------------+----------------------------------+
```

### ShardHeader JSON:
```json
{
  "version": 1,
  "file_id": "urn:uuid:7f6b9c2a-9e11-4a77-a8b2-6c1737e1b52a",
  "chunk_idx": 0,
  "shard_idx": 1,
  "shard_hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  "k": 2,
  "m": 1,
  "payload_len": 1048604,
  "created_at": 1775779200
}
```

---

## 2. File Manifest Schema (v2)

The manifest is generated client-side after FastCDC chunking, Reed-Solomon encoding, and AES-256-GCM encryption. It is signed by the client device's key.

```json
{
  "schema_version": 2,
  "file_id": "urn:uuid:7f6b9c2a-9e11-4a77-a8b2-6c1737e1b52a",
  "file_name": "backup.tar.gz",
  "original_len": 20971520,
  "root_hash": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
  "k": 2,
  "m": 1,
  "salt_hex": "4a7f20...",
  "created_at": 1775779200,
  "chunks": [
    {
      "chunk_idx": 0,
      "chunk_hash": "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae",
      "original_len": 2097152,
      "shards": [
        {
          "shard_idx": 0,
          "shard_hash": "fcde2b2edba56bf408601fb721fe9b5c338d10ee429ea04fae5511b68fbf8fb9",
          "holder_peer_id": "12D3KooWDpJ7As7BWAwRMfu1VU2WCqnjvq387JEYKDBj4kx6nXTN"
        },
        {
          "shard_idx": 1,
          "shard_hash": "8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4",
          "holder_peer_id": "12D3KooWStC8S6uRzT5eB3jV49jMeq5YhHqFz7J9R7e29gU8xabc"
        },
        {
          "shard_idx": 2,
          "shard_hash": "37f48a901804245be54c5e3d7daeb8248bfbf9e8a7ea3ee8f9e0eb37456d9539",
          "holder_peer_id": "12D3KooWJ8eS98uRzT5eB3jV49jMeq5YhHqFz7J9R7e29gU8zxyz"
        }
      ]
    }
  ]
}
```

---

## 3. Node Identity & Capabilities

```json
{
  "peer_id": "12D3KooWDpJ7As7BWAwRMfu1VU2WCqnjvq387JEYKDBj4kx6nXTN",
  "public_key_multibase": "z6MkuT4G...",
  "device_name": "Aditya-Desktop",
  "device_type": "desktop",
  "version": "0.1.0",
  "storage_quota_bytes": 10737418240,
  "storage_used_bytes": 104857600,
  "available_bytes": 10632560640,
  "capabilities": ["storage", "relay", "audit"],
  "state": "active",
  "last_seen": 1775779200
}
```

---

## 4. Signed Invitation (QR / Enrollment Payload)

An invitation is issued by an authorized admin/owner, single-use, cryptographically signed, with an explicit expiration:

```json
{
  "invitation_id": "inv_9a8b7c6d5e4f",
  "network_id": "mesh-alpha",
  "organization_id": "org_default",
  "issuer_peer_id": "12D3KooWDpJ7As7BWAwRMfu1VU2WCqnjvq387JEYKDBj4kx6nXTN",
  "issuer_signature": "3045022100...",
  "bootstrap_addrs": [
    "/ip4/192.168.1.100/tcp/4001/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqnjvq387JEYKDBj4kx6nXTN"
  ],
  "expires_at": 1775780100,
  "single_use_nonce": "98a76fbc54d32e10"
}
```

---

## 5. Local Node Management REST API (`/api/v1`)

| Endpoint | Method | Purpose | Authentication |
|---|---|---|---|
| `/api/v1/status` | GET | Comprehensive node state, quota, peers, storage health | Local token / loopback |
| `/api/v1/peers` | GET | List of connected & trusted peers with latency & role | Local token / loopback |
| `/api/v1/shards` | GET | List of local shards held with file ID & byte size | Local token / loopback |
| `/api/v1/quota` | GET/POST | Query or configure contribution limit (bytes or %) | Local token / loopback |
| `/api/v1/invite/create` | POST | Generate a signed single-use invitation & QR data | Local token / loopback |
| `/api/v1/invite/join` | POST | Accept and consume an invitation | Local token / loopback |
| `/api/v1/pause` | POST | Pause node (grace period: preserve shards, cease serving) | Local token / loopback |
| `/api/v1/leave` | POST | Permanent leave (triggers repair jobs for held shards) | Local token / loopback |
| `/api/v1/upload` | POST | Client-side chunk, encode, encrypt, and place file | Local token / loopback |
| `/api/v1/download/:file_id`| GET | Retrieve shards, verify Merkle DAG, reconstruct & decrypt | Local token / loopback |

---

## 6. P2P Request-Response Protocol (`/mesh/req-resp/1.0.0`)

```rust
pub enum ShardRequest {
    Store {
        file_id: String,
        chunk_idx: usize,
        shard_idx: usize,
        shard_hash: Hash256,
        data: Vec<u8>,
    },
    Retrieve {
        shard_hash: Hash256,
    },
    AuditChallenge {
        shard_hash: Hash256,
        nonce: [u8; 16],
    },
    Pair {
        caller_multiaddr: String,
        invitation_nonce: Option<String>,
    },
}

pub enum ShardResponse {
    StoreAck { success: bool, reason: Option<String> },
    RetrieveAck { data: Option<Vec<u8>> },
    AuditResponse { proof: [u8; 32] },
    PairAck { success: bool, reason: Option<String> },
    Error(String),
}
```
