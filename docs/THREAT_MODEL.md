# Mesh Storage Monitor — Threat Model & Security Posture

**Framework:** STRIDE-based Decentralized Threat Analysis  
**Audience:** Security Engineers & System Implementers  

---

## 1. Trust Boundaries & Security Domains

1. **Client Trust Domain (Upload/Download Device)**:
   - Contains plaintext file and master user passphrase.
   - Responsible for FastCDC chunking, Reed-Solomon encoding, and AES-256-GCM encryption before data leaves memory.
   - Master passphrase and file keys NEVER cross network boundaries.
2. **Peer Shard Storage Domain (Untrusted Remote Peers)**:
   - Peers hold only encrypted, content-addressed shard payloads and metadata headers.
   - Peers cannot decrypt shards without passphrase & salt.
   - Even if a malicious actor controls all peer nodes, confidentiality remains intact.
3. **Control Plane Domain (SaaS Metadata Store)**:
   - Stores device registries, public keys, signed invitations, audit results, and quota accounting.
   - Blind to plaintext content and cryptographic keys.

---

## 2. Threat Analysis (STRIDE)

| Threat Category | Attack Vector | Mitigating Control |
|---|---|---|
| **Spoofing** | Untrusted device claiming arbitrary PeerId | libp2p Noise protocol with Ed25519 cryptographic handshake proves private key ownership. |
| **Tampering** | Shard corrupted on disk or in transit | SHA-256 Merkle DAG verification before reconstruction/decryption. Tampered shards are discarded and reconstructed from parity. |
| **Repudiation** | Node claiming it stores shards without retaining bytes | Proof-of-storage challenge/response audits using random nonces. |
| **Information Disclosure** | Eavesdropping on LAN/WAN traffic or reading disk | Noise transport encryption on wire; AES-256-GCM authenticated encryption at rest. |
| **Denial of Service** | Flooding node with connection requests / Sybil attack | Allowlist validation immediately post-Noise handshake; 5 failed attempts in 60s triggers IP-level OS firewall ban. |
| **Elevation of Privilege** | Node attempting to issue join invitations | Only authorized issuer public keys are accepted by joiners; invitations contain nonces and cryptographic signatures. |

---

## 3. Cryptographic Invariants

- **Argon2id**: Memory-hard key derivation prevents GPU/ASIC brute force on weak passphrases.
- **Deterministic HKDF IVs**: Eliminates Merkle root mutation during repair or self-healing.
- **Zero Plaintext Leakage**: Plaintext never touches intermediate caches, logs, or unencrypted storage.
