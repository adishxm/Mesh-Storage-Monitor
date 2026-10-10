use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum SybilError {
    #[error("Subnet {subnet} exceeded maximum device density limit of {max_allowed}")]
    SubnetDensityExceeded { subnet: String, max_allowed: usize },

    #[error(
        "Capacity claim of {claimed_bytes} bytes rejected: requires at least {required_audits} passed proof-of-storage audits (got {actual_audits})"
    )]
    UnverifiedCapacityClaim {
        claimed_bytes: u64,
        required_audits: u64,
        actual_audits: u64,
    },

    #[error("Peer {peer_id} credit balance {balance} is below threshold: throttled or clamped")]
    InsufficientCredits { peer_id: String, balance: i64 },
}

/// Service tier based on reciprocity contribution.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReciprocityTier {
    /// Peer contributes healthy storage & passes audits
    Contributor,
    /// New peer within grace allowance period
    Probationary,
    /// Overdrawn consumption without contribution; bandwidth throttled
    Throttled,
    /// Abusive free-rider; uploads/downloads blocked
    Suspended,
}

/// Non-cryptocurrency credit ledger tracking verified storage and network contribution.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CreditLedger {
    pub peer_id: String,
    pub bytes_contributed: u64,
    pub bytes_consumed: u64,
    pub bandwidth_uploaded: u64,
    pub bandwidth_downloaded: u64,
    pub audits_passed: u64,
    pub audits_failed: u64,
    pub uptime_seconds: u64,
    pub created_at: u64,
}

impl CreditLedger {
    pub fn new(peer_id: String, created_at: u64) -> Self {
        Self {
            peer_id,
            bytes_contributed: 0,
            bytes_consumed: 0,
            bandwidth_uploaded: 0,
            bandwidth_downloaded: 0,
            audits_passed: 0,
            audits_failed: 0,
            uptime_seconds: 0,
            created_at,
        }
    }

    /// Records storage space contributed by storing remote shards.
    pub fn record_storage_contribution(&mut self, bytes: u64) {
        self.bytes_contributed = self.bytes_contributed.saturating_add(bytes);
    }

    /// Records storage consumed by storing local files on the network.
    pub fn record_storage_consumption(&mut self, bytes: u64) {
        self.bytes_consumed = self.bytes_consumed.saturating_add(bytes);
    }

    /// Records useful upload and download bandwidth.
    pub fn record_bandwidth(&mut self, uploaded: u64, downloaded: u64) {
        self.bandwidth_uploaded = self.bandwidth_uploaded.saturating_add(uploaded);
        self.bandwidth_downloaded = self.bandwidth_downloaded.saturating_add(downloaded);
    }

    /// Records proof-of-storage challenge outcomes.
    pub fn record_audit(&mut self, passed: bool) {
        if passed {
            self.audits_passed = self.audits_passed.saturating_add(1);
        } else {
            self.audits_failed = self.audits_failed.saturating_add(1);
        }
    }

    /// Records uptime progression.
    pub fn record_uptime(&mut self, seconds: u64) {
        self.uptime_seconds = self.uptime_seconds.saturating_add(seconds);
    }

    /// Computes reputation factor based on audit success rate and uptime.
    /// Range is [0.1, 1.5].
    pub fn reputation_multiplier(&self) -> f64 {
        let total_audits = self.audits_passed + self.audits_failed;
        let audit_ratio = if total_audits == 0 {
            0.8 // Neutral baseline for untested peers
        } else {
            (self.audits_passed as f64 + 1.0) / (total_audits as f64 + 2.0)
        };

        // Uptime bonus up to +0.2 for nodes active > 7 days (604,800s)
        let uptime_bonus = (self.uptime_seconds as f64 / 604_800.0).clamp(0.0, 0.2);

        (audit_ratio + uptime_bonus).clamp(0.1, 1.5)
    }

    /// Calculates earned storage allowance based on verified contribution.
    /// Formula: allowance = base_free_bytes + (contributed * reciprocity_ratio * reputation)
    pub fn earned_allowance_bytes(
        &self,
        base_free_bytes: u64,
        reciprocity_ratio: f64,
        max_quota_cap: u64,
    ) -> u64 {
        let reputation = self.reputation_multiplier();
        let earned_bonus = (self.bytes_contributed as f64 * reciprocity_ratio * reputation) as u64;
        let total = base_free_bytes.saturating_add(earned_bonus);
        total.min(max_quota_cap)
    }

    /// Calculates net credit balance in bytes. Positive means surplus; negative means deficit.
    pub fn credit_balance(
        &self,
        base_free_bytes: u64,
        reciprocity_ratio: f64,
        max_quota_cap: u64,
    ) -> i64 {
        let allowance =
            self.earned_allowance_bytes(base_free_bytes, reciprocity_ratio, max_quota_cap);
        allowance as i64 - self.bytes_consumed as i64
    }

    /// Calculates the fair-share contribution ratio: contributed / consumed.
    pub fn fair_share_ratio(&self) -> f64 {
        if self.bytes_consumed == 0 {
            if self.bytes_contributed > 0 { 2.0 } else { 1.0 }
        } else {
            self.bytes_contributed as f64 / self.bytes_consumed as f64
        }
    }

    /// Evaluates current reciprocity service tier.
    pub fn evaluate_tier(
        &self,
        base_free_bytes: u64,
        reciprocity_ratio: f64,
        max_quota_cap: u64,
    ) -> ReciprocityTier {
        let balance = self.credit_balance(base_free_bytes, reciprocity_ratio, max_quota_cap);
        if balance >= 0 {
            if self.bytes_contributed > 0 || self.audits_passed > 0 {
                ReciprocityTier::Contributor
            } else {
                ReciprocityTier::Probationary
            }
        } else {
            // Deficit exceeds 2x allowance or consecutive audit failures
            let deficit = (-balance) as u64;
            if deficit > base_free_bytes * 2 || self.audits_failed > 3 {
                ReciprocityTier::Suspended
            } else {
                ReciprocityTier::Throttled
            }
        }
    }
}

/// Tracks IP subnet / network clustering to defend against Sybil device proliferation.
#[derive(Debug, Clone)]
pub struct SubnetDensityGuard {
    pub max_devices_per_subnet: usize,
    /// Maps subnet identifier (e.g., "192.168.1.0/24") to set of registered peer IDs
    pub subnet_allocations: HashMap<String, Vec<String>>,
}

impl SubnetDensityGuard {
    pub fn new(max_devices_per_subnet: usize) -> Self {
        Self {
            max_devices_per_subnet,
            subnet_allocations: HashMap::new(),
        }
    }

    /// Validates whether a new peer can register from the specified subnet.
    pub fn register_peer(&mut self, subnet: &str, peer_id: &str) -> Result<(), SybilError> {
        let devices = self
            .subnet_allocations
            .entry(subnet.to_string())
            .or_default();
        if devices.iter().any(|p| p == peer_id) {
            return Ok(()); // Already registered peer
        }

        if devices.len() >= self.max_devices_per_subnet {
            return Err(SybilError::SubnetDensityExceeded {
                subnet: subnet.to_string(),
                max_allowed: self.max_devices_per_subnet,
            });
        }

        devices.push(peer_id.to_string());
        Ok(())
    }

    /// Deregisters a peer from a subnet when leaving.
    pub fn deregister_peer(&mut self, subnet: &str, peer_id: &str) {
        if let Some(devices) = self.subnet_allocations.get_mut(subnet) {
            devices.retain(|p| p != peer_id);
        }
    }

    /// Returns device count currently registered under a subnet.
    pub fn subnet_count(&self, subnet: &str) -> usize {
        self.subnet_allocations
            .get(subnet)
            .map(|v| v.len())
            .unwrap_or(0)
    }
}

/// Prevents fake or inflated capacity claims by unlocking credit allowances only
/// as storage is proven through proof-of-storage challenges.
#[derive(Debug, Clone)]
pub struct CapacityVerificationGuard {
    pub required_audits_per_gb: u64,
}

impl CapacityVerificationGuard {
    pub fn new(required_audits_per_gb: u64) -> Self {
        Self {
            required_audits_per_gb: required_audits_per_gb.max(1),
        }
    }

    /// Verifies if a peer's claimed storage contribution is legitimate based on audit evidence.
    pub fn verify_capacity_claim(
        &self,
        claimed_bytes: u64,
        audits_passed: u64,
    ) -> Result<u64, SybilError> {
        let claimed_gb = claimed_bytes.div_ceil(1_073_741_824); // 1 GB = 2^30 bytes
        let required_audits = (claimed_gb * self.required_audits_per_gb).max(1);

        if audits_passed < required_audits && claimed_bytes > 10_485_760 {
            // Allow 10MB probationary grace
            return Err(SybilError::UnverifiedCapacityClaim {
                claimed_bytes,
                required_audits,
                actual_audits: audits_passed,
            });
        }

        Ok(claimed_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credit_ledger_allowance_accrual() {
        let mut ledger = CreditLedger::new("peer-100".into(), 1000);
        let base_free = 1_000_000_000; // 1 GB free
        let reciprocity_ratio = 1.0; // 1:1 ratio
        let cap = 50_000_000_000; // 50 GB cap

        // Initial state: probationary tier with baseline allowance
        assert_eq!(
            ledger.evaluate_tier(base_free, reciprocity_ratio, cap),
            ReciprocityTier::Probationary
        );
        assert_eq!(
            ledger.earned_allowance_bytes(base_free, reciprocity_ratio, cap),
            base_free
        );

        // Contribute 10 GB and pass 10 audits
        ledger.record_storage_contribution(10_000_000_000);
        for _ in 0..10 {
            ledger.record_audit(true);
        }
        ledger.record_uptime(86400 * 3); // 3 days

        let allowance = ledger.earned_allowance_bytes(base_free, reciprocity_ratio, cap);
        assert!(
            allowance > 9_000_000_000,
            "Allowance must scale with contributed storage"
        );
        assert_eq!(
            ledger.evaluate_tier(base_free, reciprocity_ratio, cap),
            ReciprocityTier::Contributor
        );

        // Consume 5 GB
        ledger.record_storage_consumption(5_000_000_000);
        assert!(ledger.credit_balance(base_free, reciprocity_ratio, cap) > 0);
        assert!(ledger.fair_share_ratio() >= 2.0);
    }

    #[test]
    fn test_free_rider_deficit_triggers_throttling_and_suspension() {
        let mut ledger = CreditLedger::new("freerider-peer".into(), 1000);
        let base_free = 100_000_000; // 100 MB free
        let reciprocity_ratio = 1.0;
        let cap = 1_000_000_000;

        // Consume 150 MB without contributing anything (exceeds base allowance by 50MB)
        ledger.record_storage_consumption(150_000_000);
        assert_eq!(
            ledger.evaluate_tier(base_free, reciprocity_ratio, cap),
            ReciprocityTier::Throttled
        );

        // Consume 500 MB (massive deficit > 2x free allowance) -> Suspended
        ledger.record_storage_consumption(350_000_000);
        assert_eq!(
            ledger.evaluate_tier(base_free, reciprocity_ratio, cap),
            ReciprocityTier::Suspended
        );
    }

    #[test]
    fn test_subnet_density_guard_blocks_sybil_clustering() {
        let mut guard = SubnetDensityGuard::new(2); // Max 2 devices per subnet

        let subnet = "192.168.1.0/24";
        assert!(guard.register_peer(subnet, "peer-1").is_ok());
        assert!(guard.register_peer(subnet, "peer-2").is_ok());
        assert_eq!(guard.subnet_count(subnet), 2);

        // Idempotent re-registration
        assert!(guard.register_peer(subnet, "peer-1").is_ok());

        // 3rd device on same subnet rejected!
        let err = guard.register_peer(subnet, "peer-3").unwrap_err();
        assert_eq!(
            err,
            SybilError::SubnetDensityExceeded {
                subnet: subnet.into(),
                max_allowed: 2
            }
        );

        // Deregister peer-2 and now peer-3 succeeds
        guard.deregister_peer(subnet, "peer-2");
        assert_eq!(guard.subnet_count(subnet), 1);
        assert!(guard.register_peer(subnet, "peer-3").is_ok());
    }

    #[test]
    fn test_capacity_verification_guard() {
        let guard = CapacityVerificationGuard::new(5); // 5 audits per GB required

        // Claiming 10 GB requires 50 passed audits
        let claimed_10gb = 10 * 1_073_741_824;
        let err = guard.verify_capacity_claim(claimed_10gb, 10).unwrap_err();
        assert_eq!(
            err,
            SybilError::UnverifiedCapacityClaim {
                claimed_bytes: claimed_10gb,
                required_audits: 50,
                actual_audits: 10
            }
        );

        // With 50 passed audits, claim is verified!
        let verified = guard.verify_capacity_claim(claimed_10gb, 50).unwrap();
        assert_eq!(verified, claimed_10gb);
    }
}
