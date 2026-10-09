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
}
