# Reference Prototype (Legacy Node.js Implementation)

> [!WARNING]
> **ARCHITECTURAL STATUS: FROZEN REFERENCE PROTOTYPE**  
> Per Project Orchestration Decision LOG-01 and the Canonical Architecture Baseline, **Rust/libp2p is the single production node protocol**.  
> The Node.js implementation in this directory is retained strictly for reference, experimental algorithms, and legacy compatibility verification.  
> **DO NOT** add new production features to this directory.

### Running Legacy Tests
```bash
node test_core.js
```
The test verifies the pure JS FastCDC, Reed-Solomon, AES-256-GCM, and Merkle verification algorithms that were ported and formalized in `core/` (Rust).
