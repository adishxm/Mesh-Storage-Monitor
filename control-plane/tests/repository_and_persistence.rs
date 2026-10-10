use chrono::Utc;
use mesh_control_plane::{ControlPlaneService, DeviceStatus, DeviceType, OidcClaims, ServiceError};
use std::sync::Arc;
use tempfile::tempdir;

fn make_admin_claims(tenant_id: &str) -> OidcClaims {
    OidcClaims {
        sub: "admin-1".to_string(),
        email: "admin@mesh.io".to_string(),
        tenant_id: tenant_id.to_string(),
        roles: vec!["admin".to_string()],
        exp: Utc::now().timestamp() + 3600,
    }
}

#[tokio::test]
async fn test_control_plane_process_restart_persistence() {
    let dir = tempdir().expect("create temp dir");
    let db_path = dir.path().join("control_plane.json");

    let tenant_id;
    let invite_token;
    let device_id;

    // 1. Initial process lifecycle: write state and drop service
    {
        let service =
            ControlPlaneService::new_persistent(&db_path).expect("create persistent control plane");
        let claims = make_admin_claims("ten_system");

        let tenant = service
            .create_tenant(
                &claims,
                "Persistent Enterprise".into(),
                "persistent-ent".into(),
                50_000_000,
            )
            .await
            .expect("create tenant");
        tenant_id = tenant.tenant_id.clone();

        let dev_claims = make_admin_claims(&tenant_id);
        let device = service
            .register_device(
                &dev_claims,
                &tenant_id,
                "peer-persist-node-1",
                "Node 1",
                DeviceType::Server,
                10_000_000,
            )
            .await
            .expect("register device");
        device_id = device.device_id.clone();

        // Update device status to Revoked and test persistence of revocation
        service
            .update_device_status(&dev_claims, &tenant_id, &device_id, DeviceStatus::Revoked)
            .await
            .expect("revoke device");

        let invite = service
            .create_invite(&dev_claims, &tenant_id, 3600)
            .await
            .expect("create invite");
        invite_token = invite.invitation_token.clone();

        // Record credit event
        service
            .record_peer_storage_activity("peer-persist-node-1", 5_000_000, 1_000_000, Some(true))
            .await;

        // Service is dropped here as scope ends
    }

    assert!(db_path.exists(), "persistence file must exist on disk");

    // 2. Simulated process restart: re-instantiate service from disk
    {
        let restarted_service =
            ControlPlaneService::new_persistent(&db_path).expect("reload persistent control plane");
        let dev_claims = make_admin_claims(&tenant_id);

        // Verify tenant restored
        let tenant = restarted_service
            .get_tenant(&dev_claims, &tenant_id)
            .await
            .expect("tenant recovered");
        assert_eq!(tenant.slug, "persistent-ent");
        assert_eq!(tenant.name, "Persistent Enterprise");

        // Verify device and its revocation status restored
        let devices = restarted_service
            .list_devices(&dev_claims, &tenant_id)
            .await
            .expect("list devices");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_id, device_id);
        assert_eq!(devices[0].status, DeviceStatus::Revoked);

        // Verify invite restored and still consumable
        let consumed_device = restarted_service
            .consume_invite(
                &invite_token,
                "peer-persist-node-2",
                "Node 2",
                DeviceType::Desktop,
                5_000_000,
            )
            .await
            .expect("consume recovered invite");
        assert_eq!(consumed_device.peer_id, "peer-persist-node-2");

        // Verify credit report restored
        let credit_report = restarted_service
            .get_peer_credit_report("peer-persist-node-1")
            .await;
        assert_eq!(credit_report.bytes_contributed, 5_000_000);
        assert_eq!(credit_report.bytes_consumed, 1_000_000);
        assert_eq!(credit_report.audits_passed, 1);
    }
}

#[tokio::test]
async fn test_concurrent_invite_consumption_transactional_safety() {
    let service = Arc::new(ControlPlaneService::new());
    let claims = make_admin_claims("ten_system");

    let tenant = service
        .create_tenant(
            &claims,
            "Concurrency Test Tenant".into(),
            "concurrency-tenant".into(),
            100_000_000,
        )
        .await
        .expect("tenant created");

    let invite_claims = make_admin_claims(&tenant.tenant_id);
    let invite = service
        .create_invite(&invite_claims, &tenant.tenant_id, 3600)
        .await
        .expect("invite created");
    let token = Arc::new(invite.invitation_token);

    // Spawn 10 concurrent tasks attempting to consume the EXACT same single-use token simultaneously
    let mut handles = Vec::new();
    for i in 0..10 {
        let svc = Arc::clone(&service);
        let tok = Arc::clone(&token);
        handles.push(tokio::spawn(async move {
            let peer_id = format!("concurrent-peer-{}", i);
            svc.consume_invite(&tok, &peer_id, "Device", DeviceType::Mobile, 1_000_000)
                .await
        }));
    }

    let mut successes = 0;
    let mut already_consumed_errors = 0;

    for handle in handles {
        match handle.await.expect("join handle") {
            Ok(_) => successes += 1,
            Err(ServiceError::InviteAlreadyConsumed) => already_consumed_errors += 1,
            Err(e) => panic!("Unexpected error during concurrent consumption: {:?}", e),
        }
    }

    // Exactly 1 winner, 9 rejected atomically
    assert_eq!(successes, 1, "Exactly ONE concurrent consumer must succeed");
    assert_eq!(
        already_consumed_errors, 9,
        "All other concurrent consumers must receive InviteAlreadyConsumed"
    );

    // Verify tenant only has 1 device registered
    let dev_claims = make_admin_claims(&tenant.tenant_id);
    let devices = service
        .list_devices(&dev_claims, &tenant.tenant_id)
        .await
        .expect("list devices");
    assert_eq!(devices.len(), 1);
}

#[tokio::test]
async fn test_control_plane_backup_and_disaster_recovery() {
    let service = ControlPlaneService::new();
    let claims = make_admin_claims("ten_system");

    let tenant = service
        .create_tenant(
            &claims,
            "Disaster Recovery Corp".into(),
            "dr-corp".into(),
            20_000_000,
        )
        .await
        .expect("create tenant");

    let dev_claims = make_admin_claims(&tenant.tenant_id);
    let device = service
        .register_device(
            &dev_claims,
            &tenant.tenant_id,
            "peer-dr-1",
            "DR Node",
            DeviceType::Server,
            5_000_000,
        )
        .await
        .expect("register device");

    service
        .record_peer_storage_activity("peer-dr-1", 10_000_000, 2_000_000, Some(true))
        .await;

    // Export database backup
    let backup_data = service.export_backup().await.expect("export backup");
    assert!(backup_data.contains("dr-corp"));
    assert!(backup_data.contains("peer-dr-1"));

    // Create fresh empty control-plane service
    let fresh_service = ControlPlaneService::new();
    fresh_service
        .restore_backup(&backup_data)
        .await
        .expect("restore backup");

    // Verify full state recovery
    let restored_tenant = fresh_service
        .get_tenant(&dev_claims, &tenant.tenant_id)
        .await
        .expect("recovered tenant");
    assert_eq!(restored_tenant.slug, "dr-corp");

    let devices = fresh_service
        .list_devices(&dev_claims, &tenant.tenant_id)
        .await
        .expect("recovered devices");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].device_id, device.device_id);

    let credits = fresh_service.get_peer_credit_report("peer-dr-1").await;
    assert_eq!(credits.bytes_contributed, 10_000_000);
}

#[tokio::test]
async fn test_unique_database_constraints() {
    let service = ControlPlaneService::new();
    let claims = make_admin_claims("ten_system");

    service
        .create_tenant(
            &claims,
            "Unique Corp".into(),
            "unique-slug".into(),
            10_000_000,
        )
        .await
        .expect("first tenant created");

    // Duplicate slug must fail
    let duplicate_slug_result = service
        .create_tenant(
            &claims,
            "Other Corp".into(),
            "unique-slug".into(),
            10_000_000,
        )
        .await;
    assert_eq!(
        duplicate_slug_result.unwrap_err(),
        ServiceError::DuplicateSlug("unique-slug".into())
    );
}
