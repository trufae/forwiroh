// Canary written against the Node API at 93883c1; keep its old calls unchanged.
use forwiroh::{Node, TransportMode, control::Command, store::Policy};

#[tokio::test]
async fn legacy_node_consumer() {
    let directory = tempfile::tempdir().unwrap();
    let core = Node::open(
        directory.path(),
        "legacy Rust",
        None,
        TransportMode::LocalOnly,
    )
    .await
    .unwrap();
    let _events = core.subscribe();
    let policy = Policy {
        restricted: true,
        services: Default::default(),
        socks_networks: vec![],
        socks_ports: vec![443],
        reverse_ports: vec![],
        vpn: None,
    };
    core.set_pairing_policy(policy).unwrap();
    assert_eq!(core.snapshot()["id"], core.endpoint.id().to_string());
    assert_eq!(
        core.command(Command::Status).await.unwrap()["name"],
        "legacy Rust"
    );
    let invite = core.invitation().unwrap();
    assert!(invite.uri().unwrap().starts_with("forwiroh:"));
    core.cancel_invitation(&invite.id);
    core.pair_cancel();
    let listen = "127.0.0.1:0".parse().unwrap();
    assert!(core.forward("missing", listen, "ssh".into()).await.is_err());
    assert!(core.reverse_start("missing", 8080, listen).await.is_err());
    assert!(core.socks_start("missing", listen).await.is_err());
    assert!(core.vpn_start("missing", vec![]).await.is_err());
    core.forward_stop("missing").unwrap();
    core.reverse_stop("missing").unwrap();
    core.socks_stop("missing").unwrap();
    core.vpn_stop("missing").unwrap();
    core.shutdown().await;
}
