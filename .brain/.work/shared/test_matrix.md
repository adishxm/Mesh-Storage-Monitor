# Test Matrix

| Area | Unit | Integration | Failure | Security |
|---|---|---|---|---|
| Chunking | boundaries, determinism | file round trip | truncation | malformed input |
| Erasure | encode/reconstruct | 3-node recovery | missing/corrupt shards | shard substitution |
| Encryption | key/nonce/header | distributed decrypt | wrong key | plaintext leakage |
| Node | state/quota | peer sessions | restart/offline | revoked peer |
| Join | signature/expiry | device enrollment | reuse/wrong issuer | replay/Sybil |
| Repair | placement/state | worker flow | duplicate jobs | unauthorized repair |
| Android | identity/storage | real device | process death/battery | keystore/access |
| SaaS | authorization | tenant workflows | queue retry | cross-tenant access |
