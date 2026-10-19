mod common;
use common::*;
use fast_socks5::client::{Config, Socks5Stream};
use forwiroh::store::Policy;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::test]
async fn socks_connect_remote_dns_and_destination_denial() {
    let (_a, _b, home, phone) = nodes().await;
    let home_id = pair(&home, &phone).await;
    let (addr, echo) = echo().await;
    let (ipv6, echo6) = echo_on("[::1]:0").await;
    let id = phone
        .socks(&home_id, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let listen = local_listener(&phone, &id);
    let socket = Socks5Stream::connect(listen, "localhost".into(), addr.port(), Config::default())
        .await
        .unwrap();
    drop(socket);
    home.set_policy(
        &phone.endpoint.id().to_string(),
        Policy {
            restricted: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        Socks5Stream::connect(listen, "localhost".into(), addr.port(), Config::default())
            .await
            .is_err()
    );
    let policy = Policy {
        restricted: true,
        socks_networks: vec!["127.0.0.1/32".parse().unwrap(), "::1/128".parse().unwrap()],
        socks_ports: vec![addr.port(), ipv6.port()],
        ..Default::default()
    };
    home.set_policy(&phone.endpoint.id().to_string(), policy)
        .unwrap();
    for (host, port) in [
        ("localhost", addr.port()),
        ("127.0.0.1", addr.port()),
        ("::1", ipv6.port()),
    ] {
        let mut socket = Socks5Stream::connect(listen, host.into(), port, Config::default())
            .await
            .unwrap();
        socket.write_all(b"SOCKS via QUIC").await.unwrap();
        socket.shutdown().await.unwrap();
        let mut received = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut received))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, b"SOCKS via QUIC");
    }
    assert!(
        Socks5Stream::connect(
            listen,
            "169.254.169.254".into(),
            addr.port(),
            Config::default()
        )
        .await
        .is_err()
    );
    assert!(
        Socks5Stream::connect(listen, "127.0.0.1".into(), 1, Config::default())
            .await
            .is_err()
    );
    phone.shutdown().await;
    home.shutdown().await;
    echo.abort();
    echo6.abort();
}
