# Release Engineering, SBOM, and Signing Strategy

**Maturity Level:** Production Candidate Standard  

---

## 1. Artifact Integrity & Signing

1. **Binary Signing**:
   - Windows binaries (`.exe`): Authenticode signing with hardware-backed certificate.
   - Linux/macOS binaries: Cosign / Minisign cryptographic signatures (`.sig`).
   - Android APK: APK Signature Scheme v3 / Google Play App Signing.
2. **Software Bill of Materials (SBOM)**:
   - Generated at build time using `cargo-cyclonedx` / `syft`.
   - Attached to release metadata for vulnerability scanning and supply-chain auditing.
3. **Reproducible Builds**:
   - Pinned dependencies via `Cargo.lock`.
   - CI environment containerized to ensure bit-for-bit verifiable builds.

---

## 2. Release Channels & Rollback

- **Channels**: `nightly`, `beta`, `stable`.
- **Automatic Rollback**: If a newly upgraded node crashes repeatedly during initial handshake or corrupts database state, it rolls back to previous executable generation.
