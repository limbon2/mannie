//! TLS certificate inspection, leaf SHA-256 pinning (self-signed and cluster
//! CA-signed where Issuer != Subject), cert rotation mismatch protection before
//! credential transmission, expiry checks, endpoint edits, and TFA refusal.

use mannie_plugin_sdk::testing::FakeSecrets;
use serde_json::json;

use super::support::{
    FakeProxmoxServer, PASSWORD, USERNAME, expired_cert, hosted_plugin, register_pinned_server,
    valid_cluster_ca_signed_cert, valid_self_signed_cert,
};

#[tokio::test]
async fn untrusted_cert_blocks_credentials_until_inspected_and_pinned() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, false).await;
    let cert = valid_self_signed_cert(10);
    let expected_sha = cert.sha256.clone();
    let fake = FakeProxmoxServer::start(cert).await;

    let added = hosted
        .invoke(
            "add_server",
            json!({
                "name": "UntrustedLab",
                "host": "127.0.0.1",
                "port": fake.port(),
                "username": USERNAME,
                "password": PASSWORD,
            }),
        )
        .await
        .expect("add_server");
    let server_id = added["id"].as_str().unwrap();

    // Without OS trust or a pin, test_connection fails closed BEFORE sending password.
    let err = hosted
        .invoke("test_connection", json!({ "serverId": server_id }))
        .await
        .expect_err("untrusted cert refused");
    assert_eq!(
        err.code(),
        "proxmox.certificate_untrusted",
        "actual error: {err:?}"
    );
    assert_eq!(
        fake.auth_attempts(),
        0,
        "no credentials sent to untrusted cert"
    );

    // Inspecting the certificate sends no credentials and returns parsed metadata.
    let info = hosted
        .invoke("inspect_certificate", json!({ "serverId": server_id }))
        .await
        .expect("inspect_certificate");
    assert_eq!(info["sha256"], expected_sha);
    assert_eq!(info["validNow"], true);
    assert_eq!(info["osTrusted"], false);
    assert!(info["subject"].as_str().unwrap().contains("pve.local"));
    assert_eq!(fake.auth_attempts(), 0, "inspection sends zero credentials");

    // Pinning with a wrong fingerprint fails with pin_mismatch.
    let wrong_pin = hosted
        .invoke(
            "pin_certificate",
            json!({ "serverId": server_id, "sha256": "aa".repeat(32) }),
        )
        .await
        .expect_err("wrong pin fails");
    assert_eq!(wrong_pin.code(), "proxmox.pin_mismatch");

    // Pinning the inspected leaf SHA-256 succeeds and enables login.
    let pinned = hosted
        .invoke(
            "pin_certificate",
            json!({ "serverId": server_id, "sha256": expected_sha }),
        )
        .await
        .expect("pin_certificate");
    assert_eq!(pinned["pinnedCertSha256"], expected_sha);

    let test = hosted
        .invoke("test_connection", json!({ "serverId": server_id }))
        .await
        .expect("test_connection after pin");
    assert_eq!(test["visibleVmCount"], 3);
    assert_eq!(test["allowedVmCount"], 0);
    assert_eq!(fake.auth_attempts(), 1);
}

#[tokio::test]
async fn cluster_ca_signed_leaf_certificate_pins_cleanly_even_when_issuer_differs() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, false).await;
    let cert = valid_cluster_ca_signed_cert(11);
    let fake = FakeProxmoxServer::start(cert).await;

    let added = hosted
        .invoke(
            "add_server",
            json!({
                "name": "ClusterCA",
                "host": "127.0.0.1",
                "port": fake.port(),
                "username": USERNAME,
                "password": PASSWORD,
            }),
        )
        .await
        .expect("add_server");
    let server_id = added["id"].as_str().unwrap();

    let info = hosted
        .invoke("inspect_certificate", json!({ "serverId": server_id }))
        .await
        .expect("inspect_certificate");
    assert_ne!(info["subject"], info["issuer"]);
    assert!(
        info["issuer"]
            .as_str()
            .unwrap()
            .contains("Proxmox Virtual Environment Cluster CA")
    );
    assert_eq!(info["validNow"], true);

    hosted
        .invoke(
            "pin_certificate",
            json!({ "serverId": server_id, "sha256": info["sha256"] }),
        )
        .await
        .expect("pin cluster-CA-signed leaf");

    let test = hosted
        .invoke("test_connection", json!({ "serverId": server_id }))
        .await
        .expect("connect using pinned cluster-CA-signed leaf");
    assert_eq!(test["visibleVmCount"], 3);
}

#[tokio::test]
async fn rotated_certificate_mismatch_blocks_credentials_and_clear_pin_restores_os_trust() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, false).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(12)).await;
    let server = register_pinned_server(&hosted, "RotateLab", &fake, &[100]).await;
    let server_id = server["id"].as_str().unwrap();
    let auth_before_rotation = fake.auth_attempts();

    // Rotate the server certificate to a new keypair/cert.
    let rotated = valid_self_signed_cert(13);
    let new_sha = rotated.sha256.clone();
    fake.rotate_cert(rotated);

    // Every API call fails with certificate_mismatch BEFORE any credentials are sent.
    let err = hosted
        .invoke("discover_server_vms", json!({ "serverId": server_id }))
        .await
        .expect_err("rotated cert rejected");
    assert_eq!(
        err.code(),
        "proxmox.certificate_mismatch",
        "actual error: {err:?}"
    );
    assert_eq!(
        fake.auth_attempts(),
        auth_before_rotation,
        "no credentials sent on rotated cert mismatch"
    );

    // Inspecting shows matchesPin == false.
    let info = hosted
        .invoke("inspect_certificate", json!({ "serverId": server_id }))
        .await
        .expect("inspect rotated cert");
    assert_eq!(info["sha256"], new_sha);
    assert_eq!(info["matchesPin"], false);

    // Clearing the pin returns to OS trust (so untrusted self-signed fails with certificate_untrusted).
    let cleared = hosted
        .invoke("clear_certificate_pin", json!({ "serverId": server_id }))
        .await
        .expect("clear_certificate_pin");
    assert!(cleared["pinnedCertSha256"].is_null());

    let untrusted = hosted
        .invoke("test_connection", json!({ "serverId": server_id }))
        .await
        .expect_err("after clearing pin, untrusted cert fails");
    assert_eq!(untrusted.code(), "proxmox.certificate_untrusted");
}

#[tokio::test]
async fn expired_certificate_is_rejected_and_cannot_be_pinned() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, false).await;
    let cert = expired_cert(14);
    let sha = cert.sha256.clone();
    let fake = FakeProxmoxServer::start(cert).await;

    let added = hosted
        .invoke(
            "add_server",
            json!({
                "name": "ExpiredLab",
                "host": "127.0.0.1",
                "port": fake.port(),
                "username": USERNAME,
                "password": PASSWORD,
            }),
        )
        .await
        .expect("add_server");
    let server_id = added["id"].as_str().unwrap();

    let info = hosted
        .invoke("inspect_certificate", json!({ "serverId": server_id }))
        .await
        .expect("inspect expired cert");
    assert_eq!(info["validNow"], false);

    let err = hosted
        .invoke(
            "pin_certificate",
            json!({ "serverId": server_id, "sha256": sha }),
        )
        .await
        .expect_err("pinning expired cert fails");
    assert_eq!(err.code(), "proxmox.certificate_expired");
}

#[tokio::test]
async fn changing_host_or_port_clears_pinned_certificate_and_allowed_vms() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, false).await;
    let fake_a = FakeProxmoxServer::start(valid_self_signed_cert(15)).await;
    let fake_b = FakeProxmoxServer::start(valid_self_signed_cert(16)).await;

    let server = register_pinned_server(&hosted, "MovableLab", &fake_a, &[100, 101]).await;
    let server_id = server["id"].as_str().unwrap();
    assert_eq!(server["allowedVms"].as_array().unwrap().len(), 2);
    assert!(server["pinnedCertSha256"].is_string());

    // Changing only note keeps both pin and allowed VMs.
    let same_endpoint = hosted
        .invoke(
            "update_server",
            json!({
                "serverId": server_id,
                "name": "MovableLab",
                "host": "127.0.0.1",
                "port": fake_a.port(),
                "username": USERNAME,
                "note": "Updated note only",
            }),
        )
        .await
        .expect("update note");
    assert_eq!(same_endpoint["allowedVms"].as_array().unwrap().len(), 2);
    assert_eq!(same_endpoint["pinnedCertSha256"], fake_a.sha256());

    // Changing port clears BOTH pinned TLS fingerprint AND VM allowlist.
    let moved = hosted
        .invoke(
            "update_server",
            json!({
                "serverId": server_id,
                "name": "MovableLab",
                "host": "127.0.0.1",
                "port": fake_b.port(),
                "username": USERNAME,
                "note": "Moved to server B",
            }),
        )
        .await
        .expect("update port");
    assert!(moved["pinnedCertSha256"].is_null());
    assert_eq!(moved["allowedVms"].as_array().unwrap().len(), 0);
    assert_eq!(moved["status"], "no_vms_allowed");
}

#[tokio::test]
async fn tfa_account_returns_tfa_unsupported() {
    let secrets = FakeSecrets::new();
    let hosted = hosted_plugin(&secrets, false).await;
    let fake = FakeProxmoxServer::start(valid_self_signed_cert(17)).await;
    fake.state.lock().unwrap().tfa_required = true;

    let server = register_pinned_server(&hosted, "TfaLab", &fake, &[]).await;
    let server_id = server["id"].as_str().unwrap();

    let err = hosted
        .invoke("test_connection", json!({ "serverId": server_id }))
        .await
        .expect_err("TFA account refused");
    assert_eq!(err.code(), "proxmox.tfa_unsupported");
}
