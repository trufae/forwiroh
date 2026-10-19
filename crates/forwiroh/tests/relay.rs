//! External connectivity check, explicitly selected in CI. No direct UDP transport
//! exists on either endpoint, so a successful transfer must use encrypted relay.
use forwiroh::{Node, TransportMode, store::Policy};
use std::time::Duration;
mod common;

#[tokio::test]
#[ignore = "requires access to the public Iroh relay service"]
async fn public_relay_pairs_and_forwards_without_udp() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let home = Node::open(a.path(), "home", None, TransportMode::RelayOnly)
            .await
            .unwrap();
        let phone = Node::open(b.path(), "phone", None, TransportMode::RelayOnly)
            .await
            .unwrap();
        assert!(
            home.endpoint
                .addr()
                .addrs
                .iter()
                .all(|a| matches!(a, iroh::TransportAddr::Relay(_)))
        );
        let id = common::pair(&home, &phone).await;
        let (echo_addr, echo) = common::echo().await;
        let mut policy = Policy {
            restricted: true,
            ..Default::default()
        };
        policy.services.insert("echo".into(), echo_addr.to_string());
        home.set_policy(&phone.endpoint.id().to_string(), policy)
            .unwrap();
        let tunnel = phone
            .forward(&id, "127.0.0.1:0".parse().unwrap(), "echo".into())
            .await
            .unwrap();
        common::roundtrip(common::local_listener(&phone, &tunnel), 73).await;
        assert_eq!(phone.snapshot()["peers"][0]["path"], "relay");
        home.revoke(&phone.endpoint.id().to_string()).unwrap();
        assert!(phone.services(&id).await.is_err());
        phone.shutdown().await;
        home.shutdown().await;
        echo.abort();
    })
    .await
    .expect("public relay check timed out");
}
