use anyhow::Result;
use iroh::endpoint::{RecvStream, SendStream};
use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};
use tokio_util::sync::CancellationToken;
// One bounded copy buffer in each direction. Tokio's bridge preserves TCP half-close.
struct Duplex {
    send: SendStream,
    recv: RecvStream,
    last_io: std::sync::Arc<std::sync::Mutex<tokio::time::Instant>>,
}
impl AsyncRead for Duplex {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = b.filled().len();
        let result = Pin::new(&mut self.recv).poll_read(cx, b);
        if b.filled().len() != before {
            *self.last_io.lock().unwrap() = tokio::time::Instant::now();
        }
        result
    }
}
impl AsyncWrite for Duplex {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let r = AsyncWrite::poll_write(Pin::new(&mut self.send), cx, b);
        if let Poll::Ready(Ok(n)) = &r
            && *n > 0
        {
            *self.last_io.lock().unwrap() = tokio::time::Instant::now();
        }
        r
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_shutdown(cx)
    }
}
pub async fn bridge(
    mut tcp: TcpStream,
    send: SendStream,
    recv: RecvStream,
    cancel: CancellationToken,
) -> Result<()> {
    let last_io = std::sync::Arc::new(std::sync::Mutex::new(tokio::time::Instant::now()));
    let mut quic = Duplex {
        send,
        recv,
        last_io: last_io.clone(),
    };
    let idle = async {
        loop {
            let deadline = *last_io.lock().unwrap() + Duration::from_secs(300);
            tokio::time::sleep_until(deadline).await;
            if last_io.lock().unwrap().elapsed() >= Duration::from_secs(300) {
                break;
            }
        }
    };
    tokio::select! {
        r = tokio::io::copy_bidirectional(&mut tcp, &mut quic) => { r?; },
        _ = cancel.cancelled() => {},
        _ = idle => anyhow::bail!("TCP stream idle timeout"),
    }
    Ok(())
}
