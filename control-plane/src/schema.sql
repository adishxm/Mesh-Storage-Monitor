-- Mesh Storage Multi-Tenant SaaS PostgreSQL Schema (REQ-17)
-- Complete tenant isolation, device registry, invite queue, and audit trail.

CREATE TABLE IF NOT EXISTS tenants (
    tenant_id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(128) NOT NULL UNIQUE,
    max_quota_bytes BIGINT NOT NULL DEFAULT 107374182400, -- 100 GB default
    current_used_bytes BIGINT NOT NULL DEFAULT 0,
    status VARCHAR(32) NOT NULL DEFAULT 'Active',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS devices (
    device_id VARCHAR(64) PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    peer_id VARCHAR(128) NOT NULL,
    device_name VARCHAR(255) NOT NULL,
    device_type VARCHAR(32) NOT NULL, -- 'Desktop', 'Mobile', 'Server'
    quota_bytes BIGINT NOT NULL DEFAULT 10737418240, -- 10 GB
    storage_used_bytes BIGINT NOT NULL DEFAULT 0,
    status VARCHAR(32) NOT NULL DEFAULT 'Active', -- 'Active', 'Suspended', 'Revoked'
    last_heartbeat TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    enrolled_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT uq_tenant_peer UNIQUE (tenant_id, peer_id)
);

CREATE INDEX IF NOT EXISTS idx_devices_tenant_id ON devices(tenant_id);
CREATE INDEX IF NOT EXISTS idx_devices_peer_id ON devices(peer_id);

CREATE TABLE IF NOT EXISTS invites (
    invite_id VARCHAR(64) PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    created_by_sub VARCHAR(255) NOT NULL,
    invitation_token VARCHAR(255) NOT NULL UNIQUE,
    single_use_nonce VARCHAR(64) NOT NULL UNIQUE,
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL,
    consumed BOOLEAN NOT NULL DEFAULT FALSE,
    consumed_by_peer_id VARCHAR(128),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_invites_tenant_id ON invites(tenant_id);
CREATE INDEX IF NOT EXISTS idx_invites_token ON invites(invitation_token);

CREATE TABLE IF NOT EXISTS audit_logs (
    log_id BIGSERIAL PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL REFERENCES tenants(tenant_id) ON DELETE CASCADE,
    actor_sub VARCHAR(255) NOT NULL,
    action VARCHAR(128) NOT NULL,
    details JSONB,
    timestamp TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_tenant_id ON audit_logs(tenant_id);
