# Mesh Storage Monitor — Complete Roadmap

## 1. Final product definition

Mesh Storage Monitor will become a **permissioned, privacy-preserving, peer-to-peer storage network**. People install a node on a laptop, desktop, server, tablet, or Android phone. Each node contributes a configurable amount of local storage, normally 1–3% of available capacity. The network stores encrypted user files across multiple independent nodes.

A user uploads a file from any authorized client. The client encrypts the file, divides it into content-addressed chunks, applies erasure coding, and distributes encrypted shards across suitable nodes. During download, the network retrieves any sufficient set of shards, reconstructs missing pieces, verifies the result, decrypts it, and returns the original file.

The network is designed to survive temporary offline nodes and permanent departures. A graceful pause preserves a node's local data for later reconnection. A permanent leave triggers repair and relocation before the node is removed from active storage responsibility.

For a worldwide network, the product must also solve device enrollment, identity, relay connectivity, abuse prevention, proof that nodes are actually storing assigned shards, and reciprocity. A stranger should not be expected to contribute storage, battery, bandwidth, and disk space without receiving a fair storage allowance or another clearly defined benefit.

## 2. What the current repository is today

The repository is a useful algorithm and protocol prototype, not yet a production global network.

The strongest existing foundation is in `node_version/`: file encoding, chunking, encryption, Reed-Solomon recovery, shard placement, audits, repair code, persistent identity, trusted peers, and a local HTTP API. Its core tests pass after installing dependencies.

The Rust workspace contains the intended long-term direction: `core`, `node`, `mesh-cli`, and `terminal-ui`, with libp2p, Noise, Yamux, mDNS, Kademlia, gossipsub, ping, identify, and request-response dependencies. However, the Rust implementation and the Node implementation are not currently one compatible product. `mesh-cli` and `terminal-ui` are placeholders, and the current project does not yet contain a native Android application or a global control plane.

The first repository decision is therefore mandatory:

> **Rust/libp2p becomes the canonical production node. The custom Node.js TCP implementation becomes a reference prototype and is removed from the supported production path.**

Do not continue adding features independently to both implementations.

## 3. Target architecture

```text
 Web dashboard       Native Android app       Desktop CLI
       │                      │                    │
       └──────────── HTTPS / OIDC / API ──────────┘
                              │
                    SaaS control plane
       identity, organizations, devices, invites, credits,
       metadata, placement, repair jobs, audit log, releases
                              │
        ┌─────────────────────┼─────────────────────┐
        │                     │                     │
   PostgreSQL            Queue/event bus        Relay/rendezvous
 authoritative          repair and audits       NAT traversal
 metadata                asynchronous work       peer discovery
        │                     │                     │
        └─────────────────────┼─────────────────────┘
                              │
             Encrypted libp2p peer-to-peer data plane
                              │
     Linux/macOS/Windows nodes — Android nodes — servers
                              │
          encrypted shard storage and proof-of-storage
```

The control plane manages users, organizations, devices, policies, placement metadata, credits, repair work, and operations. It should not receive plaintext files or user passphrases. The data plane transports encrypted shards between authenticated nodes.

The system should support private communities first and a public network later. A public network cannot safely be launched by adding a public port to the current prototype.

## 4. Core design rules

### Identity and trust

Every node creates a persistent Ed25519/libp2p identity on first start. The private key remains on the device. User accounts and device identities are separate. A user may have multiple devices, and a device can be revoked without deleting the user's account.

A joining device receives a short-lived, single-use, signed invitation. It generates its own keypair, proves possession of the private key, and receives a signed acceptance. Peer-to-peer sessions use Noise or TLS through libp2p. The project must not invent a new cryptographic handshake when libp2p already provides the transport primitives.

### Confidentiality

Encryption occurs before shard data leaves the uploading client. Storage nodes hold ciphertext, content hashes, and placement metadata only. The production file format must be versioned and authenticated. Cryptographic choices require an independent review before a public launch; using AES-GCM or Argon2id in code is not the same as having a reviewed cryptographic design.

### Integrity

Every chunk and shard receives a content address. Manifests are authenticated and versioned. Reads verify shard hashes, reconstructed chunk hashes, and the file root before decryption is accepted. Periodic proof-of-storage audits detect silent deletion or corruption before a user requests the file.

### Availability

Reed-Solomon parameters are expressed as `k` data shards and `n` total shards, where `m = n - k` parity shards. Any `k` valid shards reconstruct the chunk. Placement must account for failure domains, not merely distinct process IDs. Two shards of one chunk must not be placed on the same physical machine, household, region, or network when the system claims those as independent failure domains.

For a small prototype, use `k=2, n=3` across three nodes. For unreliable global nodes, prefer larger redundancy such as `k=6, n=10` or another measured configuration. The exact choice must be based on observed node availability and bandwidth, not a fixed number in documentation.

### Offline, pause, and permanent leave

A node that is temporarily offline remains a trusted member. The network waits through a grace period before repairing its shards. A node performing a clean pause persists its routing table, identity, manifests, and held shards.

A permanent leave is explicit. It creates repair jobs for all held shards, waits for the redundancy floor to be restored, then revokes the node's active storage assignment. Account deletion and device removal must use a 30-day retention policy only where it is legally and operationally appropriate. Retention must not replace repair.

### Reciprocity

For a worldwide network, define a measurable contribution model. The initial recommendation is non-cryptocurrency reciprocity credits:

```text
verified storage contribution + verified uptime + useful bandwidth
                    → storage allowance and service priority
```

Credits should be backed by proof-of-storage and bounded by policy. Do not launch a token economy until the storage protocol, abuse model, and legal review are complete.

## 5. Repository target layout

Refactor the repository toward this structure:

```text
/core/                    Pure data and cryptographic primitives
/node/                    Canonical Rust node daemon
/protocol/                Versioned wire messages and schemas
/mesh-cli/                Enrollment, status, invite, leave, repair commands
/terminal-ui/             Terminal monitoring client
/control-plane/           SaaS API and worker services
/android/                 Kotlin/Compose Android node application
/web-dashboard/           Multi-tenant web dashboard
/relay/                   Relay/rendezvous deployment and configuration
/infra/                   Local, staging, and production infrastructure
/docs/                    Architecture, operations, security, and research
/tests/integration/       Multi-node and failure tests
```

The current top-level files should be migrated as follows:

- Move the v2 architecture specification to `docs/ARCHITECTURE.md`.
- Move operational instructions to `docs/OPERATIONS.md`.
- Add `docs/THREAT_MODEL.md`.
- Add `docs/PROTOCOL.md`.
- Add `docs/ANDROID.md`.
- Add `docs/INCENTIVES.md`.
- Add `docs/RELEASES.md`.
- Rename the current Node implementation documentation as prototype/legacy documentation.
- Keep the root `README.md` short and link to the detailed documents.

## 6. Phase roadmap

### Phase 0 — Freeze the vision and baseline the repository

**Goal:** make the current state reproducible and prevent architecture drift.

Tasks:

1. Create a baseline branch/tag for the current prototype.
2. Choose Rust/libp2p as the canonical node.
3. Mark `node_version/` as legacy and stop adding production features there.
4. Install Rust in CI and run `cargo fmt`, `cargo clippy`, and `cargo test --workspace`.
5. Keep the existing Node core tests temporarily as compatibility tests.
6. Add CI for dependency scanning, secret scanning, unit tests, and basic integration tests.
7. Write a threat model before expanding network exposure.
8. Record supported platforms and the minimum Rust, Android, and operating-system versions.

**Done when:** a clean checkout builds in CI, tests run automatically, the canonical implementation is explicit, and a new contributor can understand the project layout from the README.

### Phase 1 — Finish and verify the pure storage core

**Goal:** make file correctness independent of networking.

Implement and test:

- streaming file input and output instead of loading full files into memory;
- versioned chunk and shard headers;
- content-defined chunking with bounded minimum, target, and maximum sizes;
- content-addressed chunk and shard identifiers;
- Reed-Solomon encode/reconstruct/verify operations;
- authenticated encryption and key derivation;
- manifest creation, signing, versioning, and validation;
- Merkle or equivalent root verification;
- deduplication rules;
- quota accounting;
- partial upload recovery;
- corrupt shard rejection;
- deterministic repair output where required by the manifest format.

Test with small files, empty files, large files, random files, duplicate files, modified files, corrupted shards, missing shards, truncated streams, wrong keys, and insufficient shard counts.

**Done when:** a 1 GB test file can be processed with bounded memory, a complete round trip is byte-identical, corruption is detected, and every format has a version and migration strategy.

### Phase 2 — Build the canonical local node

**Goal:** run one reliable node on Linux, macOS, and Windows.

Implement:

- persistent device identity;
- configurable data directory;
- storage quota based on percentage or explicit bytes;
- atomic shard writes using temporary files and rename;
- local manifest database rather than uncontrolled JSON files;
- local API under `/api/v1`;
- status, peers, shards, files, quota, health, audit, and repair endpoints;
- structured logs and metrics;
- graceful shutdown;
- separate `pause`, `leave`, and `revoke` commands;
- local authentication for management APIs;
- safe file-name and path validation.

The node should never expose its management API publicly without explicit configuration. CORS should be restricted, and secrets must not appear in query strings or logs.

**Done when:** one node survives restart without losing identity or metadata, can store and retrieve an encrypted test file, enforces quota atomically, and produces useful health information.

### Phase 3 — Implement multi-node LAN operation

**Goal:** prove the original concept with three or more real devices.

Implement:

- signed invitation generation;
- QR payload generation;
- one-time pairing and reciprocal trust;
- LAN discovery using mDNS where appropriate;
- authenticated libp2p peer sessions;
- shard placement across different nodes;
- per-node contribution limits;
- upload and download streaming;
- failure detection and repair state machine;
- local dashboard and terminal status view.

Use a repeatable test topology:

```text
Node A: laptop
Node B: second laptop or desktop
Node C: Android phone or tablet
```

Start with `k=1,n=2` for a two-node mirror test. Move to `k=2,n=3` only when three independent nodes are available.

**Done when:** the user can upload a file, stop one node, download the original file, restart the node, reconcile state, and observe every event in the dashboard and terminal.

### Phase 4 — Build the native Android node

**Goal:** make Android a real storage peer rather than a browser dashboard.

Create a Kotlin/Jetpack Compose app with:

- QR and universal-link onboarding;
- device name and contribution controls;
- Android Keystore-backed identity protection;
- native Rust core integration only after the Rust protocol stabilizes;
- Room database for local metadata and job state;
- app-specific storage by default;
- Storage Access Framework for user-selected locations;
- foreground service for active node operation;
- visible notification while the node is active;
- pause, resume, leave, and revoke states;
- Wi-Fi/mobile-data policy controls;
- battery and data-usage limits;
- restart and process-death recovery;
- secure update handling.

Android 14 and later require the appropriate foreground-service type and permission declarations. Storage should follow scoped-storage rules and should not assume unrestricted access to the device filesystem. [1] [2]

**Done when:** a phone scans an invitation, creates its own identity, joins the network, contributes a user-approved quota, stores real encrypted shards, survives app restarts, and reports honest background limitations.

### Phase 5 — Add internet connectivity

**Goal:** connect nodes on different networks.

Deploy dedicated bootstrap, rendezvous, and relay services. Add:

- AutoNAT;
- circuit relay;
- hole punching where available;
- QUIC and TCP fallback;
- regional relay selection;
- relay bandwidth quotas;
- connection timeouts and backpressure;
- encrypted sessions end-to-end;
- abuse controls to prevent open-proxy behavior;
- peer address caching and rotation;
- connectivity diagnostics in the dashboard.

libp2p supports multiple transports, encrypted connections, AutoNAT, relays, and hole punching, which should be used instead of a custom global transport layer. [3]

**Done when:** three devices on separate home/mobile networks can join, exchange encrypted shards, and recover a file through direct or relayed connections.

### Phase 6 — Build the SaaS control plane

**Goal:** operate many users and organizations safely.

Create the control plane with versioned services for:

- account and session management;
- organizations and roles;
- device registration;
- invitation issuance and one-time nonce consumption;
- device certificates and revocation;
- file and manifest metadata;
- shard placement metadata;
- repair jobs;
- audit events;
- usage and storage contribution accounting;
- reciprocity credits;
- notifications;
- release channels and device versions.

Use PostgreSQL as the authoritative metadata store. Use a durable queue for repair, audit, notification, and accounting jobs. Use object storage for encrypted backups and release artifacts. Redis may handle ephemeral locks and rate limits, but it must not be the source of truth.

Enforce tenant context in API middleware, database access, queue messages, object-store paths, device enrollment, and audit events. Never trust an organization ID supplied only by a browser form.

**Done when:** two organizations cannot read each other's metadata, a device can be enrolled and revoked, all important actions are auditable, and workers can retry jobs idempotently.

### Phase 7 — Proof-of-storage and repair reliability

**Goal:** verify that nodes really retain their assigned shards.

Implement periodic challenges using random nonces and authenticated responses. A challenge should prove possession without transferring the entire shard each time. Store audit results and use them in placement and reciprocity accounting.

Build a repair state machine with:

- job IDs;
- leases;
- idempotency keys;
- retry limits;
- dead-letter handling;
- source-shard verification;
- destination confirmation;
- manifest update transactions;
- repair cancellation;
- re-repair after concurrent failures;
- failure-domain-aware placement;
- operator visibility.

**Done when:** silent deletion or corruption is detected, a healthy replacement shard is created, duplicate repair jobs do not corrupt metadata, and the redundancy target is restored after a node leaves.

### Phase 8 — Reciprocity and abuse resistance

**Goal:** make a worldwide network economically and operationally viable without immediately introducing cryptocurrency.

Define a credit ledger based on verified contribution. Track:

- storage reserved and actually used;
- uptime and availability;
- successful proof-of-storage results;
- useful upload/download bandwidth;
- failed or fraudulent audits;
- relay consumption;
- device age and reputation.

Add defenses against:

- Sybil devices;
- fake capacity claims;
- colluding nodes;
- duplicate shard placement;
- selective withholding;
- relay abuse;
- invitation farming;
- denial-of-service;
- malicious file or metadata uploads;
- account takeover.

Start with invitation-only communities and explicit limits. A public permissionless mode should be disabled until identity, abuse, and accounting controls have been tested.

**Done when:** a new user can understand what they contribute, what storage they receive, how reputation changes, and what happens when they stop contributing.

### Phase 9 — Web dashboard and terminal product

**Goal:** expose the system clearly to normal users and operators.

The web dashboard should include:

- organization switcher;
- devices and health;
- storage contribution and quota;
- files and upload/download status;
- shard redundancy;
- repair history;
- audit history;
- invitations;
- device revocation;
- credits and usage;
- security settings;
- incident notifications.

The terminal client should include:

```text
mesh-cli login
mesh-cli invite
mesh-cli join
mesh-cli status
mesh-cli peers
mesh-cli files
mesh-cli audits
mesh-cli repairs
mesh-cli pause
mesh-cli leave
mesh-cli revoke
mesh-cli update
```

The dashboard must never claim that the whole network is healthy based on one peer's partial view. Show the scope and freshness of every health measurement.

**Done when:** a user can complete enrollment, upload/download a file, see redundancy, understand an offline node, and revoke a device without reading a protocol document.

### Phase 10 — Production hardening

Before public beta, complete:

- threat modeling;
- external security review;
- penetration testing;
- dependency and container scanning;
- fuzzing of parsers and protocol messages;
- signed release artifacts;
- SBOM generation;
- reproducible or attestable builds;
- key rotation and revocation drills;
- backup and restore drills;
- regional outage tests;
- data deletion and export tests;
- privacy and retention review;
- incident response exercises;
- rate-limit and abuse tests;
- Android battery and connectivity matrix tests;
- staged rollout and rollback tests.

If Kubernetes is used, start with a managed control plane where possible. Production guidance emphasizes high availability, replicated control planes, load balancing, backups, RBAC, resource limits, and multi-zone planning. [4] Secrets must be encrypted at rest and protected with least-privilege access; base64 encoding alone is not encryption. [5]

Use OpenTelemetry-compatible telemetry for traces, metrics, and logs across the control plane, workers, relay services, and node agents. [6]

**Done when:** the system has measurable SLOs, monitored failure modes, tested backups, documented recovery steps, and a release process that can stop or roll back a bad update.

## 7. Suggested production milestones

### Milestone A — Local proof

Three local nodes, one file, encrypted shards, one node offline, successful reconstruction, visible status.

### Milestone B — Android proof

One laptop and one Android phone on the same network, real shard storage on the phone, phone survives laptop shutdown, file is recovered from the phone.

### Milestone C — Internet proof

Nodes on different networks connect through a relay or direct hole punch, upload and recovery work, and relay bandwidth is measured.

### Milestone D — Private community beta

Ten to fifty invited users, multiple organizations, device revocation, proof-of-storage, credits, repair jobs, support logs, and documented limits.

### Milestone E — Public beta

Signed releases, native Android distribution, abuse controls, monitored infrastructure, backups, security review, staged rollout, and clear privacy/retention policies.

### Milestone F — Global scale

Multiple relay regions, failure-domain-aware placement, horizontally scaled workers, stronger Sybil resistance, multi-region metadata strategy, and formal operational ownership.

## 8. Exact first pull requests to make in the repository

### Pull request 1 — Build and ownership baseline

Add CI, Rust toolchain configuration, formatting and lint checks, a clear canonical-node decision, and a migration note for `node_version/`.

### Pull request 2 — Versioned protocol package

Create `/protocol` with versioned serde message types, request IDs, tenant IDs where applicable, capability negotiation, error codes, and compatibility tests.

### Pull request 3 — Streaming and storage correctness

Move file operations to streaming interfaces, add atomic writes, versioned shard headers, quota tests, corruption tests, and recovery tests.

### Pull request 4 — Signed invitations

Implement invitation payloads, signatures, expiry, single-use nonces, QR output, and prepare/complete join exchange.

### Pull request 5 — Real CLI

Implement `mesh-cli invite`, `join`, `status`, `peers`, `pause`, `leave`, and `revoke`. Ensure every command has machine-readable JSON output.

### Pull request 6 — Three-node integration tests

Automate three local processes, upload, kill, recover, restart, reconcile, pause, and leave scenarios.

### Pull request 7 — Android shell

Create the Kotlin app, QR deep link, device identity setup, node status screen, permissions, and foreground service skeleton. Do not claim storage participation until the native node integration passes.

### Pull request 8 — Relay connectivity

Add relay/rendezvous configuration, direct-versus-relayed connection reporting, timeouts, and bandwidth limits.

### Pull request 9 — Control-plane vertical slice

Implement organization, device enrollment, invitation issuance, revocation, PostgreSQL migrations, audit events, and one node registration path.

### Pull request 10 — Proof-of-storage and repair worker

Implement challenges, audit results, repair jobs, idempotency, and dashboards for redundancy health.

## 9. Recommended technology choices

Use Rust for the canonical node and storage core because it can serve desktop, server, and Android through a shared core while providing predictable resource use. Use Kotlin and Jetpack Compose for the Android product shell. Use PostgreSQL for authoritative SaaS metadata. Use a durable queue for asynchronous work. Use S3-compatible object storage for encrypted backups and release artifacts. Use a modern web frontend for the dashboard, but keep all security decisions in the backend.

Python can be used for experiments, test harnesses, data analysis, and hackathon prototypes. It should not become a second production node protocol. C, Java, and MERN can be useful in isolated components, but adding languages without a clear boundary will increase operational complexity.

## 10. Performance and scale targets

Treat these as initial targets to measure, not guaranteed claims:

- 1,000 registered devices;
- 100 concurrently active organizations;
- 10,000 files per organization;
- horizontal API and worker scaling;
- p95 metadata reads below 250 ms;
- p95 metadata writes below 500 ms;
- join completion within 60 seconds for healthy devices;
- failure detection within two minutes;
- metadata recovery point objective of five minutes or better;
- repair acknowledgement within 30 seconds;
- documented throughput limits for Android and relayed traffic.

Scale testing should vary node count, file size, shard count, node churn, relay usage, slow devices, partitions, duplicate requests, and simultaneous repairs.

## 11. Rules for claiming completion

Do not call the system production-ready because one file worked on two local processes. Use these labels:

- **Algorithm prototype:** core encode/decode tests pass.
- **Local network prototype:** three real devices exchange and recover files.
- **Internet prototype:** nodes on separate networks connect through relay/direct paths.
- **Private beta:** invited users, revocation, audits, repair, backups, and monitoring work.
- **Production candidate:** security review, release signing, recovery drills, and SLO evidence exist.
- **Production:** operational ownership, incident response, privacy/legal readiness, and tested upgrade/rollback are in place.

## 12. Final build order

The safest complete order is:

1. Freeze the architecture and choose Rust/libp2p.
2. Make the storage core streaming, versioned, and independently testable.
3. Make one Rust node reliable.
4. Make three real nodes work on one LAN.
5. Prove Android can be a real storage node.
6. Add global connectivity with relay and NAT traversal.
7. Add signed enrollment and device revocation.
8. Add the SaaS control plane and tenant isolation.
9. Add proof-of-storage and repair workers.
10. Add reciprocity credits and abuse resistance.
11. Add the full web dashboard and terminal client.
12. Harden, audit, monitor, back up, and release gradually.

Do not begin with billing, tokens, a large dashboard, or a global public launch. The strongest proof of your vision is still the simple test: upload a file, take a storage node offline, and recover the exact original file from the remaining nodes. Build that proof first, then make every later layer preserve it.

## References

[1]: https://developer.android.com/develop/background-work/services/fgs/service-types "Android foreground service types"
[2]: https://developer.android.com/training/data-storage "Android data and file storage overview"
[3]: https://libp2p.io/ "libp2p modular peer-to-peer networking"
[4]: https://kubernetes.io/docs/setup/production-environment/ "Kubernetes production environment guidance"
[5]: https://kubernetes.io/docs/concepts/security/secrets-good-practices/ "Kubernetes Secrets good practices"
[6]: https://opentelemetry.io/docs/platforms/kubernetes/ "OpenTelemetry with Kubernetes"
