use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use http_body_util::BodyExt;
use mesh_control_plane::{
    ControlPlaneService, DeviceStatus, DeviceType, OidcClaims, ServiceError, make_router,
};
use tower::ServiceExt;

fn make_claims(sub: &str, tenant_id: &str, role: &str) -> OidcClaims {
    OidcClaims {
        sub: sub.to_string(),
        email: format!("{}@{}.com", sub, tenant_id),
        tenant_id: tenant_id.to_string(),
        roles: vec![role.to_string()],
        exp: Utc::now().timestamp() + 3600,
    }
}

#[tokio::test]
async fn test_strict_tenant_isolation_boundary() {
    let service = ControlPlaneService::new();
    let superadmin = make_claims("admin", "system", "superadmin");

    // 1. Create two distinct tenants
    let tenant_a = service
        .create_tenant(
            &superadmin,
            "Tenant Alpha".into(),
            "alpha".into(),
            100_000_000,
        )
        .await
        .expect("Tenant A created");

    let tenant_b = service
        .create_tenant(
            &superadmin,
            "Tenant Beta".into(),
            "beta".into(),
            100_000_000,
        )
        .await
        .expect("Tenant B created");

    let user_a = make_claims("alice", &tenant_a.tenant_id, "admin");
    let user_b = make_claims("bob", &tenant_b.tenant_id, "admin");

    // 2. User A registers device in Tenant A
    let dev_a = service
        .register_device(
            &user_a,
            &tenant_a.tenant_id,
            "peer-alice-desktop",
            "Alice Laptop",
            DeviceType::Desktop,
            10_000_000,
        )
        .await
        .expect("Register dev in Tenant A");

    // 3. User B registers device in Tenant B
    let dev_b = service
        .register_device(
            &user_b,
            &tenant_b.tenant_id,
            "peer-bob-server",
            "Bob Server",
            DeviceType::Server,
            20_000_000,
        )
        .await
        .expect("Register dev in Tenant B");

    // 4. Isolation Checks:
    // User A attempts to list devices in Tenant B -> MUST FAIL with TenantIsolationViolation
    let list_b_result = service.list_devices(&user_a, &tenant_b.tenant_id).await;
    assert_eq!(
        list_b_result.unwrap_err(),
        ServiceError::TenantIsolationViolation {
            requester_tenant: tenant_a.tenant_id.clone(),
            target_tenant: tenant_b.tenant_id.clone(),
        }
    );

    // User A attempts to access Tenant B directly -> MUST FAIL
    let get_b_result = service.get_tenant(&user_a, &tenant_b.tenant_id).await;
    assert_eq!(
        get_b_result.unwrap_err(),
        ServiceError::TenantIsolationViolation {
            requester_tenant: tenant_a.tenant_id.clone(),
            target_tenant: tenant_b.tenant_id.clone(),
        }
    );

    // User A attempts to modify Device B's status -> MUST FAIL
    let status_b_result = service
        .update_device_status(
            &user_a,
            &tenant_b.tenant_id,
            &dev_b.device_id,
            DeviceStatus::Revoked,
        )
        .await;
    assert_eq!(
        status_b_result.unwrap_err(),
        ServiceError::TenantIsolationViolation {
            requester_tenant: tenant_a.tenant_id.clone(),
            target_tenant: tenant_b.tenant_id.clone(),
        }
    );

    // Legitimate accesses succeed
    let list_a = service
        .list_devices(&user_a, &tenant_a.tenant_id)
        .await
        .expect("List A");
    assert_eq!(list_a.len(), 1);
    assert_eq!(list_a[0].device_id, dev_a.device_id);

    let list_b = service
        .list_devices(&user_b, &tenant_b.tenant_id)
        .await
        .expect("List B");
    assert_eq!(list_b.len(), 1);
    assert_eq!(list_b[0].device_id, dev_b.device_id);
}

#[tokio::test]
async fn test_aggregate_tenant_quota_enforcement() {
    let service = ControlPlaneService::new();
    let superadmin = make_claims("admin", "system", "superadmin");

    // Tenant with 10 GB limit
    let quota_10gb = 10 * 1024 * 1024 * 1024;
    let tenant = service
        .create_tenant(
            &superadmin,
            "Capped Org".into(),
            "capped".into(),
            quota_10gb,
        )
        .await
        .expect("Tenant created");

    let user = make_claims("lead", &tenant.tenant_id, "admin");

    // Device 1: 6 GB -> Success
    service
        .register_device(
            &user,
            &tenant.tenant_id,
            "peer-1",
            "Node 1",
            DeviceType::Server,
            6 * 1024 * 1024 * 1024,
        )
        .await
        .expect("Device 1 registered");

    // Device 2: 3 GB -> Success (Total requested = 9 GB <= 10 GB)
    service
        .register_device(
            &user,
            &tenant.tenant_id,
            "peer-2",
            "Node 2",
            DeviceType::Desktop,
            3 * 1024 * 1024 * 1024,
        )
        .await
        .expect("Device 2 registered");

    // Device 3: 2 GB -> Should FAIL because 9 + 2 = 11 GB > 10 GB
    let overflow_res = service
        .register_device(
            &user,
            &tenant.tenant_id,
            "peer-3",
            "Node 3",
            DeviceType::Mobile,
            2 * 1024 * 1024 * 1024,
        )
        .await;

    assert!(matches!(
        overflow_res.unwrap_err(),
        ServiceError::QuotaExceeded { .. }
    ));
}

#[tokio::test]
async fn test_device_heartbeat_and_usage_propagation() {
    let service = ControlPlaneService::new();
    let superadmin = make_claims("admin", "system", "superadmin");

    let tenant = service
        .create_tenant(&superadmin, "Health Org".into(), "health".into(), 1_000_000)
        .await
        .expect("Tenant created");

    let admin = make_claims("admin", &tenant.tenant_id, "admin");

    let dev = service
        .register_device(
            &admin,
            &tenant.tenant_id,
            "peer-hb-1",
            "Health Node",
            DeviceType::Server,
            500_000,
        )
        .await
        .expect("Device registered");

    // Send heartbeat reporting 150_000 bytes stored
    service
        .update_device_heartbeat(&tenant.tenant_id, &dev.device_id, 150_000)
        .await
        .expect("Heartbeat update");

    let metrics = service
        .get_tenant_metrics(&admin, &tenant.tenant_id)
        .await
        .expect("Get metrics");
    assert_eq!(metrics.total_used_bytes, 150_000);
    assert_eq!(metrics.online_devices, 1);
    assert_eq!(metrics.server_devices, 1);

    // Update with 300_000 bytes
    service
        .update_device_heartbeat(&tenant.tenant_id, &dev.device_id, 300_000)
        .await
        .expect("Heartbeat update 2");

    let updated_metrics = service
        .get_tenant_metrics(&admin, &tenant.tenant_id)
        .await
        .expect("Get metrics 2");
    assert_eq!(updated_metrics.total_used_bytes, 300_000);
}

#[tokio::test]
async fn test_tenant_invitation_queue_and_single_use() {
    let service = ControlPlaneService::new();
    let superadmin = make_claims("admin", "system", "superadmin");

    let tenant = service
        .create_tenant(
            &superadmin,
            "Invite Org".into(),
            "invites".into(),
            500_000_000,
        )
        .await
        .expect("Tenant created");

    let admin = make_claims("admin", &tenant.tenant_id, "admin");

    // 1. Admin creates 1-hour invitation
    let invite = service
        .create_invite(&admin, &tenant.tenant_id, 3600)
        .await
        .expect("Invite created");

    assert!(!invite.consumed);
    assert!(invite.is_valid());

    // 2. Client node consumes invitation to join
    let enrolled_device = service
        .consume_invite(
            &invite.invitation_token,
            "peer-new-worker",
            "Worker 01",
            DeviceType::Desktop,
            50_000_000,
        )
        .await
        .expect("Enrolled via invite");

    assert_eq!(enrolled_device.tenant_id, tenant.tenant_id);

    // 3. Replay attack: trying to consume the same invite again MUST FAIL
    let replay_res = service
        .consume_invite(
            &invite.invitation_token,
            "peer-imposter",
            "Imposter",
            DeviceType::Desktop,
            50_000_000,
        )
        .await;

    assert_eq!(replay_res.unwrap_err(), ServiceError::InviteAlreadyConsumed);

    // 4. Expired invite test
    let expired_invite = service
        .create_invite(&admin, &tenant.tenant_id, 0)
        .await
        .expect("Expired invite created");

    // Sleep 10ms to guarantee past expiration
    tokio::time::sleep(std::time::Duration::from_millis(15)).await;

    let expired_res = service
        .consume_invite(
            &expired_invite.invitation_token,
            "peer-late",
            "Late Node",
            DeviceType::Desktop,
            10_000,
        )
        .await;

    assert_eq!(expired_res.unwrap_err(), ServiceError::InviteExpired);
}

#[tokio::test]
async fn test_control_plane_axum_http_api() {
    let service = ControlPlaneService::new();
    let app = make_router(service);

    // 1. Create Tenant via HTTP POST
    let create_payload = serde_json::json!({
        "name": "Acme Global",
        "slug": "acme-global",
        "max_quota_bytes": 1000000000
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/control/tenants")
        .header("content-type", "application/json")
        .header("x-oidc-sub", "admin-root")
        .header("x-oidc-tenant", "system")
        .header("x-oidc-roles", "superadmin")
        .body(Body::from(serde_json::to_vec(&create_payload).unwrap()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let tenant_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let tenant_id = tenant_json["tenant_id"].as_str().unwrap().to_string();

    // 2. Query Tenant with matching tenant claims -> HTTP 200 OK
    let get_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/control/tenants/{}", tenant_id))
        .header("x-oidc-sub", "alice")
        .header("x-oidc-tenant", &tenant_id)
        .header("x-oidc-roles", "member")
        .body(Body::empty())
        .unwrap();

    let get_resp = app.clone().oneshot(get_req).await.unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);

    // 3. Query Tenant with FOREIGN tenant claims -> HTTP 403 Forbidden!
    let forbidden_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/control/tenants/{}", tenant_id))
        .header("x-oidc-sub", "eve")
        .header("x-oidc-tenant", "foreign_tenant_xyz")
        .header("x-oidc-roles", "member")
        .body(Body::empty())
        .unwrap();

    let forbidden_resp = app.oneshot(forbidden_req).await.unwrap();
    assert_eq!(forbidden_resp.status(), StatusCode::FORBIDDEN);
}
