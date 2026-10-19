#![allow(dead_code)]
use forwiroh::Node;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

pub async fn nodes() -> (tempfile::TempDir, tempfile::TempDir, Arc<Node>, Arc<Node>) {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let home = Node::open(a.path(), "home", None, forwiroh::TransportMode::LocalOnly)
        .await
        .unwrap();
    let phone = Node::open(b.path(), "phone", None, forwiroh::TransportMode::LocalOnly)
        .await
        .unwrap();
    (a, b, home, phone)
}
pub async fn pair(home: &Arc<Node>, phone: &Arc<Node>) -> String {
    let invitation = home.invitation().unwrap();
    let uri = invitation.uri().unwrap();
    eprintln!("home addr: {:?}", home.endpoint.addr());
    let mut events = home.subscribe();
    let p = phone.clone();
    let task = tokio::spawn(async move {
        let r = p.pair(&uri, "test").await;
        eprintln!("pair outcome {r:?}");
        r
    });
    timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.unwrap();
            eprintln!("event {event}");
            if event["event"] == "pair_request" {
                home.approve(event["id"].as_str().unwrap(), true).unwrap();
                break;
            }
        }
    })
    .await
    .unwrap();
    task.await.unwrap().unwrap()
}
pub async fn echo() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    echo_on("127.0.0.1:0").await
}
pub async fn echo_on(bind: &str) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let server = TcpListener::bind(bind).await.unwrap();
    let addr = server.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = server.accept().await {
            tokio::spawn(async move {
                let mut data = Vec::new();
                socket.read_to_end(&mut data).await.unwrap();
                socket.write_all(&data).await.unwrap();
                socket.shutdown().await.unwrap();
            });
        }
    });
    (addr, task)
}
pub fn local_listener(node: &Node, id: &str) -> SocketAddr {
    let snapshot = node.snapshot();
    let tunnels = snapshot["tunnels"].as_array().unwrap();
    tunnels.iter().find(|t| t["id"] == id).unwrap()["detail"]
        .as_str()
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}
pub async fn roundtrip(addr: SocketAddr, byte: u8) {
    let mut socket = TcpStream::connect(addr).await.unwrap();
    let data = vec![byte; 100_000];
    socket.write_all(&data).await.unwrap();
    socket.shutdown().await.unwrap(); // Response is only sent after FIN.
    let mut response = Vec::new();
    timeout(Duration::from_secs(10), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(data, response);
}
