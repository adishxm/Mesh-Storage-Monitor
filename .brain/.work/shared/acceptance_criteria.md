# Acceptance Criteria

## MVP
- Three real devices enroll through a signed invitation.
- A file is encrypted before distribution.
- Shards are distributed across distinct nodes.
- One node can go offline and the original file still downloads.
- Corruption is detected and reconstructed when redundancy allows.
- Node restart reconciles state without changing identity.
- Pause and permanent leave are distinct.
- Dashboard/CLI reports state honestly.

## Production
- Devices work across separate networks.
- Relay and direct paths are encrypted and metered.
- Tenant isolation is tested.
- Device revocation prevents new sessions.
- Proof-of-storage drives repair and credits.
- Android survives documented lifecycle events.
- Releases are signed and rollback-tested.
- Backups, restore drills, SLOs, alerts, and incident runbooks exist.
