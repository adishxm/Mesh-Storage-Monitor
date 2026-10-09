use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum TenantStatus {
    Active,
    Suspended,
    Terminated,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Tenant {
    pub tenant_id: String,
    pub name: String,
    pub slug: String,
    pub max_quota_bytes: u64,
    pub current_used_bytes: u64,
    pub status: TenantStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Tenant {
    pub fn new(tenant_id: String, name: String, slug: String, max_quota_bytes: u64) -> Self {
        let now = Utc::now();
        Self {
            tenant_id,
            name,
            slug,
            max_quota_bytes,
            current_used_bytes: 0,
            status: TenantStatus::Active,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn usage_ratio(&self) -> f64 {
        if self.max_quota_bytes == 0 {
            0.0
        } else {
            (self.current_used_bytes as f64 / self.max_quota_bytes as f64).min(1.0)
        }
    }

    pub fn remaining_bytes(&self) -> u64 {
        self.max_quota_bytes.saturating_sub(self.current_used_bytes)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Desktop,
    Mobile,
    Server,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    Active,
    Suspended,
    Revoked,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Device {
    pub device_id: String,
    pub tenant_id: String,
    pub peer_id: String,
    pub device_name: String,
    pub device_type: DeviceType,
    pub quota_bytes: u64,
    pub storage_used_bytes: u64,
    pub status: DeviceStatus,
    pub last_heartbeat: DateTime<Utc>,
    pub enrolled_at: DateTime<Utc>,
}

impl Device {
    pub fn new(
        device_id: String,
        tenant_id: String,
        peer_id: String,
        device_name: String,
        device_type: DeviceType,
        quota_bytes: u64,
    ) -> Self {
        let now = Utc::now();
        Self {
            device_id,
            tenant_id,
            peer_id,
            device_name,
            device_type,
            quota_bytes,
            storage_used_bytes: 0,
            status: DeviceStatus::Active,
            last_heartbeat: now,
            enrolled_at: now,
        }
    }

    pub fn is_online(&self, threshold_secs: i64) -> bool {
        let now = Utc::now();
        (now - self.last_heartbeat).num_seconds() <= threshold_secs
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Invite {
    pub invite_id: String,
    pub tenant_id: String,
    pub created_by_sub: String,
    pub invitation_token: String,
    pub single_use_nonce: String,
    pub expires_at: DateTime<Utc>,
    pub consumed: bool,
    pub consumed_by_peer_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Invite {
    pub fn is_valid(&self) -> bool {
        !self.consumed && Utc::now() < self.expires_at
    }
}

/// OpenID Connect (OIDC) JWT claims standard for multi-tenant SaaS authorization.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct OidcClaims {
    pub sub: String,
    pub email: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
    pub exp: i64,
}

impl OidcClaims {
    pub fn is_expired(&self) -> bool {
        Utc::now().timestamp() >= self.exp
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    pub fn can_access_tenant(&self, target_tenant_id: &str) -> bool {
        !self.is_expired() && (self.tenant_id == target_tenant_id || self.has_role("superadmin"))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TenantMetrics {
    pub tenant_id: String,
    pub total_quota_bytes: u64,
    pub total_used_bytes: u64,
    pub usage_ratio: f64,
    pub total_devices: usize,
    pub online_devices: usize,
    pub mobile_devices: usize,
    pub desktop_devices: usize,
    pub server_devices: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_tenant_usage_and_remaining() {
        let mut tenant = Tenant::new(
            "tenant-1".to_string(),
            "Acme Corp".to_string(),
            "acme".to_string(),
            1000,
        );
        assert_eq!(tenant.usage_ratio(), 0.0);
        assert_eq!(tenant.remaining_bytes(), 1000);

        tenant.current_used_bytes = 400;
        assert_eq!(tenant.usage_ratio(), 0.4);
        assert_eq!(tenant.remaining_bytes(), 600);

        tenant.current_used_bytes = 1500;
        assert_eq!(tenant.usage_ratio(), 1.0);
        assert_eq!(tenant.remaining_bytes(), 0);
    }

    #[test]
    fn test_device_online_status() {
        let mut device = Device::new(
            "dev-1".to_string(),
            "tenant-1".to_string(),
            "peer-123".to_string(),
            "Workstation".to_string(),
            DeviceType::Desktop,
            5000,
        );
        assert!(device.is_online(60));

        // Set last heartbeat to 10 minutes ago
        device.last_heartbeat = Utc::now() - Duration::seconds(600);
        assert!(!device.is_online(60));
    }

    #[test]
    fn test_oidc_claims_tenant_isolation() {
        let valid_claims = OidcClaims {
            sub: "user-1".to_string(),
            email: "alice@acme.com".to_string(),
            tenant_id: "tenant-acme".to_string(),
            roles: vec!["member".to_string()],
            exp: Utc::now().timestamp() + 3600,
        };

        // Access to own tenant allowed
        assert!(valid_claims.can_access_tenant("tenant-acme"));

        // Access to other tenant strictly denied!
        assert!(!valid_claims.can_access_tenant("tenant-globex"));

        // Expired claims denied even for own tenant
        let expired_claims = OidcClaims {
            sub: "user-1".to_string(),
            email: "alice@acme.com".to_string(),
            tenant_id: "tenant-acme".to_string(),
            roles: vec!["member".to_string()],
            exp: Utc::now().timestamp() - 10,
        };
        assert!(!expired_claims.can_access_tenant("tenant-acme"));

        // Superadmin bypass allowed
        let superadmin_claims = OidcClaims {
            sub: "admin-root".to_string(),
            email: "root@meshstorage.cloud".to_string(),
            tenant_id: "system".to_string(),
            roles: vec!["superadmin".to_string()],
            exp: Utc::now().timestamp() + 3600,
        };
        assert!(superadmin_claims.can_access_tenant("tenant-globex"));
    }

    #[test]
    fn test_invite_validity() {
        let mut invite = Invite {
            invite_id: "inv-1".to_string(),
            tenant_id: "tenant-1".to_string(),
            created_by_sub: "admin-1".to_string(),
            invitation_token: "tok-abc".to_string(),
            single_use_nonce: "nonce-123".to_string(),
            expires_at: Utc::now() + Duration::hours(1),
            consumed: false,
            consumed_by_peer_id: None,
            created_at: Utc::now(),
        };
        assert!(invite.is_valid());

        // Consumed invite is invalid
        invite.consumed = true;
        assert!(!invite.is_valid());

        // Expired invite is invalid
        invite.consumed = false;
        invite.expires_at = Utc::now() - Duration::seconds(10);
        assert!(!invite.is_valid());
    }
}
