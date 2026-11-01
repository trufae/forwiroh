use anyhow::Result;
use iroh::endpoint::{RecvStream, SendStream};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_util::sync::CancellationToken;

pub async fn bridge(
    tcp: TcpStream,
    send: SendStream,
    recv: RecvStream,
    cancel: CancellationToken,
) -> Result<()> {
    bridge_io(tcp, tokio::io::join(recv, send), cancel).await
}

async fn bridge_io(
    mut tcp: impl AsyncRead + AsyncWrite + Unpin,
    mut quic: impl AsyncRead + AsyncWrite + Unpin,
    cancel: CancellationToken,
) -> Result<()> {
    // Bounded buffers and TCP half-close; QUIC keepalives detect a dead peer.
    tokio::select! {
        r = tokio::io::copy_bidirectional(&mut tcp, &mut quic) => { r?; },
        _ = cancel.cancelled() => {},
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test(start_paused = true)]
    async fn idle_streams_remain_usable_and_preserve_half_close() {
        let (mut client, tcp) = tokio::io::duplex(64);
        let (quic, mut server) = tokio::io::duplex(64);
        let bridge = tokio::spawn(bridge_io(tcp, quic, CancellationToken::new()));
        client.write_all(b"before").await.unwrap();
        server.read_exact(&mut [0; 6]).await.unwrap();
        tokio::time::advance(Duration::from_secs(360)).await;
        client.write_all(b"after").await.unwrap();
        client.shutdown().await.unwrap();
        let mut request = Vec::new();
        server.read_to_end(&mut request).await.unwrap();
        assert_eq!(request, b"after");
        server.write_all(b"reply").await.unwrap();
        server.shutdown().await.unwrap();
        let mut reply = Vec::new();
        client.read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, b"reply");
        bridge.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_a_quiet_stream_releases_both_ends() {
        let (mut client, tcp) = tokio::io::duplex(64);
        let (quic, mut server) = tokio::io::duplex(64);
        let cancel = CancellationToken::new();
        let bridge = tokio::spawn(bridge_io(tcp, quic, cancel.clone()));
        client.write_all(b"ready").await.unwrap();
        server.read_exact(&mut [0; 5]).await.unwrap();
        tokio::time::advance(Duration::from_secs(360)).await;
        cancel.cancel();
        bridge.await.unwrap().unwrap();
        assert_eq!(client.read(&mut [0; 1]).await.unwrap(), 0);
        assert_eq!(server.read(&mut [0; 1]).await.unwrap(), 0);
    }
}
