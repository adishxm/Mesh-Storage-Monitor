use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum QuotaError {
    #[error(
        "Storage quota exceeded: limit is {limit_bytes} bytes, current usage is {current_bytes} bytes, cannot add {additional_bytes} bytes"
    )]
    Exceeded {
        limit_bytes: u64,
        current_bytes: u64,
        additional_bytes: u64,
    },

    #[error("Invalid percentage: must be between 0.0 and 100.0, got {0}")]
    InvalidPercentage(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum QuotaPolicy {
    Percentage(f64),
    AbsoluteBytes(u64),
}

impl Default for QuotaPolicy {
    fn default() -> Self {
        QuotaPolicy::Percentage(2.0) // 2% default storage contribution
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct QuotaTracker {
    pub total_disk_bytes: u64,
    pub quota_bytes: u64,
    pub used_bytes: u64,
}

impl QuotaTracker {
    pub fn from_policy(total_disk_bytes: u64, policy: &QuotaPolicy) -> Result<Self, QuotaError> {
        let quota_bytes = match policy {
            QuotaPolicy::AbsoluteBytes(bytes) => *bytes,
            QuotaPolicy::Percentage(pct) => {
                if *pct <= 0.0 || *pct > 100.0 {
                    return Err(QuotaError::InvalidPercentage(format!("{pct}%")));
                }
                ((total_disk_bytes as f64) * (pct / 100.0)).round() as u64
            }
        };

        Ok(Self {
            total_disk_bytes,
            quota_bytes,
            used_bytes: 0,
        })
    }

    pub fn new(total_disk_bytes: u64, quota_bytes: u64) -> Self {
        Self {
            total_disk_bytes,
            quota_bytes,
            used_bytes: 0,
        }
    }

    pub fn can_store(&self, additional_bytes: u64) -> bool {
        self.used_bytes.saturating_add(additional_bytes) <= self.quota_bytes
    }

    pub fn record_store(&mut self, additional_bytes: u64) -> Result<(), QuotaError> {
        if !self.can_store(additional_bytes) {
            return Err(QuotaError::Exceeded {
                limit_bytes: self.quota_bytes,
                current_bytes: self.used_bytes,
                additional_bytes,
            });
        }
        self.used_bytes += additional_bytes;
        Ok(())
    }

    pub fn record_delete(&mut self, freed_bytes: u64) {
        self.used_bytes = self.used_bytes.saturating_sub(freed_bytes);
    }

    pub fn remaining_bytes(&self) -> u64 {
        self.quota_bytes.saturating_sub(self.used_bytes)
    }

    pub fn usage_ratio(&self) -> f64 {
        if self.quota_bytes == 0 {
            0.0
        } else {
            (self.used_bytes as f64) / (self.quota_bytes as f64)
        }
    }
}

/// Power source constraint policy for mobile Android nodes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum AndroidPowerPolicy {
    OnlyCharging,
    AnyPower,
}

/// Network interface constraint policy for mobile Android nodes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum AndroidNetworkPolicy {
    UnmeteredWifiOnly,
    AnyNetwork,
}

/// Live hardware telemetry reported by the Android OS layer.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AndroidDeviceState {
    pub total_device_storage_bytes: u64,
    pub free_device_storage_bytes: u64,
    pub is_charging: bool,
    pub is_unmetered_wifi: bool,
    pub requested_contribution_pct: f64,
}

/// Decision output instructing the Android Foreground Service whether to run and with what quota.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AndroidPolicyDecision {
    pub allowed_to_operate: bool,
    pub effective_quota_bytes: u64,
    pub reason: String,
}

/// Evaluates Android runtime constraints:
/// 1. Only runs when charging if power policy is OnlyCharging.
/// 2. Only runs on unmetered Wi-Fi if network policy is UnmeteredWifiOnly.
/// 3. Clamps contribution percentage strictly within 1.0% to 3.0%.
/// 4. Enforces a 10% device storage floor: if free host storage < 10%, pauses node to avoid filling user device.
pub fn evaluate_android_policy(
    state: &AndroidDeviceState,
    power_policy: &AndroidPowerPolicy,
    net_policy: &AndroidNetworkPolicy,
) -> AndroidPolicyDecision {
    if *power_policy == AndroidPowerPolicy::OnlyCharging && !state.is_charging {
        return AndroidPolicyDecision {
            allowed_to_operate: false,
            effective_quota_bytes: 0,
            reason: "Node paused: Device is not charging".to_string(),
        };
    }

    if *net_policy == AndroidNetworkPolicy::UnmeteredWifiOnly && !state.is_unmetered_wifi {
        return AndroidPolicyDecision {
            allowed_to_operate: false,
            effective_quota_bytes: 0,
            reason: "Node paused: Device not on unmetered Wi-Fi".to_string(),
        };
    }

    let min_free_headroom = (state.total_device_storage_bytes as f64 * 0.10) as u64;
    if state.free_device_storage_bytes < min_free_headroom {
        return AndroidPolicyDecision {
            allowed_to_operate: false,
            effective_quota_bytes: 0,
            reason: format!(
                "Node paused: Host storage critically low (free: {} B, floor required: {} B)",
                state.free_device_storage_bytes, min_free_headroom
            ),
        };
    }

    let clamped_pct = state.requested_contribution_pct.clamp(1.0, 3.0);
    let target_quota =
        ((state.total_device_storage_bytes as f64) * (clamped_pct / 100.0)).round() as u64;

    let safe_quota = target_quota.min(state.free_device_storage_bytes / 2);

    AndroidPolicyDecision {
        allowed_to_operate: true,
        effective_quota_bytes: safe_quota,
        reason: format!(
            "Operational: contributing {:.1}% storage ({} bytes)",
            clamped_pct, safe_quota
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentage_quota() {
        // 1 TB disk, 2% contribution = ~20 GB
        let one_tb = 1_000_000_000_000u64;
        let tracker = QuotaTracker::from_policy(one_tb, &QuotaPolicy::Percentage(2.0)).unwrap();
        assert_eq!(tracker.quota_bytes, 20_000_000_000u64);
        assert_eq!(tracker.used_bytes, 0);
        assert_eq!(tracker.remaining_bytes(), 20_000_000_000u64);
    }

    #[test]
    fn test_invalid_percentage() {
        let err = QuotaTracker::from_policy(1000, &QuotaPolicy::Percentage(105.0)).unwrap_err();
        assert!(matches!(err, QuotaError::InvalidPercentage(_)));
    }

    #[test]
    fn test_quota_store_and_overflow() {
        let mut tracker = QuotaTracker::new(10_000, 1_000);
        assert!(tracker.can_store(500));
        tracker.record_store(500).unwrap();
        assert_eq!(tracker.used_bytes, 500);
        assert_eq!(tracker.remaining_bytes(), 500);
        assert!((tracker.usage_ratio() - 0.5).abs() < 1e-6);

        // Fits exact boundary
        tracker.record_store(500).unwrap();
        assert_eq!(tracker.used_bytes, 1_000);
        assert_eq!(tracker.remaining_bytes(), 0);

        // Exceeds quota
        let err = tracker.record_store(1).unwrap_err();
        assert_eq!(
            err,
            QuotaError::Exceeded {
                limit_bytes: 1_000,
                current_bytes: 1_000,
                additional_bytes: 1,
            }
        );
    }

    #[test]
    fn test_quota_delete_recycle() {
        let mut tracker = QuotaTracker::new(10_000, 1_000);
        tracker.record_store(800).unwrap();
        tracker.record_delete(300);
        assert_eq!(tracker.used_bytes, 500);
        assert!(tracker.can_store(500));
    }

    #[test]
    fn test_android_policy_charging_guard() {
        let state = AndroidDeviceState {
            total_device_storage_bytes: 128_000_000_000,
            free_device_storage_bytes: 50_000_000_000,
            is_charging: false,
            is_unmetered_wifi: true,
            requested_contribution_pct: 2.0,
        };

        let decision = evaluate_android_policy(
            &state,
            &AndroidPowerPolicy::OnlyCharging,
            &AndroidNetworkPolicy::UnmeteredWifiOnly,
        );

        assert!(!decision.allowed_to_operate);
        assert_eq!(decision.effective_quota_bytes, 0);
        assert!(decision.reason.contains("not charging"));
    }

    #[test]
    fn test_android_policy_unmetered_wifi_guard() {
        let state = AndroidDeviceState {
            total_device_storage_bytes: 128_000_000_000,
            free_device_storage_bytes: 50_000_000_000,
            is_charging: true,
            is_unmetered_wifi: false, // on cellular
            requested_contribution_pct: 2.0,
        };

        let decision = evaluate_android_policy(
            &state,
            &AndroidPowerPolicy::OnlyCharging,
            &AndroidNetworkPolicy::UnmeteredWifiOnly,
        );

        assert!(!decision.allowed_to_operate);
        assert!(decision.reason.contains("not on unmetered Wi-Fi"));
    }

    #[test]
    fn test_android_policy_host_storage_floor_10_percent() {
        let state = AndroidDeviceState {
            total_device_storage_bytes: 100_000_000_000,
            free_device_storage_bytes: 8_000_000_000, // 8% free (< 10% floor!)
            is_charging: true,
            is_unmetered_wifi: true,
            requested_contribution_pct: 2.0,
        };

        let decision = evaluate_android_policy(
            &state,
            &AndroidPowerPolicy::AnyPower,
            &AndroidNetworkPolicy::AnyNetwork,
        );

        assert!(!decision.allowed_to_operate);
        assert_eq!(decision.effective_quota_bytes, 0);
        assert!(decision.reason.contains("critically low"));
    }

    #[test]
    fn test_android_policy_envelope_clamping() {
        let state_over = AndroidDeviceState {
            total_device_storage_bytes: 100_000_000_000,
            free_device_storage_bytes: 50_000_000_000,
            is_charging: true,
            is_unmetered_wifi: true,
            requested_contribution_pct: 10.0, // Above 3% maximum ceiling
        };

        let decision_over = evaluate_android_policy(
            &state_over,
            &AndroidPowerPolicy::AnyPower,
            &AndroidNetworkPolicy::AnyNetwork,
        );

        assert!(decision_over.allowed_to_operate);
        // Clamped to 3% of 100GB = 3 GB
        assert_eq!(decision_over.effective_quota_bytes, 3_000_000_000);

        let state_under = AndroidDeviceState {
            total_device_storage_bytes: 100_000_000_000,
            free_device_storage_bytes: 50_000_000_000,
            is_charging: true,
            is_unmetered_wifi: true,
            requested_contribution_pct: 0.1, // Below 1% minimum floor
        };

        let decision_under = evaluate_android_policy(
            &state_under,
            &AndroidPowerPolicy::AnyPower,
            &AndroidNetworkPolicy::AnyNetwork,
        );

        assert!(decision_under.allowed_to_operate);
        // Clamped to 1% of 100GB = 1 GB
        assert_eq!(decision_under.effective_quota_bytes, 1_000_000_000);
    }
}
