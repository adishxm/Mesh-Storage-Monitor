use chrono::{Duration, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::models::{
    Device, DeviceStatus, DeviceType, Invite, OidcClaims, PeerCreditReport, Tenant, TenantMetrics,
    TenantStatus,
};

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ServiceError {
    #[error("Tenant not found: {0}")]
    TenantNotFound(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Invite not found or invalid: {0}")]
    InviteNotFound(String),

    #[error("Invite has expired")]
    InviteExpired,

    #[error("Invite has already been consumed")]
    InviteAlreadyConsumed,

    #[error(
        "Tenant isolation violation: requester from {requester_tenant} attempted access to {target_tenant}"
    )]
    TenantIsolationViolation {
        requester_tenant: String,
        target_tenant: String,
    },

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error(
        "Tenant quota exceeded: requested {requested} bytes, but only {available} bytes remaining"
    )]
    QuotaExceeded { requested: u64, available: u64 },

    #[error("Tenant slug already exists: {0}")]
    DuplicateSlug(String),

    #[error("Device peer ID already registered in tenant: {0}")]
    DuplicateDevice(String),

    #[error("Tenant is suspended or terminated: {0}")]
    TenantInactive(String),
}

#[derive(Clone, Default)]
pub struct ControlPlaneService {
    tenants: Arc<RwLock<HashMap<String, Tenant>>>,
    devices: Arc<RwLock<HashMap<String, Device>>>,
    invites: Arc<RwLock<HashMap<String, Invite>>>,
    credits: Arc<RwLock<HashMap<String, mesh_core::CreditLedger>>>,
}

impl ControlPlaneService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Verifies that OidcClaims allow access to target_tenant_id.
    pub fn verify_tenant_access(
        claims: &OidcClaims,
        target_tenant_id: &str,
    ) -> Result<(), ServiceError> {
        if !claims.can_access_tenant(target_tenant_id) {
            return Err(ServiceError::TenantIsolationViolation {
                requester_tenant: claims.tenant_id.clone(),
                target_tenant: target_tenant_id.to_string(),
            });
        }
        Ok(())
    }

    pub async fn create_tenant(
        &self,
        claims: &OidcClaims,
        name: String,
        slug: String,
        max_quota_bytes: u64,
    ) -> Result<Tenant, ServiceError> {
        if !claims.has_role("admin") && !claims.has_role("superadmin") {
            return Err(ServiceError::Unauthorized(
                "Only admins can create tenants".to_string(),
            ));
        }

        let mut tenants = self.tenants.write().await;
        if tenants.values().any(|t| t.slug == slug) {
            return Err(ServiceError::DuplicateSlug(slug));
        }

        let tenant_id = format!("ten_{}", Uuid::new_v4().simple());
        let tenant = Tenant::new(tenant_id.clone(), name, slug, max_quota_bytes);
        tenants.insert(tenant_id, tenant.clone());
        Ok(tenant)
    }

    pub async fn get_tenant(
        &self,
        claims: &OidcClaims,
        tenant_id: &str,
    ) -> Result<Tenant, ServiceError> {
        Self::verify_tenant_access(claims, tenant_id)?;
        let tenants = self.tenants.read().await;
        tenants
            .get(tenant_id)
            .cloned()
            .ok_or_else(|| ServiceError::TenantNotFound(tenant_id.to_string()))
    }

    pub async fn register_device(
        &self,
        claims: &OidcClaims,
        tenant_id: &str,
        peer_id: &str,
        device_name: &str,
        device_type: DeviceType,
        quota_bytes: u64,
    ) -> Result<Device, ServiceError> {
        Self::verify_tenant_access(claims, tenant_id)?;

        let mut tenants = self.tenants.write().await;
        let tenant = tenants
            .get_mut(tenant_id)
            .ok_or_else(|| ServiceError::TenantNotFound(tenant_id.to_string()))?;

        if tenant.status != TenantStatus::Active {
            return Err(ServiceError::TenantInactive(tenant_id.to_string()));
        }

        let mut devices = self.devices.write().await;
        if devices
            .values()
            .any(|d| d.tenant_id == tenant_id && d.peer_id == peer_id)
        {
            return Err(ServiceError::DuplicateDevice(peer_id.to_string()));
        }

        let allocated_bytes: u64 = devices
            .values()
            .filter(|d| d.tenant_id == tenant_id && d.status != DeviceStatus::Revoked)
            .map(|d| d.quota_bytes)
            .sum();

        let available = tenant.max_quota_bytes.saturating_sub(allocated_bytes);
        if quota_bytes > available {
            return Err(ServiceError::QuotaExceeded {
                requested: quota_bytes,
                available,
            });
        }

        let device_id = format!("dev_{}", Uuid::new_v4().simple());
        let device = Device::new(
            device_id.clone(),
            tenant_id.to_string(),
            peer_id.to_string(),
            device_name.to_string(),
            device_type,
            quota_bytes,
        );

        devices.insert(device_id, device.clone());
        Ok(device)
    }

    pub async fn list_devices(
        &self,
        claims: &OidcClaims,
        tenant_id: &str,
    ) -> Result<Vec<Device>, ServiceError> {
        Self::verify_tenant_access(claims, tenant_id)?;

        let devices = self.devices.read().await;
        let tenant_devices = devices
            .values()
            .filter(|d| d.tenant_id == tenant_id)
            .cloned()
            .collect();
        Ok(tenant_devices)
    }

    pub async fn update_device_heartbeat(
        &self,
        tenant_id: &str,
        device_id: &str,
        storage_used_bytes: u64,
    ) -> Result<Device, ServiceError> {
        let mut devices = self.devices.write().await;
        let device = devices
            .get_mut(device_id)
            .ok_or_else(|| ServiceError::DeviceNotFound(device_id.to_string()))?;

        if device.tenant_id != tenant_id {
            return Err(ServiceError::TenantIsolationViolation {
                requester_tenant: tenant_id.to_string(),
                target_tenant: device.tenant_id.clone(),
            });
        }

        let old_used = device.storage_used_bytes;
        device.last_heartbeat = Utc::now();
        device.storage_used_bytes = storage_used_bytes;
        let cloned_dev = device.clone();

        // Update aggregated tenant usage
        let mut tenants = self.tenants.write().await;
        if let Some(tenant) = tenants.get_mut(tenant_id) {
            if storage_used_bytes >= old_used {
                tenant.current_used_bytes += storage_used_bytes - old_used;
            } else {
                tenant.current_used_bytes = tenant
                    .current_used_bytes
                    .saturating_sub(old_used - storage_used_bytes);
            }
            tenant.updated_at = Utc::now();
        }

        Ok(cloned_dev)
    }

    pub async fn update_device_status(
        &self,
        claims: &OidcClaims,
        tenant_id: &str,
        device_id: &str,
        status: DeviceStatus,
    ) -> Result<Device, ServiceError> {
        Self::verify_tenant_access(claims, tenant_id)?;
        if !claims.has_role("admin") && !claims.has_role("superadmin") {
            return Err(ServiceError::Unauthorized(
                "Only admins can modify device status".to_string(),
            ));
        }

        let mut devices = self.devices.write().await;
        let device = devices
            .get_mut(device_id)
            .ok_or_else(|| ServiceError::DeviceNotFound(device_id.to_string()))?;

        if device.tenant_id != tenant_id {
            return Err(ServiceError::TenantIsolationViolation {
                requester_tenant: tenant_id.to_string(),
                target_tenant: device.tenant_id.clone(),
            });
        }

        device.status = status;
        Ok(device.clone())
    }

    pub async fn create_invite(
        &self,
        claims: &OidcClaims,
        tenant_id: &str,
        duration_secs: u64,
    ) -> Result<Invite, ServiceError> {
        Self::verify_tenant_access(claims, tenant_id)?;
        if !claims.has_role("admin") && !claims.has_role("superadmin") {
            return Err(ServiceError::Unauthorized(
                "Only admins can issue tenant invitations".to_string(),
            ));
        }

        let tenants = self.tenants.read().await;
        let tenant = tenants
            .get(tenant_id)
            .ok_or_else(|| ServiceError::TenantNotFound(tenant_id.to_string()))?;

        if tenant.status != TenantStatus::Active {
            return Err(ServiceError::TenantInactive(tenant_id.to_string()));
        }

        let invite_id = format!("inv_{}", Uuid::new_v4().simple());
        let invitation_token = format!("tok_{}", Uuid::new_v4().simple());
        let single_use_nonce = format!("nonce_{}", Uuid::new_v4().simple());
        let now = Utc::now();
        let expires_at = now + Duration::seconds(duration_secs as i64);

        let invite = Invite {
            invite_id: invite_id.clone(),
            tenant_id: tenant_id.to_string(),
            created_by_sub: claims.sub.clone(),
            invitation_token,
            single_use_nonce,
            expires_at,
            consumed: false,
            consumed_by_peer_id: None,
            created_at: now,
        };

        let mut invites = self.invites.write().await;
        invites.insert(invite_id, invite.clone());
        Ok(invite)
    }

    pub async fn consume_invite(
        &self,
        invitation_token: &str,
        peer_id: &str,
        device_name: &str,
        device_type: DeviceType,
        quota_bytes: u64,
    ) -> Result<Device, ServiceError> {
        let mut invites = self.invites.write().await;
        let invite = invites
            .values_mut()
            .find(|inv| inv.invitation_token == invitation_token)
            .ok_or_else(|| ServiceError::InviteNotFound(invitation_token.to_string()))?;

        if invite.consumed {
            return Err(ServiceError::InviteAlreadyConsumed);
        }
        if Utc::now() >= invite.expires_at {
            return Err(ServiceError::InviteExpired);
        }

        let tenant_id = invite.tenant_id.clone();
        let mut tenants = self.tenants.write().await;
        let tenant = tenants
            .get_mut(&tenant_id)
            .ok_or_else(|| ServiceError::TenantNotFound(tenant_id.clone()))?;

        if tenant.status != TenantStatus::Active {
            return Err(ServiceError::TenantInactive(tenant_id.clone()));
        }

        let mut devices = self.devices.write().await;
        if devices
            .values()
            .any(|d| d.tenant_id == tenant_id && d.peer_id == peer_id)
        {
            return Err(ServiceError::DuplicateDevice(peer_id.to_string()));
        }

        let allocated_bytes: u64 = devices
            .values()
            .filter(|d| d.tenant_id == tenant_id && d.status != DeviceStatus::Revoked)
            .map(|d| d.quota_bytes)
            .sum();

        let available = tenant.max_quota_bytes.saturating_sub(allocated_bytes);
        if quota_bytes > available {
            return Err(ServiceError::QuotaExceeded {
                requested: quota_bytes,
                available,
            });
        }

        // Mark invite as consumed
        invite.consumed = true;
        invite.consumed_by_peer_id = Some(peer_id.to_string());

        let device_id = format!("dev_{}", Uuid::new_v4().simple());
        let device = Device::new(
            device_id.clone(),
            tenant_id,
            peer_id.to_string(),
            device_name.to_string(),
            device_type,
            quota_bytes,
        );

        devices.insert(device_id, device.clone());
        Ok(device)
    }

    pub async fn get_tenant_metrics(
        &self,
        claims: &OidcClaims,
        tenant_id: &str,
    ) -> Result<TenantMetrics, ServiceError> {
        Self::verify_tenant_access(claims, tenant_id)?;

        let tenants = self.tenants.read().await;
        let tenant = tenants
            .get(tenant_id)
            .ok_or_else(|| ServiceError::TenantNotFound(tenant_id.to_string()))?;

        let devices = self.devices.read().await;
        let tenant_devices: Vec<&Device> = devices
            .values()
            .filter(|d| d.tenant_id == tenant_id)
            .collect();

        let total_devices = tenant_devices.len();
        let online_devices = tenant_devices.iter().filter(|d| d.is_online(300)).count();
        let mobile_devices = tenant_devices
            .iter()
            .filter(|d| d.device_type == DeviceType::Mobile)
            .count();
        let desktop_devices = tenant_devices
            .iter()
            .filter(|d| d.device_type == DeviceType::Desktop)
            .count();
        let server_devices = tenant_devices
            .iter()
            .filter(|d| d.device_type == DeviceType::Server)
            .count();

        Ok(TenantMetrics {
            tenant_id: tenant_id.to_string(),
            total_quota_bytes: tenant.max_quota_bytes,
            total_used_bytes: tenant.current_used_bytes,
            usage_ratio: tenant.usage_ratio(),
            total_devices,
            online_devices,
            mobile_devices,
            desktop_devices,
            server_devices,
        })
    }

    pub async fn get_peer_credit_report(&self, peer_id: &str) -> PeerCreditReport {
        let mut credits = self.credits.write().await;
        let ledger = credits.entry(peer_id.to_string()).or_insert_with(|| {
            mesh_core::CreditLedger::new(peer_id.to_string(), Utc::now().timestamp() as u64)
        });

        let base_free = 1_073_741_824; // 1 GB free base
        let ratio = 1.0;
        let cap = 100_000_000_000; // 100 GB cap
        PeerCreditReport {
            peer_id: peer_id.to_string(),
            tier: ledger.evaluate_tier(base_free, ratio, cap),
            bytes_contributed: ledger.bytes_contributed,
            bytes_consumed: ledger.bytes_consumed,
            earned_allowance_bytes: ledger.earned_allowance_bytes(base_free, ratio, cap),
            credit_balance: ledger.credit_balance(base_free, ratio, cap),
            fair_share_ratio: ledger.fair_share_ratio(),
            audits_passed: ledger.audits_passed,
            audits_failed: ledger.audits_failed,
        }
    }

    pub async fn record_peer_storage_activity(
        &self,
        peer_id: &str,
        contributed_delta: u64,
        consumed_delta: u64,
        audit_pass: Option<bool>,
    ) {
        let mut credits = self.credits.write().await;
        let ledger = credits.entry(peer_id.to_string()).or_insert_with(|| {
            mesh_core::CreditLedger::new(peer_id.to_string(), Utc::now().timestamp() as u64)
        });

        if contributed_delta > 0 {
            ledger.record_storage_contribution(contributed_delta);
        }
        if consumed_delta > 0 {
            ledger.record_storage_consumption(consumed_delta);
        }
        if let Some(passed) = audit_pass {
            ledger.record_audit(passed);
        }
    }
}
