//! PostgreSQL Migration Runner and Schema Definitions for Multi-Tenant Control Plane
//! REQ-17: Tenant isolation, transactional invite consumption, credit ledgers, and audit logs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Migration {
    pub version: i32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_initial_schema",
        sql: r#"
-- Core multi-tenant tables
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    applied_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS tenants (
    tenant_id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(128) NOT NULL,
    max_quota_bytes BIGINT NOT NULL DEFAULT 107374182400,
    current_used_bytes BIGINT NOT NULL DEFAULT 0,
    status VARCHAR(32) NOT NULL DEFAULT 'Active',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT uq_tenants_slug UNIQUE (slug)
);

CREATE TABLE IF NOT EXISTS devices (
    device_id VARCHAR(64) PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    peer_id VARCHAR(128) NOT NULL,
    device_name VARCHAR(255) NOT NULL,
    device_type VARCHAR(32) NOT NULL,
    quota_bytes BIGINT NOT NULL DEFAULT 10737418240,
    storage_used_bytes BIGINT NOT NULL DEFAULT 0,
    status VARCHAR(32) NOT NULL DEFAULT 'Active',
    last_heartbeat TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    enrolled_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT uq_tenant_peer UNIQUE (tenant_id, peer_id)
);

CREATE INDEX IF NOT EXISTS idx_devices_tenant_id ON devices(tenant_id);
CREATE INDEX IF NOT EXISTS idx_devices_peer_id ON devices(peer_id);
"#,
    },
    Migration {
        version: 2,
        name: "0002_transactional_invites_and_nonces",
        sql: r#"
CREATE TABLE IF NOT EXISTS invites (
    invite_id VARCHAR(64) PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    created_by_sub VARCHAR(255) NOT NULL,
    invitation_token VARCHAR(255) NOT NULL,
    single_use_nonce VARCHAR(64) NOT NULL,
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL,
    consumed BOOLEAN NOT NULL DEFAULT FALSE,
    consumed_by_peer_id VARCHAR(128),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT uq_invites_token UNIQUE (invitation_token),
    CONSTRAINT uq_invites_nonce UNIQUE (single_use_nonce)
);

CREATE INDEX IF NOT EXISTS idx_invites_tenant_id ON invites(tenant_id);
CREATE INDEX IF NOT EXISTS idx_invites_token ON invites(invitation_token);
"#,
    },
    Migration {
        version: 3,
        name: "0003_credit_ledger_events_and_audit",
        sql: r#"
CREATE TABLE IF NOT EXISTS credit_ledger_events (
    event_id VARCHAR(64) PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    peer_id VARCHAR(128) NOT NULL,
    delta BIGINT NOT NULL,
    reason VARCHAR(255) NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_credit_events_tenant ON credit_ledger_events(tenant_id);
CREATE INDEX IF NOT EXISTS idx_credit_events_peer ON credit_ledger_events(peer_id);

CREATE TABLE IF NOT EXISTS audit_logs (
    log_id BIGSERIAL PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    actor_sub VARCHAR(255) NOT NULL,
    action VARCHAR(128) NOT NULL,
    details JSONB,
    timestamp TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_tenant_id ON audit_logs(tenant_id);
"#,
    },
];

/// Migration plan descriptor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub pending_migrations: Vec<i32>,
    pub applied_migrations: Vec<i32>,
}

pub struct PostgresMigrationRunner;

impl PostgresMigrationRunner {
    /// Returns the full combined SQL script representing the complete initialized schema.
    pub fn get_combined_schema_sql() -> String {
        MIGRATIONS
            .iter()
            .map(|m| format!("-- Migration {}: {}\n{}\n", m.version, m.name, m.sql.trim()))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Validates that migration versions are strictly sequential and non-empty.
    pub fn validate_migrations() -> Result<(), String> {
        for (idx, m) in MIGRATIONS.iter().enumerate() {
            let expected = (idx + 1) as i32;
            if m.version != expected {
                return Err(format!(
                    "Migration version gap detected: expected {}, got {}",
                    expected, m.version
                ));
            }
            if m.sql.trim().is_empty() {
                return Err(format!("Migration {} contains empty SQL", m.name));
            }
        }
        Ok(())
    }

    /// Calculates which migrations need to be applied given a set of already executed versions.
    pub fn plan(applied_versions: &[i32]) -> MigrationPlan {
        let applied_set: std::collections::HashSet<i32> =
            applied_versions.iter().copied().collect();
        let mut pending = Vec::new();
        let mut applied = Vec::new();

        for m in MIGRATIONS {
            if applied_set.contains(&m.version) {
                applied.push(m.version);
            } else {
                pending.push(m.version);
            }
        }

        MigrationPlan {
            pending_migrations: pending,
            applied_migrations: applied,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrations_are_valid_and_sequential() {
        assert!(PostgresMigrationRunner::validate_migrations().is_ok());
    }

    #[test]
    fn test_migration_planner() {
        let plan = PostgresMigrationRunner::plan(&[1]);
        assert_eq!(plan.applied_migrations, vec![1]);
        assert_eq!(plan.pending_migrations, vec![2, 3]);

        let all_applied = PostgresMigrationRunner::plan(&[1, 2, 3]);
        assert!(all_applied.pending_migrations.is_empty());
    }

    #[test]
    fn test_combined_schema_contains_core_tables() {
        let sql = PostgresMigrationRunner::get_combined_schema_sql();
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS tenants"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS devices"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS invites"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS credit_ledger_events"));
        assert!(sql.contains("uq_tenants_slug"));
        assert!(sql.contains("uq_tenant_peer"));
        assert!(sql.contains("uq_invites_token"));
    }
}
