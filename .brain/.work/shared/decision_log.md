# Decision Log

| Decision | Choice | Reason |
|---|---|---|
| Canonical node | Rust/libp2p | One secure cross-platform protocol; avoid divergent Node TCP path |
| Android | Native Kotlin + Rust core | Browser cannot reliably remain a storage node |
| Global identity | Device keypair separate from user session | Supports multi-device accounts and revocation |
| Incentive starting point | Reciprocity credits | Avoid token/economic complexity before protocol maturity |
| Metadata | PostgreSQL control plane | Authoritative transactions and tenant isolation |
| Large binary data | Encrypted shard/object storage | Avoid large blobs in relational metadata |
| Temporary offline | Pause/grace period | Avoid unnecessary repair churn |
