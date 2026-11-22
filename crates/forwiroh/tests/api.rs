use forwiroh::{ErrorCode, Forwiroh, Options, TransportMode};
use serde_json::json;

#[test]
fn opaque_ids_and_legacy_vpn_responses_keep_their_wire_shapes() {
    let id = forwiroh::TunnelId::from_raw("opaque");
    assert_eq!(serde_json::to_value(&id).unwrap(), json!("opaque"));
    let mut value = json!({"id":"session","local_ip":"10.250.0.2","remote_ip":"10.250.0.1",
        "local_ipv6":"fd00:fa::2","remote_ipv6":"fd00:fa::1","forward_ip":"10.250.0.3","routes":[],"mtu":1280});
    let old: forwiroh::VpnSession = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(old.session_id.as_str(), "session");
    assert!(old.tunnel_id.is_none());
    value["session_id"] = json!("session");
    value["tunnel_id"] = json!("tunnel");
    let new: forwiroh::VpnSession = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(new.tunnel_id.unwrap().as_str(), "tunnel");
    value["session_id"] = json!("conflict");
    assert!(serde_json::from_value::<forwiroh::VpnSession>(value).is_err());
}

#[tokio::test]
async fn supported_facade_opens_commands_and_closes_without_transport_types() {
    let path = tempfile::tempdir().unwrap();
    let core = Forwiroh::open_with_options(
        path.path(),
        "Rust consumer",
        Options::default().transport(TransportMode::LocalOnly),
    )
    .await
    .unwrap();
    let status = core.command(json!({"op":"status"})).await.unwrap();
    assert_eq!(status["id"], core.id());
    assert_eq!(status["name"], "Rust consumer");
    let info = core.api_info().await.unwrap();
    assert_eq!(info.control_api_version, forwiroh::CONTROL_API_VERSION);
    assert_eq!(info.c_abi_version, forwiroh::C_ABI_VERSION);
    assert_eq!(info.max_packet_size, forwiroh::MAX_PACKET_SIZE);
    for command in ["forward", "forward_start", "pair", "pair_start", "stop"] {
        assert!(info.supports(command));
    }
    assert!(!info.supports("unknown"));
    assert!(info.backends.iter().any(|b| b == "vpn_packet"));
    assert_eq!(
        core.command(json!({"op":"stop", "id":"missing"}))
            .await
            .unwrap(),
        serde_json::Value::Null
    );
    assert_eq!(
        core.command(json!({"op":"not_a_command"}))
            .await
            .unwrap_err()
            .code(),
        ErrorCode::InvalidCommand
    );
    assert_eq!(
        core.receive_packet(&forwiroh::VpnSessionId::from_raw("missing"))
            .await
            .unwrap_err()
            .code(),
        ErrorCode::SessionNotFound
    );
    core.close().await;
    core.close().await;
    assert!(core.next_event().await.is_none());
    assert_eq!(
        core.command(json!({"op":"status"}))
            .await
            .unwrap_err()
            .code(),
        ErrorCode::CoreStopped
    );
}

#[tokio::test]
async fn dropping_the_facade_cancels_its_background_ownership() {
    let path = tempfile::tempdir().unwrap();
    let options = Options::default().transport(TransportMode::LocalOnly);
    let core = Forwiroh::open_with_options(path.path(), "owner", options.clone())
        .await
        .unwrap();
    drop(core);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match Forwiroh::open_with_options(path.path(), "next", options.clone()).await {
                Ok(core) => {
                    core.close().await;
                    break;
                }
                Err(error) => {
                    assert!(
                        error.to_string().contains("state is already in use"),
                        "{error}"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_tunnel_and_vpn_ids_address_distinct_resources() {
    use std::{sync::Arc, time::Duration};
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let options = Options::default().transport(TransportMode::LocalOnly);
    let server = Forwiroh::open_with_options(a.path(), "server", options.clone())
        .await
        .unwrap();
    let client = Arc::new(
        Forwiroh::open_with_options(b.path(), "client", options)
            .await
            .unwrap(),
    );
    let invitation = server.command(json!({"op":"invite_create"})).await.unwrap();
    let other = client.clone();
    let pairing = tokio::spawn(async move {
        other
            .command(json!({"op":"pair","uri":invitation["uri"],"device":"Rust"}))
            .await
    });
    let pending = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let pending = server.command(json!({"op":"peer_pending"})).await.unwrap();
            if let Some(first) = pending.as_array().unwrap().first() {
                break first.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    server
        .command(json!({"op":"peer_approve","id":pending["id"],"accept":true}))
        .await
        .unwrap();
    pairing.await.unwrap().unwrap();
    let forward = client
        .forward(&server.id(), "127.0.0.1:0".parse().unwrap(), "127.0.0.1:22")
        .await
        .unwrap();
    let session = client
        .vpn_start(&server.id(), &["198.18.0.0/16".into()])
        .await
        .unwrap();
    let tunnel_id = session.tunnel_id.as_ref().unwrap();
    assert_ne!(tunnel_id.as_str(), session.session_id.as_str());
    let status = client.command(json!({"op":"status"})).await.unwrap();
    let tunnel = status["tunnels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == tunnel_id.as_str())
        .unwrap();
    assert_eq!(tunnel["session_id"], session.session_id.as_str());
    client.vpn_stop(&session.session_id).await.unwrap();
    client.vpn_stop(&session.session_id).await.unwrap();
    client.tunnel_stop(&forward).await.unwrap();
    client.tunnel_stop(&forward).await.unwrap();
    client.close().await;
    server.close().await;
}
