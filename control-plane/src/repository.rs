//! Control-Plane Repository Abstraction with Transactional Semantics and Crash-Safe File Persistence
//! REQ-17: Persistent tenant isolation, transactional invites, and crash-safe storage.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use uuid::Uuid;

use crate::models::{Device, DeviceStatus, DeviceType, Invite, Tenant, TenantStatus};
use crate::service::ServiceError;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CreditLedgerEvent {
    pub event_id: String,
    pub tenant_id: String,
    pub peer_id: String,
    pub delta: i64,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ControlPlaneSnapshot {
    pub tenants: HashMap<String, Tenant>,
    pub devices: HashMap<String, Device>,
    pub invites: HashMap<String, Invite>,
    pub credits: HashMap<String, mesh_core::CreditLedger>,
    pub credit_events: Vec<CreditLedgerEvent>,
}

/// Abstract Control Plane repository interface for storage backends.
pub trait ControlPlaneRepository: Send + Sync {
    fn create_tenant(&self, tenant: Tenant) -> Result<Tenant, ServiceError>;
    fn get_tenant(&self, tenant_id: &str) -> Result<Option<Tenant>, ServiceError>;
    fn list_tenants(&self) -> Result<Vec<Tenant>, ServiceError>;
    fn update_tenant(&self, tenant: Tenant) -> Result<Tenant, ServiceError>;

    fn register_device(&self, device: Device) -> Result<Device, ServiceError>;
    fn get_device(&self, device_id: &str) -> Result<Option<Device>, ServiceError>;
    fn list_devices_by_tenant(&self, tenant_id: &str) -> Result<Vec<Device>, ServiceError>;
    fn update_device(&self, device: Device) -> Result<Device, ServiceError>;

    fn create_invite(&self, invite: Invite) -> Result<Invite, ServiceError>;
    fn get_invite(&self, invite_id: &str) -> Result<Option<Invite>, ServiceError>;
    fn get_invite_by_token(&self, token: &str) -> Result<Option<Invite>, ServiceError>;

    /// Atomically consumes an invite, validating expiration, single-use status,
    /// tenant status, quota limits, and device unicity under an exclusive transaction lock.
    fn consume_invite_transactional(
        &self,
        token: &str,
        peer_id: &str,
        device_name: &str,
        device_type: DeviceType,
        quota_bytes: u64,
    ) -> Result<Device, ServiceError>;

    fn record_credit_event(&self, event: CreditLedgerEvent) -> Result<(), ServiceError>;
    fn get_credit_ledger(
        &self,
        tenant_id: &str,
    ) -> Result<Option<mesh_core::CreditLedger>, ServiceError>;
    fn update_credit_ledger(
        &self,
        tenant_id: &str,
        ledger: mesh_core::CreditLedger,
    ) -> Result<(), ServiceError>;
    fn list_credit_events(&self, tenant_id: &str) -> Result<Vec<CreditLedgerEvent>, ServiceError>;

    fn export_snapshot(&self) -> Result<ControlPlaneSnapshot, ServiceError>;
    fn import_snapshot(&self, snapshot: ControlPlaneSnapshot) -> Result<(), ServiceError>;
}

/// Thread-safe in-memory repository with transactional integrity checks.
#[derive(Debug, Default)]
pub struct InMemoryRepository {
    state: RwLock<ControlPlaneSnapshot>,
}

impl InMemoryRepository {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_snapshot(snapshot: ControlPlaneSnapshot) -> Self {
        Self {
            state: RwLock::new(snapshot),
        }
    }

    fn read_lock(
        &self,
    ) -> Result<std::sync::RwLockReadGuard<'_, ControlPlaneSnapshot>, ServiceError> {
        self.state
            .read()
            .map_err(|e| ServiceError::Internal(format!("RwLock poisoned: {}", e)))
    }

    fn write_lock(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, ControlPlaneSnapshot>, ServiceError> {
        self.state
            .write()
            .map_err(|e| ServiceError::Internal(format!("RwLock poisoned: {}", e)))
    }
}

impl ControlPlaneRepository for InMemoryRepository {
    fn create_tenant(&self, tenant: Tenant) -> Result<Tenant, ServiceError> {
        let mut state = self.write_lock()?;
        if state.tenants.values().any(|t| t.slug == tenant.slug) {
            return Err(ServiceError::DuplicateSlug(tenant.slug));
        }
        state
            .tenants
            .insert(tenant.tenant_id.clone(), tenant.clone());
        Ok(tenant)
    }

    fn get_tenant(&self, tenant_id: &str) -> Result<Option<Tenant>, ServiceError> {
        let state = self.read_lock()?;
        Ok(state.tenants.get(tenant_id).cloned())
    }

    fn list_tenants(&self) -> Result<Vec<Tenant>, ServiceError> {
        let state = self.read_lock()?;
        Ok(state.tenants.values().cloned().collect())
    }

    fn update_tenant(&self, tenant: Tenant) -> Result<Tenant, ServiceError> {
        let mut state = self.write_lock()?;
        state
            .tenants
            .insert(tenant.tenant_id.clone(), tenant.clone());
        Ok(tenant)
    }

    fn register_device(&self, device: Device) -> Result<Device, ServiceError> {
        let mut state = self.write_lock()?;
        if state
            .devices
            .values()
            .any(|d| d.tenant_id == device.tenant_id && d.peer_id == device.peer_id)
        {
            return Err(ServiceError::DuplicateDevice(device.peer_id));
        }
        state
            .devices
            .insert(device.device_id.clone(), device.clone());
        Ok(device)
    }

    fn get_device(&self, device_id: &str) -> Result<Option<Device>, ServiceError> {
        let state = self.read_lock()?;
        Ok(state.devices.get(device_id).cloned())
    }

    fn list_devices_by_tenant(&self, tenant_id: &str) -> Result<Vec<Device>, ServiceError> {
        let state = self.read_lock()?;
        Ok(state
            .devices
            .values()
            .filter(|d| d.tenant_id == tenant_id)
            .cloned()
            .collect())
    }

    fn update_device(&self, device: Device) -> Result<Device, ServiceError> {
        let mut state = self.write_lock()?;
        state
            .devices
            .insert(device.device_id.clone(), device.clone());
        Ok(device)
    }

    fn create_invite(&self, invite: Invite) -> Result<Invite, ServiceError> {
        let mut state = self.write_lock()?;
        if state
            .invites
            .values()
            .any(|i| i.invitation_token == invite.invitation_token)
        {
            return Err(ServiceError::Internal("Duplicate invite token".into()));
        }
        if state
            .invites
            .values()
            .any(|i| i.single_use_nonce == invite.single_use_nonce)
        {
            return Err(ServiceError::Internal("Duplicate invite nonce".into()));
        }
        state
            .invites
            .insert(invite.invite_id.clone(), invite.clone());
        Ok(invite)
    }

    fn get_invite(&self, invite_id: &str) -> Result<Option<Invite>, ServiceError> {
        let state = self.read_lock()?;
        Ok(state.invites.get(invite_id).cloned())
    }

    fn get_invite_by_token(&self, token: &str) -> Result<Option<Invite>, ServiceError> {
        let state = self.read_lock()?;
        Ok(state
            .invites
            .values()
            .find(|i| i.invitation_token == token)
            .cloned())
    }

    fn consume_invite_transactional(
        &self,
        token: &str,
        peer_id: &str,
        device_name: &str,
        device_type: DeviceType,
        quota_bytes: u64,
    ) -> Result<Device, ServiceError> {
        let mut state = self.write_lock()?;

        // 1. Find invite id and validate freshness
        let invite_id = {
            let invite = state
                .invites
                .values()
                .find(|i| i.invitation_token == token)
                .ok_or_else(|| ServiceError::InviteNotFound(token.to_string()))?;

            if invite.consumed {
                return Err(ServiceError::InviteAlreadyConsumed);
            }
            if Utc::now() >= invite.expires_at {
                return Err(ServiceError::InviteExpired);
            }
            invite.invite_id.clone()
        };

        let tenant_id = state.invites.get(&invite_id).unwrap().tenant_id.clone();

        // 3. Validate tenant
        let tenant = state
            .tenants
            .get(&tenant_id)
            .ok_or_else(|| ServiceError::TenantNotFound(tenant_id.clone()))?;

        if tenant.status != TenantStatus::Active {
            return Err(ServiceError::TenantInactive(tenant_id.clone()));
        }

        // 4. Validate device uniqueness
        if state
            .devices
            .values()
            .any(|d| d.tenant_id == tenant_id && d.peer_id == peer_id)
        {
            return Err(ServiceError::DuplicateDevice(peer_id.to_string()));
        }

        // 5. Validate quota
        let allocated_bytes: u64 = state
            .devices
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

        // 6. Commit transactionally
        if let Some(inv) = state.invites.get_mut(&invite_id) {
            inv.consumed = true;
            inv.consumed_by_peer_id = Some(peer_id.to_string());
        }

        let device_id = format!("dev_{}", Uuid::new_v4().simple());
        let device = Device::new(
            device_id.clone(),
            tenant_id,
            peer_id.to_string(),
            device_name.to_string(),
            device_type,
            quota_bytes,
        );

        state.devices.insert(device_id, device.clone());
        Ok(device)
    }

    fn record_credit_event(&self, event: CreditLedgerEvent) -> Result<(), ServiceError> {
        let mut state = self.write_lock()?;
        state.credit_events.push(event);
        Ok(())
    }

    fn get_credit_ledger(
        &self,
        tenant_id: &str,
    ) -> Result<Option<mesh_core::CreditLedger>, ServiceError> {
        let state = self.read_lock()?;
        Ok(state.credits.get(tenant_id).cloned())
    }

    fn update_credit_ledger(
        &self,
        tenant_id: &str,
        ledger: mesh_core::CreditLedger,
    ) -> Result<(), ServiceError> {
        let mut state = self.write_lock()?;
        state.credits.insert(tenant_id.to_string(), ledger);
        Ok(())
    }

    fn list_credit_events(&self, tenant_id: &str) -> Result<Vec<CreditLedgerEvent>, ServiceError> {
        let state = self.read_lock()?;
        Ok(state
            .credit_events
            .iter()
            .filter(|e| e.tenant_id == tenant_id)
            .cloned()
            .collect())
    }

    fn export_snapshot(&self) -> Result<ControlPlaneSnapshot, ServiceError> {
        let state = self.read_lock()?;
        Ok(state.clone())
    }

    fn import_snapshot(&self, snapshot: ControlPlaneSnapshot) -> Result<(), ServiceError> {
        let mut state = self.write_lock()?;
        *state = snapshot;
        Ok(())
    }
}

/// Crash-safe persistent repository that automatically flushes to disk upon mutations
/// and restores state upon restart.
#[derive(Debug)]
pub struct FilePersistentRepository {
    inner: InMemoryRepository,
    file_path: PathBuf,
}

impl FilePersistentRepository {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, ServiceError> {
        let file_path = path.as_ref().to_path_buf();
        let initial_snapshot = if file_path.exists() {
            let data = fs::read_to_string(&file_path).map_err(|e| {
                ServiceError::Internal(format!("Failed to read persistence file: {}", e))
            })?;
            serde_json::from_str::<ControlPlaneSnapshot>(&data).map_err(|e| {
                ServiceError::Internal(format!("Corrupt persistence snapshot: {}", e))
            })?
        } else {
            if let Some(parent) = file_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            ControlPlaneSnapshot::default()
        };

        Ok(Self {
            inner: InMemoryRepository::from_snapshot(initial_snapshot),
            file_path,
        })
    }

    fn persist_to_disk(&self) -> Result<(), ServiceError> {
        let snapshot = self.inner.export_snapshot()?;
        let serialized = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| ServiceError::Internal(format!("Failed to serialize snapshot: {}", e)))?;

        let temp_path = self.file_path.with_extension("tmp");
        fs::write(&temp_path, serialized)
            .map_err(|e| ServiceError::Internal(format!("Failed to write tmp file: {}", e)))?;
        fs::rename(&temp_path, &self.file_path).map_err(|e| {
            ServiceError::Internal(format!("Failed to commit persistence file: {}", e))
        })?;
        Ok(())
    }
}

impl ControlPlaneRepository for FilePersistentRepository {
    fn create_tenant(&self, tenant: Tenant) -> Result<Tenant, ServiceError> {
        let res = self.inner.create_tenant(tenant)?;
        self.persist_to_disk()?;
        Ok(res)
    }

    fn get_tenant(&self, tenant_id: &str) -> Result<Option<Tenant>, ServiceError> {
        self.inner.get_tenant(tenant_id)
    }

    fn list_tenants(&self) -> Result<Vec<Tenant>, ServiceError> {
        self.inner.list_tenants()
    }

    fn update_tenant(&self, tenant: Tenant) -> Result<Tenant, ServiceError> {
        let res = self.inner.update_tenant(tenant)?;
        self.persist_to_disk()?;
        Ok(res)
    }

    fn register_device(&self, device: Device) -> Result<Device, ServiceError> {
        let res = self.inner.register_device(device)?;
        self.persist_to_disk()?;
        Ok(res)
    }

    fn get_device(&self, device_id: &str) -> Result<Option<Device>, ServiceError> {
        self.inner.get_device(device_id)
    }

    fn list_devices_by_tenant(&self, tenant_id: &str) -> Result<Vec<Device>, ServiceError> {
        self.inner.list_devices_by_tenant(tenant_id)
    }

    fn update_device(&self, device: Device) -> Result<Device, ServiceError> {
        let res = self.inner.update_device(device)?;
        self.persist_to_disk()?;
        Ok(res)
    }

    fn create_invite(&self, invite: Invite) -> Result<Invite, ServiceError> {
        let res = self.inner.create_invite(invite)?;
        self.persist_to_disk()?;
        Ok(res)
    }

    fn get_invite(&self, invite_id: &str) -> Result<Option<Invite>, ServiceError> {
        self.inner.get_invite(invite_id)
    }

    fn get_invite_by_token(&self, token: &str) -> Result<Option<Invite>, ServiceError> {
        self.inner.get_invite_by_token(token)
    }

    fn consume_invite_transactional(
        &self,
        token: &str,
        peer_id: &str,
        device_name: &str,
        device_type: DeviceType,
        quota_bytes: u64,
    ) -> Result<Device, ServiceError> {
        let res = self.inner.consume_invite_transactional(
            token,
            peer_id,
            device_name,
            device_type,
            quota_bytes,
        )?;
        self.persist_to_disk()?;
        Ok(res)
    }

    fn record_credit_event(&self, event: CreditLedgerEvent) -> Result<(), ServiceError> {
        self.inner.record_credit_event(event)?;
        self.persist_to_disk()?;
        Ok(())
    }

    fn get_credit_ledger(
        &self,
        tenant_id: &str,
    ) -> Result<Option<mesh_core::CreditLedger>, ServiceError> {
        self.inner.get_credit_ledger(tenant_id)
    }

    fn update_credit_ledger(
        &self,
        tenant_id: &str,
        ledger: mesh_core::CreditLedger,
    ) -> Result<(), ServiceError> {
        self.inner.update_credit_ledger(tenant_id, ledger)?;
        self.persist_to_disk()?;
        Ok(())
    }

    fn list_credit_events(&self, tenant_id: &str) -> Result<Vec<CreditLedgerEvent>, ServiceError> {
        self.inner.list_credit_events(tenant_id)
    }

    fn export_snapshot(&self) -> Result<ControlPlaneSnapshot, ServiceError> {
        self.inner.export_snapshot()
    }

    fn import_snapshot(&self, snapshot: ControlPlaneSnapshot) -> Result<(), ServiceError> {
        self.inner.import_snapshot(snapshot)?;
        self.persist_to_disk()?;
        Ok(())
    }
}
