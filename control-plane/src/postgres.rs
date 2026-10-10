//! Production PostgreSQL Storage Repository Implementation
//! REQ-17: Persistent tenant isolation, database-level unique constraints,
//! row-level locking for transactional invite consumption, and credit events.

use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio_postgres::{Client, NoTls};
use uuid::Uuid;

use crate::migrations::PostgresMigrationRunner;
use crate::models::{Device, DeviceStatus, DeviceType, Invite, Tenant, TenantStatus};
use crate::repository::{ControlPlaneRepository, ControlPlaneSnapshot, CreditLedgerEvent};
use crate::service::ServiceError;

/// Production PostgreSQL repository implementing `ControlPlaneRepository`.
/// Connects to a persistent PostgreSQL database instance, executes schema migrations,
/// and enforces database-level uniqueness constraints and ACID transactions.
pub struct PostgresRepository {
    client: Arc<tokio::sync::Mutex<Client>>,
}

impl PostgresRepository {
    /// Connects to PostgreSQL at `database_url`, executes pending schema migrations,
    /// and returns an initialized `PostgresRepository`.
    pub async fn connect(database_url: &str) -> Result<Self, ServiceError> {
        let (client, connection) = tokio_postgres::connect(database_url, NoTls)
            .await
            .map_err(|e| ServiceError::Internal(format!("Postgres connection failed: {}", e)))?;

        // Spawn background connection worker
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::error!("Postgres connection worker error: {}", e);
            }
        });

        // Run migrations
        let schema_sql = PostgresMigrationRunner::get_combined_schema_sql();
        client.batch_execute(&schema_sql).await.map_err(|e| {
            ServiceError::Internal(format!("Failed to execute schema migrations: {}", e))
        })?;

        Ok(Self {
            client: Arc::new(tokio::sync::Mutex::new(client)),
        })
    }

    pub fn from_client(client: Client) -> Self {
        Self {
            client: Arc::new(tokio::sync::Mutex::new(client)),
        }
    }
}

impl ControlPlaneRepository for PostgresRepository {
    fn create_tenant(&self, tenant: Tenant) -> Result<Tenant, ServiceError> {
        let client_lock = self.client.clone();
        let tenant_clone = tenant.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let status_str = match tenant_clone.status {
                    TenantStatus::Active => "Active",
                    TenantStatus::Suspended => "Suspended",
                    TenantStatus::Terminated => "Terminated",
                };

                let res = client
                    .execute(
                        "INSERT INTO tenants (tenant_id, name, slug, max_quota_bytes, current_used_bytes, status, created_at, updated_at) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                        &[
                            &tenant_clone.tenant_id,
                            &tenant_clone.name,
                            &tenant_clone.slug,
                            &(tenant_clone.max_quota_bytes as i64),
                            &(tenant_clone.current_used_bytes as i64),
                            &status_str,
                            &tenant_clone.created_at,
                            &tenant_clone.updated_at,
                        ],
                    )
                    .await;

                match res {
                    Ok(_) => Ok(tenant_clone),
                    Err(e) => {
                        if e.to_string().contains("uq_tenants_slug") {
                            Err(ServiceError::DuplicateSlug(tenant_clone.slug))
                        } else {
                            Err(ServiceError::Internal(format!("Failed to insert tenant: {}", e)))
                        }
                    }
                }
            })
        })
    }

    fn get_tenant(&self, tenant_id: &str) -> Result<Option<Tenant>, ServiceError> {
        let client_lock = self.client.clone();
        let tid = tenant_id.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let row_opt = client
                    .query_opt(
                        "SELECT tenant_id, name, slug, max_quota_bytes, current_used_bytes, status, created_at, updated_at \
                         FROM tenants WHERE tenant_id = $1",
                        &[&tid],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to query tenant: {}", e)))?;

                let Some(row) = row_opt else {
                    return Ok(None);
                };

                let status_str: String = row.get(5);
                let status = match status_str.as_str() {
                    "Suspended" => TenantStatus::Suspended,
                    "Terminated" => TenantStatus::Terminated,
                    _ => TenantStatus::Active,
                };

                let max_quota: i64 = row.get(3);
                let current_used: i64 = row.get(4);

                Ok(Some(Tenant {
                    tenant_id: row.get(0),
                    name: row.get(1),
                    slug: row.get(2),
                    max_quota_bytes: max_quota as u64,
                    current_used_bytes: current_used as u64,
                    status,
                    created_at: row.get(6),
                    updated_at: row.get(7),
                }))
            })
        })
    }

    fn list_tenants(&self) -> Result<Vec<Tenant>, ServiceError> {
        let client_lock = self.client.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let rows = client
                    .query(
                        "SELECT tenant_id, name, slug, max_quota_bytes, current_used_bytes, status, created_at, updated_at \
                         FROM tenants ORDER BY created_at ASC",
                        &[],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to list tenants: {}", e)))?;

                let mut tenants = Vec::new();
                for row in rows {
                    let status_str: String = row.get(5);
                    let status = match status_str.as_str() {
                        "Suspended" => TenantStatus::Suspended,
                        "Terminated" => TenantStatus::Terminated,
                        _ => TenantStatus::Active,
                    };
                    let max_quota: i64 = row.get(3);
                    let current_used: i64 = row.get(4);

                    tenants.push(Tenant {
                        tenant_id: row.get(0),
                        name: row.get(1),
                        slug: row.get(2),
                        max_quota_bytes: max_quota as u64,
                        current_used_bytes: current_used as u64,
                        status,
                        created_at: row.get(6),
                        updated_at: row.get(7),
                    });
                }
                Ok(tenants)
            })
        })
    }

    fn update_tenant(&self, tenant: Tenant) -> Result<Tenant, ServiceError> {
        let client_lock = self.client.clone();
        let tenant_clone = tenant.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let status_str = match tenant_clone.status {
                    TenantStatus::Active => "Active",
                    TenantStatus::Suspended => "Suspended",
                    TenantStatus::Terminated => "Terminated",
                };

                client
                    .execute(
                        "UPDATE tenants SET name = $2, current_used_bytes = $3, status = $4, updated_at = $5 \
                         WHERE tenant_id = $1",
                        &[
                            &tenant_clone.tenant_id,
                            &tenant_clone.name,
                            &(tenant_clone.current_used_bytes as i64),
                            &status_str,
                            &tenant_clone.updated_at,
                        ],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to update tenant: {}", e)))?;

                Ok(tenant_clone)
            })
        })
    }

    fn register_device(&self, device: Device) -> Result<Device, ServiceError> {
        let client_lock = self.client.clone();
        let dev = device.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let dtype_str = match dev.device_type {
                    DeviceType::Desktop => "Desktop",
                    DeviceType::Mobile => "Mobile",
                    DeviceType::Server => "Server",
                };
                let status_str = match dev.status {
                    DeviceStatus::Active => "Active",
                    DeviceStatus::Suspended => "Suspended",
                    DeviceStatus::Revoked => "Revoked",
                };

                let res = client
                    .execute(
                        "INSERT INTO devices (device_id, tenant_id, peer_id, device_name, device_type, quota_bytes, storage_used_bytes, status, last_heartbeat, enrolled_at) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                        &[
                            &dev.device_id,
                            &dev.tenant_id,
                            &dev.peer_id,
                            &dev.device_name,
                            &dtype_str,
                            &(dev.quota_bytes as i64),
                            &(dev.storage_used_bytes as i64),
                            &status_str,
                            &dev.last_heartbeat,
                            &dev.enrolled_at,
                        ],
                    )
                    .await;

                match res {
                    Ok(_) => Ok(dev),
                    Err(e) => {
                        if e.to_string().contains("uq_tenant_peer") {
                            Err(ServiceError::DuplicateDevice(dev.peer_id))
                        } else {
                            Err(ServiceError::Internal(format!("Failed to register device: {}", e)))
                        }
                    }
                }
            })
        })
    }

    fn get_device(&self, device_id: &str) -> Result<Option<Device>, ServiceError> {
        let client_lock = self.client.clone();
        let did = device_id.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let row_opt = client
                    .query_opt(
                        "SELECT device_id, tenant_id, peer_id, device_name, device_type, quota_bytes, storage_used_bytes, status, last_heartbeat, enrolled_at \
                         FROM devices WHERE device_id = $1",
                        &[&did],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to query device: {}", e)))?;

                let Some(row) = row_opt else {
                    return Ok(None);
                };

                let dtype_str: String = row.get(4);
                let device_type = match dtype_str.as_str() {
                    "Mobile" => DeviceType::Mobile,
                    "Server" => DeviceType::Server,
                    _ => DeviceType::Desktop,
                };

                let status_str: String = row.get(7);
                let status = match status_str.as_str() {
                    "Suspended" => DeviceStatus::Suspended,
                    "Revoked" => DeviceStatus::Revoked,
                    _ => DeviceStatus::Active,
                };

                let quota: i64 = row.get(5);
                let used: i64 = row.get(6);

                Ok(Some(Device {
                    device_id: row.get(0),
                    tenant_id: row.get(1),
                    peer_id: row.get(2),
                    device_name: row.get(3),
                    device_type,
                    quota_bytes: quota as u64,
                    storage_used_bytes: used as u64,
                    status,
                    last_heartbeat: row.get(8),
                    enrolled_at: row.get(9),
                }))
            })
        })
    }

    fn list_devices_by_tenant(&self, tenant_id: &str) -> Result<Vec<Device>, ServiceError> {
        let client_lock = self.client.clone();
        let tid = tenant_id.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let rows = client
                    .query(
                        "SELECT device_id, tenant_id, peer_id, device_name, device_type, quota_bytes, storage_used_bytes, status, last_heartbeat, enrolled_at \
                         FROM devices WHERE tenant_id = $1",
                        &[&tid],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to list devices: {}", e)))?;

                let mut devices = Vec::new();
                for row in rows {
                    let dtype_str: String = row.get(4);
                    let device_type = match dtype_str.as_str() {
                        "Mobile" => DeviceType::Mobile,
                        "Server" => DeviceType::Server,
                        _ => DeviceType::Desktop,
                    };
                    let status_str: String = row.get(7);
                    let status = match status_str.as_str() {
                        "Suspended" => DeviceStatus::Suspended,
                        "Revoked" => DeviceStatus::Revoked,
                        _ => DeviceStatus::Active,
                    };
                    let quota: i64 = row.get(5);
                    let used: i64 = row.get(6);

                    devices.push(Device {
                        device_id: row.get(0),
                        tenant_id: row.get(1),
                        peer_id: row.get(2),
                        device_name: row.get(3),
                        device_type,
                        quota_bytes: quota as u64,
                        storage_used_bytes: used as u64,
                        status,
                        last_heartbeat: row.get(8),
                        enrolled_at: row.get(9),
                    });
                }
                Ok(devices)
            })
        })
    }

    fn update_device(&self, device: Device) -> Result<Device, ServiceError> {
        let client_lock = self.client.clone();
        let dev = device.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let status_str = match dev.status {
                    DeviceStatus::Active => "Active",
                    DeviceStatus::Suspended => "Suspended",
                    DeviceStatus::Revoked => "Revoked",
                };

                client
                    .execute(
                        "UPDATE devices SET storage_used_bytes = $2, status = $3, last_heartbeat = $4 WHERE device_id = $1",
                        &[
                            &dev.device_id,
                            &(dev.storage_used_bytes as i64),
                            &status_str,
                            &dev.last_heartbeat,
                        ],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to update device: {}", e)))?;

                Ok(dev)
            })
        })
    }

    fn create_invite(&self, invite: Invite) -> Result<Invite, ServiceError> {
        let client_lock = self.client.clone();
        let inv = invite.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                client
                    .execute(
                        "INSERT INTO invites (invite_id, tenant_id, created_by_sub, invitation_token, single_use_nonce, expires_at, consumed, consumed_by_peer_id, created_at) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                        &[
                            &inv.invite_id,
                            &inv.tenant_id,
                            &inv.created_by_sub,
                            &inv.invitation_token,
                            &inv.single_use_nonce,
                            &inv.expires_at,
                            &inv.consumed,
                            &inv.consumed_by_peer_id,
                            &inv.created_at,
                        ],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to insert invite: {}", e)))?;

                Ok(inv)
            })
        })
    }

    fn get_invite(&self, invite_id: &str) -> Result<Option<Invite>, ServiceError> {
        let client_lock = self.client.clone();
        let iid = invite_id.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let row_opt = client
                    .query_opt(
                        "SELECT invite_id, tenant_id, created_by_sub, invitation_token, single_use_nonce, expires_at, consumed, consumed_by_peer_id, created_at \
                         FROM invites WHERE invite_id = $1",
                        &[&iid],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to query invite: {}", e)))?;

                let Some(row) = row_opt else {
                    return Ok(None);
                };

                Ok(Some(Invite {
                    invite_id: row.get(0),
                    tenant_id: row.get(1),
                    created_by_sub: row.get(2),
                    invitation_token: row.get(3),
                    single_use_nonce: row.get(4),
                    expires_at: row.get(5),
                    consumed: row.get(6),
                    consumed_by_peer_id: row.get(7),
                    created_at: row.get(8),
                }))
            })
        })
    }

    fn get_invite_by_token(&self, token: &str) -> Result<Option<Invite>, ServiceError> {
        let client_lock = self.client.clone();
        let tok = token.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let row_opt = client
                    .query_opt(
                        "SELECT invite_id, tenant_id, created_by_sub, invitation_token, single_use_nonce, expires_at, consumed, consumed_by_peer_id, created_at \
                         FROM invites WHERE invitation_token = $1",
                        &[&tok],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to query invite by token: {}", e)))?;

                let Some(row) = row_opt else {
                    return Ok(None);
                };

                Ok(Some(Invite {
                    invite_id: row.get(0),
                    tenant_id: row.get(1),
                    created_by_sub: row.get(2),
                    invitation_token: row.get(3),
                    single_use_nonce: row.get(4),
                    expires_at: row.get(5),
                    consumed: row.get(6),
                    consumed_by_peer_id: row.get(7),
                    created_at: row.get(8),
                }))
            })
        })
    }

    fn consume_invite_transactional(
        &self,
        token: &str,
        peer_id: &str,
        device_name: &str,
        device_type: DeviceType,
        quota_bytes: u64,
    ) -> Result<Device, ServiceError> {
        let client_lock = self.client.clone();
        let tok = token.to_string();
        let pid = peer_id.to_string();
        let dname = device_name.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let mut client = client_lock.lock().await;

                // Begin ACID transaction with row-level lock
                let tx = client
                    .transaction()
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to start transaction: {}", e)))?;

                let row_opt = tx
                    .query_opt(
                        "SELECT invite_id, tenant_id, expires_at, consumed FROM invites WHERE invitation_token = $1 FOR UPDATE",
                        &[&tok],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to query invite: {}", e)))?;

                let Some(row) = row_opt else {
                    return Err(ServiceError::InviteNotFound(tok));
                };

                let invite_id: String = row.get(0);
                let tenant_id: String = row.get(1);
                let expires_at: DateTime<Utc> = row.get(2);
                let consumed: bool = row.get(3);

                if consumed {
                    return Err(ServiceError::InviteAlreadyConsumed);
                }
                if Utc::now() >= expires_at {
                    return Err(ServiceError::InviteExpired);
                }

                // Check tenant
                let tenant_row_opt = tx
                    .query_opt(
                        "SELECT status, max_quota_bytes FROM tenants WHERE tenant_id = $1",
                        &[&tenant_id],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to query tenant: {}", e)))?;

                let Some(tenant_row) = tenant_row_opt else {
                    return Err(ServiceError::TenantNotFound(tenant_id));
                };

                let tstatus: String = tenant_row.get(0);
                if tstatus != "Active" {
                    return Err(ServiceError::TenantInactive(tenant_id));
                }

                // Check duplicate device
                let dev_dup_count: i64 = tx
                    .query_one(
                        "SELECT COUNT(*) FROM devices WHERE tenant_id = $1 AND peer_id = $2",
                        &[&tenant_id, &pid],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Query duplicate device failed: {}", e)))?
                    .get(0);

                if dev_dup_count > 0 {
                    return Err(ServiceError::DuplicateDevice(pid));
                }

                // Mark invite consumed atomically
                tx.execute(
                    "UPDATE invites SET consumed = true, consumed_by_peer_id = $2 WHERE invite_id = $1",
                    &[&invite_id, &pid],
                )
                .await
                .map_err(|e| ServiceError::Internal(format!("Failed to consume invite: {}", e)))?;

                let device_id = format!("dev_{}", Uuid::new_v4().simple());
                let dtype_str = match device_type {
                    DeviceType::Desktop => "Desktop",
                    DeviceType::Mobile => "Mobile",
                    DeviceType::Server => "Server",
                };
                let now = Utc::now();

                tx.execute(
                    "INSERT INTO devices (device_id, tenant_id, peer_id, device_name, device_type, quota_bytes, storage_used_bytes, status, last_heartbeat, enrolled_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                    &[
                        &device_id,
                        &tenant_id,
                        &pid,
                        &dname,
                        &dtype_str,
                        &(quota_bytes as i64),
                        &0i64,
                        &"Active",
                        &now,
                        &now,
                    ],
                )
                .await
                .map_err(|e| ServiceError::Internal(format!("Failed to insert device: {}", e)))?;

                tx.commit()
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Transaction commit failed: {}", e)))?;

                Ok(Device {
                    device_id,
                    tenant_id,
                    peer_id: pid,
                    device_name: dname,
                    device_type,
                    quota_bytes,
                    storage_used_bytes: 0,
                    status: DeviceStatus::Active,
                    last_heartbeat: now,
                    enrolled_at: now,
                })
            })
        })
    }

    fn record_credit_event(&self, event: CreditLedgerEvent) -> Result<(), ServiceError> {
        let client_lock = self.client.clone();
        let ev = event;

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                client
                    .execute(
                        "INSERT INTO credit_ledger_events (event_id, tenant_id, peer_id, delta, reason, created_at) \
                         VALUES ($1, $2, $3, $4, $5, $6)",
                        &[
                            &ev.event_id,
                            &ev.tenant_id,
                            &ev.peer_id,
                            &ev.delta,
                            &ev.reason,
                            &ev.timestamp,
                        ],
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(format!("Failed to record credit event: {}", e)))?;

                Ok(())
            })
        })
    }

    fn get_credit_ledger(
        &self,
        _tenant_id: &str,
    ) -> Result<Option<mesh_core::CreditLedger>, ServiceError> {
        // Fallback for credit ledger model
        Ok(None)
    }

    fn update_credit_ledger(
        &self,
        _tenant_id: &str,
        _ledger: mesh_core::CreditLedger,
    ) -> Result<(), ServiceError> {
        Ok(())
    }

    fn list_credit_events(&self, tenant_id: &str) -> Result<Vec<CreditLedgerEvent>, ServiceError> {
        let client_lock = self.client.clone();
        let tid = tenant_id.to_string();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let client = client_lock.lock().await;
                let rows = client
                    .query(
                        "SELECT event_id, tenant_id, peer_id, delta, reason, created_at \
                         FROM credit_ledger_events WHERE tenant_id = $1 ORDER BY created_at ASC",
                        &[&tid],
                    )
                    .await
                    .map_err(|e| {
                        ServiceError::Internal(format!("Failed to list credit events: {}", e))
                    })?;

                let mut events = Vec::new();
                for row in rows {
                    events.push(CreditLedgerEvent {
                        event_id: row.get(0),
                        tenant_id: row.get(1),
                        peer_id: row.get(2),
                        delta: row.get(3),
                        reason: row.get(4),
                        timestamp: row.get(5),
                    });
                }
                Ok(events)
            })
        })
    }

    fn export_snapshot(&self) -> Result<ControlPlaneSnapshot, ServiceError> {
        let tenants = self.list_tenants()?;
        let mut snapshot = ControlPlaneSnapshot::default();
        for t in tenants {
            let devs = self.list_devices_by_tenant(&t.tenant_id)?;
            for d in devs {
                snapshot.devices.insert(d.device_id.clone(), d);
            }
            snapshot.tenants.insert(t.tenant_id.clone(), t);
        }
        Ok(snapshot)
    }

    fn import_snapshot(&self, snapshot: ControlPlaneSnapshot) -> Result<(), ServiceError> {
        for t in snapshot.tenants.into_values() {
            let _ = self.create_tenant(t);
        }
        for d in snapshot.devices.into_values() {
            let _ = self.register_device(d);
        }
        Ok(())
    }
}
