use forwiroh::{pairing::Invitation, store::Store, wire};
use iroh::{EndpointAddr, SecretKey};
use tokio::io::AsyncWriteExt;

#[test]
fn saved_policies_default_to_open_paired_access_and_can_enable_restrictions() {
    let legacy: forwiroh::store::Policy =
        serde_json::from_value(serde_json::json!({"services":{"ssh":"127.0.0.1:22"}})).unwrap();
    assert!(!legacy.restricted);
    legacy.validate().unwrap();
    let restricted: forwiroh::store::Policy = serde_json::from_value(
        serde_json::json!({"restricted":true,"services":{"ssh":"127.0.0.1:22"}}),
    )
    .unwrap();
    restricted.validate().unwrap();
    let saved = serde_json::to_vec(&restricted).unwrap();
    let reloaded: forwiroh::store::Policy = serde_json::from_slice(&saved).unwrap();
    assert!(reloaded.restricted);
}

#[test]
fn identity_is_persistent_and_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), "home", None).unwrap();
    let id = store.key.public();
    assert!(Store::open(dir.path(), "other", None).is_err());
    drop(store);
    let store = Store::open(dir.path(), "other", None).unwrap();
    assert_eq!(id, store.key.public());
    assert_eq!(store.state.name, "home");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(dir.path().join("identity"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
#[test]
fn external_key_never_written_to_identity_file() {
    let dir = tempfile::tempdir().unwrap();
    let key = [17; 32];
    let store = Store::open(dir.path(), "phone", Some(key)).unwrap();
    assert_eq!(store.key.public(), SecretKey::from_bytes(&key).public());
    assert!(!dir.path().join("identity").exists());
}
#[test]
fn invitations_validate_expiry_version_and_token_sizes() {
    let addr = EndpointAddr::new(SecretKey::generate().public());
    let mut invite = Invitation::new(addr, "home".into()).unwrap();
    assert!(Invitation::parse(&invite.uri().unwrap()).is_ok());
    invite.expires = 0;
    assert!(Invitation::parse(&invite.uri().unwrap()).is_err());
    invite.expires = forwiroh::pairing::now() + 300;
    invite.secret = "short".into();
    assert!(Invitation::parse(&invite.uri().unwrap()).is_err());
    invite.version = 2;
    assert!(Invitation::parse(&invite.uri().unwrap()).is_err());
    assert!(Invitation::parse(&"x".repeat(5000)).is_err());
}
#[tokio::test]
async fn wire_rejects_oversize_before_allocating_and_unknown_versions() {
    for prefix in [[1, 0xff, 0xff, 0xff, 0xff], [2, 0, 0, 0, 0]] {
        let (mut writer, mut reader) = tokio::io::duplex(16);
        writer.write_all(&prefix).await.unwrap();
        assert!(wire::read::<serde_json::Value>(&mut reader).await.is_err());
    }
    let (mut writer, mut reader) = tokio::io::duplex(128);
    wire::write(&mut writer, &serde_json::json!({"a":1}))
        .await
        .unwrap();
    assert_eq!(
        wire::read::<serde_json::Value>(&mut reader).await.unwrap()["a"],
        1
    );
}
