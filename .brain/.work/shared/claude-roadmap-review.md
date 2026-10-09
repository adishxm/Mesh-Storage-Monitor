# Claude Roadmap Review

## What Claude's shared conversation confirms

The shared Claude conversation supports the same core direction: encrypted files are chunked, Reed-Solomon protected, distributed across nodes, periodically verified, and reconstructed when some nodes are offline. It also confirms that a temporary shutdown is different from permanent departure, that a browser cannot reliably act as a persistent storage node, and that native Android or Termux is required for a real phone storage peer.

For the global version, Claude adds four important requirements: public bootstrap/relay infrastructure instead of only mDNS, DHT/Kademlia-style discovery, higher redundancy because strangers' devices are less reliable than owned devices, and a reciprocity or contribution-accounting model. Claude also correctly warns that a permissionless worldwide network has a freeloader and Sybil-resistance problem.

## What remains the recommended order

The roadmap should still begin with the three-device proof: upload a file, stop one node, reconstruct the original, restart and reconcile. Then validate Android as a real storage node. Only after that should the project add internet relay/NAT traversal, SaaS tenancy, proof-of-storage, credits, and public-network controls.

## Corrections recorded

- Reed-Solomon is the erasure-coding mechanism; do not describe it as two separate recovery systems.
- A node should rate-limit or temporarily ban attackers, not intentionally crash; crashing creates a denial-of-service weakness.
- The current repository's Rust and Node implementations must not evolve as separate production protocols.
- A global network cannot rely on altruism alone. Start with invitation-only or private-community operation and measured reciprocity credits before considering tokens.
- The shared Claude conversation predates this solo roadmap; it does not directly review the latest roadmap file. This note records the comparison based on the conversation content.
