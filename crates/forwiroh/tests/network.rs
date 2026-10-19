use forwiroh::{Node, store::Policy};
use serde_json::Value;
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

mod common;
use common::*;

#[tokio::test]
async fn pairing_requires_approval_and_is_single_use() {
    let (_a, _b, home, phone) = nodes().await;
    let invite = home.invitation().unwrap();
    let uri = invite.uri().unwrap();
    let mut events = home.subscribe();
    let p = phone.clone();
    let u = uri.clone();
    let task = tokio::spawn(async move { p.pair(&u, "test").await });
    timeout(Duration::from_secs(10), async {
        loop {
            let e = events.recv().await.unwrap();
            if e["event"] == "pair_request" {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(home.snapshot()["peers"].as_array().unwrap().is_empty());
    home.approve(&invite.id, false).unwrap();
    assert!(task.await.unwrap().is_err());
    assert!(phone.pair(&uri, "test").await.is_err());
    assert!(home.snapshot()["peers"].as_array().unwrap().is_empty());
    pair(&home, &phone).await;
    assert_eq!(
        home.snapshot()["peers"][0]["policy"]["services"],
        serde_json::json!({})
    );
    assert_eq!(home.snapshot()["peers"][0]["policy"]["restricted"], false);
    assert_eq!(phone.snapshot()["peers"][0]["policy"]["restricted"], false);
    phone.shutdown().await;
    home.shutdown().await;
}
#[tokio::test]
async fn pairing_again_requires_approval_and_preserves_permissions() {
    let (_a, _b, home, phone) = nodes().await;
    let home_id = pair(&home, &phone).await;
    let phone_id = phone.endpoint.id().to_string();
    home.rename(&phone_id, "My iPhone").unwrap();
    home.set_policy(
        &phone_id,
        Policy {
            restricted: true,
            ..Default::default()
        },
    )
    .unwrap();
    // The server still knows the identity even if the client lost its peer record.
    phone.revoke(&home_id).unwrap();
    let invite = home.invitation().unwrap();
    let mut forged = invite.clone();
    forged.secret = forwiroh::pairing::random().unwrap();
    assert_eq!(
        phone
            .pair(&forged.uri().unwrap(), "iPhone")
            .await
            .unwrap_err()
            .to_string(),
        "invalid invitation proof"
    );
    assert!(home.pending().as_array().unwrap().is_empty());
    let uri = invite.uri().unwrap();
    let p = phone.clone();
    let u = uri.clone();
    let task = tokio::spawn(async move { p.pair(&u, "iPhone").await });
    timeout(Duration::from_secs(5), async {
        loop {
            if !home.pending().as_array().unwrap().is_empty() {
                break;
            }
            assert!(
                !task.is_finished(),
                "re-pairing closed before showing approval"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!task.is_finished(), "re-pairing must wait for approval");
    home.approve(&invite.id, true).unwrap();
    assert_eq!(task.await.unwrap().unwrap(), home_id);
    let peer = &home.snapshot()["peers"][0];
    assert_eq!(peer["name"], "My iPhone");
    assert_eq!(peer["policy"]["restricted"], true);
    assert_eq!(peer["device"], "iPhone");
    assert_eq!(phone.services(&home_id).await.unwrap()["restricted"], true);
    let error = phone.pair(&uri, "iPhone").await.unwrap_err().to_string();
    assert_eq!(error, "invitation unavailable");
    phone.shutdown().await;
    home.shutdown().await;
}

#[tokio::test]
async fn paired_access_accepts_selected_tcp_destinations_and_reverse_ports_and_logs_them() {
    let (_a, _b, home, phone) = nodes().await;
    let home_id = pair(&home, &phone).await;
    let (target, echo) = echo().await;
    let mut events = home.subscribe();
    assert_eq!(phone.services(&home_id).await.unwrap()["restricted"], false);
    for destination in [target.to_string(), format!("localhost:{}", target.port())] {
        let id = phone
            .forward(
                &home_id,
                "127.0.0.1:0".parse().unwrap(),
                destination.clone(),
            )
            .await
            .unwrap();
        roundtrip(local_listener(&phone, &id), 41).await;
        timeout(Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.unwrap();
                if event["event"] == "access_request" && event["kind"] == "tcp" {
                    assert_eq!(event["destination"], destination);
                    assert_eq!(event["peer"], phone.endpoint.id().to_string());
                    break;
                }
            }
        })
        .await
        .unwrap();
        phone.stop(&id);
    }
    let reserve = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = reserve.local_addr().unwrap().port();
    drop(reserve);
    let id = phone.reverse(&home_id, port, target).await.unwrap();
    roundtrip(SocketAddr::from(([127, 0, 0, 1], port)), 42).await;
    timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if event["event"] == "access_request" && event["kind"] == "reverse" {
                assert_eq!(event["port"], port);
                break;
            }
        }
    })
    .await
    .unwrap();
    phone.stop(&id);
    home.set_policy(
        &phone.endpoint.id().to_string(),
        Policy {
            restricted: true,
            ..Default::default()
        },
    )
    .unwrap();
    let id = phone
        .forward(&home_id, "127.0.0.1:0".parse().unwrap(), target.to_string())
        .await
        .unwrap();
    let mut client = TcpStream::connect(local_listener(&phone, &id))
        .await
        .unwrap();
    client.write_all(b"blocked").await.unwrap();
    client.shutdown().await.unwrap();
    let mut response = Vec::new();
    let result = timeout(Duration::from_secs(5), client.read_to_end(&mut response))
        .await
        .unwrap();
    assert!(
        result.is_err() || response.is_empty(),
        "restricted peers cannot select arbitrary destinations"
    );
    phone.shutdown().await;
    home.shutdown().await;
    echo.abort();
}

#[tokio::test]
async fn forward_can_listen_on_all_interfaces_and_stops_listener() {
    let (_a, _b, home, phone) = nodes().await;
    let home_id = pair(&home, &phone).await;
    let (target, echo) = echo().await;
    let result = phone
        .command(forwiroh::control::Command::Forward {
            peer: home_id,
            listen: "0.0.0.0:0".parse().unwrap(),
            service: target.to_string(),
        })
        .await
        .unwrap();
    let id = result["id"].as_str().unwrap();
    let listen = local_listener(&phone, id);
    assert!(listen.ip().is_unspecified());
    roundtrip(SocketAddr::from(([127, 0, 0, 1], listen.port())), 43).await;
    phone.stop(id);
    timeout(Duration::from_secs(5), async {
        loop {
            if TcpListener::bind(listen).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    phone.shutdown().await;
    home.shutdown().await;
    echo.abort();
}

#[tokio::test]
async fn concurrent_tcp_half_close_policy_and_revoke() {
    let (_a, _b, home, phone) = nodes().await;
    let home_id = pair(&home, &phone).await;
    let (addr, echo) = echo().await;
    let mut policy = Policy {
        restricted: true,
        ..Default::default()
    };
    policy.services.insert("echo".into(), addr.to_string());
    home.set_policy(&phone.endpoint.id().to_string(), policy)
        .unwrap();
    let services = phone.services(&home_id).await.unwrap();
    assert_eq!(services["services"], serde_json::json!(["echo"]));
    let tunnel = phone
        .forward(&home_id, "127.0.0.1:0".parse().unwrap(), "echo".into())
        .await
        .unwrap();
    let listen = local_listener(&phone, &tunnel);
    let mut jobs = tokio::task::JoinSet::new();
    for byte in 0..12 {
        jobs.spawn(roundtrip(listen, byte));
    }
    while let Some(r) = jobs.join_next().await {
        r.unwrap();
    }
    let mut live = TcpStream::connect(listen).await.unwrap();
    live.write_all(b"pending").await.unwrap();
    home.revoke(&phone.endpoint.id().to_string()).unwrap();
    let mut buf = Vec::new();
    let result = timeout(Duration::from_secs(5), live.read_to_end(&mut buf)).await;
    assert!(result.is_ok(), "revocation must terminate active streams");
    assert!(phone.services(&home_id).await.is_err());
    phone.stop(&tunnel);
    phone.shutdown().await;
    home.shutdown().await;
    echo.abort();
}
#[tokio::test]
async fn reverse_requires_explicit_port_and_stops_listener() {
    let (_a, _b, home, phone) = nodes().await;
    let home_id = pair(&home, &phone).await;
    let (target, echo) = echo().await;
    home.set_policy(
        &phone.endpoint.id().to_string(),
        Policy {
            restricted: true,
            ..Default::default()
        },
    )
    .unwrap();
    let reserve = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = reserve.local_addr().unwrap().port();
    drop(reserve);
    assert!(phone.reverse(&home_id, port, target).await.is_err());
    let policy = Policy {
        restricted: true,
        reverse_ports: vec![port],
        ..Default::default()
    };
    home.set_policy(&phone.endpoint.id().to_string(), policy)
        .unwrap();
    let id = phone.reverse(&home_id, port, target).await.unwrap();
    roundtrip(SocketAddr::from(([127, 0, 0, 1], port)), 55).await;
    phone.stop(&id);
    timeout(Duration::from_secs(5), async {
        loop {
            if TcpListener::bind(("127.0.0.1", port)).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    phone.shutdown().await;
    home.shutdown().await;
    echo.abort();
}
#[tokio::test]
async fn multiple_peers_and_shutdown_do_not_cross_authorize() {
    let (_a, _b, home, phone) = nodes().await;
    pair(&home, &phone).await;
    let dir = tempfile::tempdir().unwrap();
    let laptop = Node::open(
        dir.path(),
        "laptop",
        None,
        forwiroh::TransportMode::LocalOnly,
    )
    .await
    .unwrap();
    pair(&home, &laptop).await;
    assert_eq!(home.snapshot()["peers"].as_array().unwrap().len(), 2);
    assert_eq!(
        laptop
            .services(&home.endpoint.id().to_string())
            .await
            .unwrap()["services"],
        Value::Array(vec![])
    );
    timeout(Duration::from_secs(5), phone.shutdown())
        .await
        .unwrap();
    timeout(Duration::from_secs(5), laptop.shutdown())
        .await
        .unwrap();
    timeout(Duration::from_secs(5), home.shutdown())
        .await
        .unwrap();
}

#[tokio::test]
async fn cancelled_pairing_and_invitation_revocation_release_pending_requests() {
    let (_a, _b, home, phone) = nodes().await;
    let invite = home.invitation().unwrap();
    let uri = invite.uri().unwrap();
    let mut events = home.subscribe();
    let p = phone.clone();
    let task = tokio::spawn(async move { p.pair(&uri, "test").await });
    timeout(Duration::from_secs(5), async {
        loop {
            if events.recv().await.unwrap()["event"] == "pair_request" {
                break;
            }
        }
    })
    .await
    .unwrap();
    home.cancel_invitation(&invite.id);
    assert!(
        timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(home.pending().as_array().unwrap().is_empty());
    assert!(home.snapshot()["peers"].as_array().unwrap().is_empty());
    phone.shutdown().await;
    home.shutdown().await;
}
#[tokio::test]
async fn modified_invitation_secret_does_not_consume_valid_invitation() {
    let (_a, _b, home, phone) = nodes().await;
    let invite = home.invitation().unwrap();
    let mut forged = invite.clone();
    forged.secret = forwiroh::pairing::random().unwrap();
    assert!(phone.pair(&forged.uri().unwrap(), "test").await.is_err());
    let mut events = home.subscribe();
    let p = phone.clone();
    let uri = invite.uri().unwrap();
    let task = tokio::spawn(async move { p.pair(&uri, "test").await });
    timeout(Duration::from_secs(5), async {
        loop {
            if events.recv().await.unwrap()["event"] == "pair_request" {
                break;
            }
        }
    })
    .await
    .unwrap();
    home.approve(&invite.id, true).unwrap();
    assert!(task.await.unwrap().is_ok());
    phone.shutdown().await;
    home.shutdown().await;
}

#[tokio::test]
async fn initiator_cancellation_closes_connection_and_clears_approval() {
    let (_a, _b, home, phone) = nodes().await;
    let uri = home.invitation().unwrap().uri().unwrap();
    let mut events = home.subscribe();
    let p = phone.clone();
    let task = tokio::spawn(async move { p.pair(&uri, "test").await });
    timeout(Duration::from_secs(5), async {
        loop {
            if events.recv().await.unwrap()["event"] == "pair_request" {
                break;
            }
        }
    })
    .await
    .unwrap();
    phone.cancel_pair();
    assert!(
        timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    timeout(Duration::from_secs(5), async {
        while !home.pending().as_array().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    phone.shutdown().await;
    home.shutdown().await;
}

#[tokio::test]
async fn simultaneous_peers_keep_permissions_and_revocation_isolated() {
    let (_a, _b, home, phone) = nodes().await;
    let third = tempfile::tempdir().unwrap();
    let laptop = Node::open(
        third.path(),
        "laptop",
        None,
        forwiroh::TransportMode::LocalOnly,
    )
    .await
    .unwrap();
    let home_id = pair(&home, &phone).await;
    pair(&home, &laptop).await;
    let (addr, echo) = echo().await;
    let mut policy = Policy {
        restricted: true,
        ..Default::default()
    };
    policy.services.insert("echo".into(), addr.to_string());
    home.set_policy(&phone.endpoint.id().to_string(), policy.clone())
        .unwrap();
    assert_eq!(
        laptop.services(&home_id).await.unwrap()["services"],
        serde_json::json!([])
    );
    home.set_policy(&laptop.endpoint.id().to_string(), policy)
        .unwrap();
    let (a, b) = tokio::join!(
        phone.forward(&home_id, "127.0.0.1:0".parse().unwrap(), "echo".into()),
        laptop.forward(&home_id, "127.0.0.1:0".parse().unwrap(), "echo".into())
    );
    let a = local_listener(&phone, &a.unwrap());
    let b = local_listener(&laptop, &b.unwrap());
    tokio::join!(roundtrip(a, 17), roundtrip(b, 29));
    home.revoke(&phone.endpoint.id().to_string()).unwrap();
    assert!(phone.services(&home_id).await.is_err());
    roundtrip(b, 31).await;
    assert_eq!(home.snapshot()["peers"].as_array().unwrap().len(), 1);
    laptop.shutdown().await;
    phone.shutdown().await;
    home.shutdown().await;
    echo.abort();
}
